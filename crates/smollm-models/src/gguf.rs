//! GGUF inspection + content integrity.
//!
//! Two jobs the fork guard depends on:
//!   1. Verify a downloaded pack byte-for-byte against the sha256 recorded in the catalog (the LFS
//!      `oid` from the HF tree API), so a truncated or swapped file can never reach the engine.
//!   2. Read the GGUF header — KV metadata and sampled tensor types — *without* loading the model,
//!      to assert the pack really carries Prism's rotation metadata. A model quantized to
//!      PQ2_0/PTQ1_0 by anyone other than Prism has no `prism.hadamard.*` keys, loads without
//!      warning and generates nonsense (llama.cpp#242) — the cheapest possible false positive, so we
//!      check metadata before spending gigabytes of I/O.
//!
//! Parsing follows the GGUF v3 spec, bounded to a header window so a corrupt count field cannot
//! make us read the whole weight file.

use std::io::{Read, Take};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ModelsError;

/// How much of the file to read for header parsing. A Qwen-class GGUF's KV section, including long
/// chat templates, stays well under this.
pub const HEADER_WINDOW_BYTES: usize = 4 << 20;
/// Tensor infos to sample for their quant type ids.
pub const TENSOR_SAMPLE: usize = 64;
/// Upper bound on a single array field we are willing to walk.
const MAX_ARRAY_LEN: u64 = 1_000_000;
/// Upper bound on the KV pair count of a well-formed LLM GGUF.
const MAX_KV_PAIRS: u64 = 100_000;

fn err<T>(path: &Path, reason: impl Into<String>) -> Result<T, ModelsError> {
    Err(ModelsError::NotGguf {
        path: path.to_path_buf(),
        reason: reason.into(),
    })
}

/// Streaming sha256 of a file, hex-encoded lowercase.
pub fn sha256_file(path: &Path) -> Result<String, ModelsError> {
    let file = std::fs::File::open(path).map_err(|source| ModelsError::ReadModel {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|source| ModelsError::ReadModel {
                path: path.to_path_buf(),
                source,
            })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityStatus {
    Ok,
    Missing,
    SizeMismatch { expected: u64, actual: u64 },
    HashMismatch { expected: String, actual: String },
}

impl IntegrityStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// One-line, user-facing description. Both hashes go to the log, not at the user, who cannot
    /// act on a hex difference.
    pub fn describe(&self) -> String {
        match self {
            Self::Ok => "verified".into(),
            Self::Missing => "file is not present".into(),
            Self::SizeMismatch { expected, actual } => {
                format!("size mismatch: on disk {actual} bytes, catalog expects {expected}")
            }
            Self::HashMismatch { .. } => "checksum mismatch: re-download required".into(),
        }
    }
}

/// Check size first (cheap), then hash (a multi-GB read).
pub fn verify(
    path: &Path,
    expected_size_bytes: u64,
    expected_sha256: &str,
) -> Result<IntegrityStatus, ModelsError> {
    if !path.exists() {
        return Ok(IntegrityStatus::Missing);
    }
    let actual_size = std::fs::metadata(path)
        .map_err(|source| ModelsError::ReadModel {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if actual_size != expected_size_bytes {
        return Ok(IntegrityStatus::SizeMismatch {
            expected: expected_size_bytes,
            actual: actual_size,
        });
    }
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected_sha256) {
        Ok(IntegrityStatus::Ok)
    } else {
        Ok(IntegrityStatus::HashMismatch {
            expected: expected_sha256.to_string(),
            actual,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Scalar(Vec<u8>),
    Str(String),
    Array(Vec<Value>),
}

/// What we learned from a GGUF file's header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GgufHeader {
    pub path: PathBuf,
    pub version: u32,
    pub n_tensors: u64,
    pub n_kv: u64,
    /// `general.architecture`, e.g. `qwen35`.
    pub architecture: Option<String>,
    /// `general.name`. The shipped files currently report "Hf" (cosmetic bug awaiting a metadata
    /// update), so this is never used for display.
    pub display_name: Option<String>,
    /// `<architecture>.context_length`.
    pub context_length: Option<u32>,
    /// Distinct tensor type ids seen in the sampled tensor infos, sorted.
    pub quant_type_ids: Vec<u32>,
    /// Keys under the rotation prefix, e.g. `prism.hadamard.block_size`.
    pub rotation_keys: Vec<String>,
}

impl GgufHeader {
    pub fn has_rotation_metadata(&self) -> bool {
        !self.rotation_keys.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    Bool,
    String,
    Array,
    U64,
    I64,
    F64,
}

impl Kind {
    fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            0 => Self::U8,
            1 => Self::I8,
            2 => Self::U16,
            3 => Self::I16,
            4 => Self::U32,
            5 => Self::I32,
            6 => Self::F32,
            7 => Self::Bool,
            8 => Self::String,
            9 => Self::Array,
            10 => Self::U64,
            11 => Self::I64,
            12 => Self::F64,
            _ => return None,
        })
    }

    fn scalar_len(self) -> Option<usize> {
        Some(match self {
            Self::U8 | Self::I8 | Self::Bool => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::U64 | Self::I64 | Self::F64 => 8,
            Self::String | Self::Array => return None,
        })
    }
}

/// Cursor over a bounded header window.
struct Cursor_ {
    buf: Vec<u8>,
    pos: usize,
}

impl Cursor_ {
    fn take(&mut self, n: usize) -> Option<Vec<u8>> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let out = self.buf[self.pos..end].to_vec();
        self.pos = end;
        Some(out)
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        Some(u64::from_le_bytes(b.as_slice().try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let len = self.u64()?;
        if len > 1 << 22 {
            return None;
        }
        let b = self.take(len as usize)?;
        Some(String::from_utf8_lossy(&b).into_owned())
    }
}

pub fn read_header(path: &Path) -> Result<GgufHeader, ModelsError> {
    let buf = read_prefix(path, HEADER_WINDOW_BYTES)?;
    parse_header(path, &buf)
}

/// Read at most `limit` bytes from the start of a file, so header parsing never streams a 7 GB
/// weight pack into memory.
fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, ModelsError> {
    let file = std::fs::File::open(path).map_err(|source| ModelsError::ReadModel {
        path: path.to_path_buf(),
        source,
    })?;
    let mut take: Take<std::fs::File> = file.take(limit as u64);
    let mut buf = Vec::with_capacity(1 << 16);
    take.read_to_end(&mut buf)
        .map_err(|source| ModelsError::ReadModel {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(buf)
}

fn parse_header(path: &Path, buf: &[u8]) -> Result<GgufHeader, ModelsError> {
    if buf.len() < 24 || &buf[..4] != b"GGUF" {
        return err(path, "missing GGUF magic");
    }
    let mut c = Cursor_ {
        buf: buf.to_vec(),
        pos: 4,
    };
    let version = c.u32().ok_or_else(|| unparseable(path))?;
    if version != 2 && version != 3 {
        return Err(ModelsError::NotGguf {
            path: path.to_path_buf(),
            reason: format!("unsupported GGUF version {version}"),
        });
    }
    let n_tensors = c.u64().ok_or_else(|| unparseable(path))?;
    let n_kv = c.u64().ok_or_else(|| unparseable(path))?;
    if n_kv > MAX_KV_PAIRS {
        return err(path, format!("implausible KV count {n_kv}"));
    }

    let mut header = GgufHeader {
        path: path.to_path_buf(),
        version,
        n_tensors,
        n_kv,
        ..Default::default()
    };
    for _ in 0..n_kv {
        let key = c.string().ok_or_else(|| unparseable(path))?;
        let raw_kind = c.u32().ok_or_else(|| unparseable(path))?;
        let kind = Kind::from_u32(raw_kind).ok_or_else(|| ModelsError::NotGguf {
            path: path.to_path_buf(),
            reason: format!("unknown value type {raw_kind} for key {key}"),
        })?;
        let value = read_value(path, &mut c, kind)?;
        record_kv(&mut header, &key, &value);
    }

    // Tensor infos follow the KV section: name, n_dims, dims, type, offset.
    let sample = n_tensors.min(TENSOR_SAMPLE as u64);
    let mut types = Vec::new();
    for _ in 0..sample {
        let _name = match c.string() {
            Some(n) => n,
            // The header window can end mid-list; what we have is still a valid sample.
            None => break,
        };
        let Some(n_dims) = c.u32() else { break };
        if n_dims > 8 {
            return err(path, format!("implausible tensor rank {n_dims}"));
        }
        let mut dims_ok = true;
        for _ in 0..n_dims {
            if c.u64().is_none() {
                dims_ok = false;
                break;
            }
        }
        if !dims_ok {
            break;
        }
        match c.u32() {
            Some(t) => types.push(t),
            None => break,
        }
        // Offset.
        let _ = c.u64();
    }
    types.sort_unstable();
    types.dedup();
    header.quant_type_ids = types;
    header.rotation_keys.sort();
    Ok(header)
}

fn unparseable(path: &Path) -> ModelsError {
    ModelsError::NotGguf {
        path: path.to_path_buf(),
        reason: "truncated or unrecognised header".into(),
    }
}

fn read_value(path: &Path, c: &mut Cursor_, kind: Kind) -> Result<Value, ModelsError> {
    match kind {
        Kind::String => Ok(Value::Str(c.string().ok_or_else(|| unparseable(path))?)),
        Kind::Array => {
            let elem_raw = c.u32().ok_or_else(|| unparseable(path))?;
            let elem = Kind::from_u32(elem_raw).ok_or_else(|| ModelsError::NotGguf {
                path: path.to_path_buf(),
                reason: format!("unknown array element type {elem_raw}"),
            })?;
            let count = c.u64().ok_or_else(|| unparseable(path))?;
            if count > MAX_ARRAY_LEN {
                return err(path, format!("implausible array length {count}"));
            }
            let mut items = Vec::with_capacity(count.min(64) as usize);
            for _ in 0..count {
                items.push(read_value(path, c, elem)?);
            }
            Ok(Value::Array(items))
        }
        scalar => {
            let n = scalar.scalar_len().unwrap_or(0);
            Ok(Value::Scalar(c.take(n).ok_or_else(|| unparseable(path))?))
        }
    }
}

fn as_u32(value: &Value) -> Option<u32> {
    match value {
        Value::Scalar(b) if b.len() == 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => None,
    }
}

fn record_kv(header: &mut GgufHeader, key: &str, value: &Value) {
    match key {
        "general.architecture" => {
            if let Value::Str(s) = value {
                header.architecture = Some(s.clone());
            }
        }
        "general.name" => {
            if let Value::Str(s) = value {
                header.display_name = Some(s.clone());
            }
        }
        _ => {}
    }
    if key.ends_with(".context_length") {
        header.context_length = as_u32(value);
    }
    if key.starts_with("prism.hadamard.") || key.starts_with("prism.") {
        header.rotation_keys.push(key.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((v.len() as u64).to_le_bytes());
        out.extend(v.as_bytes());
        out
    }

    fn kv_str(key: &str, value: &str) -> Vec<u8> {
        let mut out = s(key);
        out.extend((Kind::String as u32).to_le_bytes());
        out.extend(s(value));
        out
    }

    fn kv_u32(key: &str, value: u32) -> Vec<u8> {
        let mut out = s(key);
        out.extend((Kind::U32 as u32).to_le_bytes());
        out.extend(value.to_le_bytes());
        out
    }

    fn kv_str_array(key: &str, values: &[&str]) -> Vec<u8> {
        let mut out = s(key);
        out.extend((Kind::Array as u32).to_le_bytes());
        out.extend((Kind::String as u32).to_le_bytes());
        out.extend((values.len() as u64).to_le_bytes());
        for v in values {
            out.extend(s(v));
        }
        out
    }

    /// One tensor info: name, 2 dims, type 31, offset 0.
    fn tensor(name: &str, quant_type: u32) -> Vec<u8> {
        let mut out = s(name);
        out.extend(2u32.to_le_bytes());
        out.extend(4096u64.to_le_bytes());
        out.extend(4096u64.to_le_bytes());
        out.extend(quant_type.to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out
    }

    fn synth_file(dir: &Path, kv: &[Vec<u8>], n_tensors: u64, tensors: &[Vec<u8>]) -> PathBuf {
        let mut bytes = Vec::new();
        bytes.extend(b"GGUF");
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(n_tensors.to_le_bytes());
        bytes.extend((kv.len() as u64).to_le_bytes());
        for entry in kv {
            bytes.extend(entry);
        }
        for t in tensors {
            bytes.extend(t);
        }
        let path = dir.join("model.gguf");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn parses_architecture_context_and_rotation_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = synth_file(
            dir.path(),
            &[
                kv_str("general.architecture", "qwen35"),
                kv_str("general.name", "Hf"),
                kv_u32("qwen35.context_length", 262_144),
                kv_str_array("tokenizer.ggml.tokens", &["a", "b"]),
                kv_u32("prism.hadamard.block_size", 1024),
                kv_str("prism.hadamard.signs", "fixed"),
            ],
            2,
            &[
                tensor("blk.0.attn_q.weight", 31),
                tensor("token_embd.weight", 32),
            ],
        );
        let h = read_header(&path).unwrap();
        assert_eq!(h.architecture.as_deref(), Some("qwen35"));
        assert_eq!(h.display_name.as_deref(), Some("Hf"));
        assert_eq!(h.context_length, Some(262_144));
        assert_eq!(h.quant_type_ids, vec![31, 32]);
        assert!(h.has_rotation_metadata());
        assert_eq!(
            h.rotation_keys,
            vec!["prism.hadamard.block_size", "prism.hadamard.signs"]
        );
    }

    #[test]
    fn a_file_without_rotation_metadata_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let path = synth_file(
            dir.path(),
            &[kv_str("general.architecture", "qwen35")],
            0,
            &[],
        );
        assert!(!read_header(&path).unwrap().has_rotation_metadata());
    }

    #[test]
    fn rejects_non_gguf_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.gguf");
        std::fs::write(&path, b"NOTAGGUFFILEATALL").unwrap();
        assert!(matches!(
            read_header(&path),
            Err(ModelsError::NotGguf { .. })
        ));
    }

    #[test]
    fn rejects_an_absurd_array_length_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let mut key = s("general.architecture");
        key.extend((Kind::Array as u32).to_le_bytes());
        key.extend((Kind::U64 as u32).to_le_bytes());
        key.extend((9_999_999_999u64).to_le_bytes());
        let path = synth_file(dir.path(), &[key], 0, &[]);
        assert!(matches!(
            read_header(&path),
            Err(ModelsError::NotGguf { .. })
        ));
    }

    #[test]
    fn verify_reports_size_before_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pack.gguf");
        std::fs::write(&path, b"abc").unwrap();
        let status = verify(&path, 999, "whatever").unwrap();
        assert_eq!(
            status,
            IntegrityStatus::SizeMismatch {
                expected: 999,
                actual: 3
            }
        );
        // 3 bytes of "abc": sha256 known value.
        let digest = sha256_file(&path).unwrap();
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(verify(&path, 3, &digest).unwrap().is_ok());
        assert!(!verify(&path, 3, "00".repeat(32).as_str()).unwrap().is_ok());
        assert_eq!(
            verify(&dir.path().join("nope.gguf"), 1, "x").unwrap(),
            IntegrityStatus::Missing
        );
    }
}
