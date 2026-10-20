//! The load-guard: the check that runs before any weight file is handed to any binary.
//!
//! It exists because of a failure mode documented in
//! <https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf/raw/main/KNOWN_ISSUES.md> and
//! llama.cpp#242:
//! `PQ2_0`/`PTQ1_0` are *new quant types that exist only in the PrismML fork*, and the F16
//! reference is in a rotated basis whose activation transform only the fork applies. Point stock
//! llama.cpp at these files and it does not fail loudly — the ternary types are rejected as
//! unknown, while F16 *loads successfully and generates nonsense*. Equally, an ordinary model
//! quantized to `PQ2_0` (no Prism export, so no rotation metadata) loads with no warning and
//! emits garbage.
//!
//! So the guard demands positive proof of both halves of the pairing:
//!   * the engine is our pinned fork build under `engines/<tag>/`, never a `PATH` lookup;
//!   * the GGUF carries every prefix in the entry's `required_metadata_prefixes`.
//!
//! Warnings never block a launch. They are the catalog's `[[model.packs.caveats]]` rows that
//! match this host — the KNOWN_ISSUES items that degrade a specific combination — and they belong
//! in the UI next to the model chip.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use smollm_models::catalog::{ModelEntry, Pack, Severity};
use smollm_models::gguf::GgufHeader;
use smollm_models::hardware::{host_os, Backend, HostHardware};
use smollm_models::lock::EngineLock;

use crate::engine::EngineBinary;

/// Everything the guard needs, already read off disk by the Model Manager.
#[derive(Debug, Clone)]
pub struct GuardRequest<'a> {
    /// Repo root (or the app data root) that holds `engines/`.
    pub root: &'a Path,
    pub lock: &'a EngineLock,
    pub model: &'a ModelEntry,
    pub pack: &'a Pack,
    /// Absolute path of the GGUF we would load.
    pub file: &'a Path,
    pub state: FileState,
    /// Parsed header, or `None` when the file could not be read at all.
    pub header: Option<&'a GgufHeader>,
    /// The resolved fork binary, if one is installed.
    pub engine: Option<&'a EngineBinary>,
    /// Host facts used to match caveats; `None` skips caveat matching.
    pub host: Option<&'a HostHardware>,
}

/// What the catalog's integrity check says about the file on disk.
#[derive(Debug, Clone, PartialEq)]
pub enum FileState {
    /// Present and byte-for-byte equal to the published object.
    Verified,
    /// Present, but the size or sha256 disagrees with the catalog.
    Mismatch { status: String },
    /// Not on disk yet.
    Absent,
    /// The catalog entry carries no usable hash, so nothing could be compared.
    Unverifiable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Blocker {
    /// No pinned fork binary on disk.
    EngineNotInstalled {
        message: String,
        download_url: String,
    },
    /// A binary outside `engines/` was offered: stock llama.cpp, Homebrew, a stale vendored copy.
    StockEngineRejected { path: PathBuf },
    /// The file on disk is not the file Hugging Face published, or is absent.
    IntegrityFailed { file: String, status: String },
    /// A rotated-basis file without its `prism.hadamard.*` keys: it would load and emit garbage.
    RotationMetadataMissing {
        path: PathBuf,
        required_prefixes: Vec<String>,
        found_keys: Vec<String>,
    },
    /// The lock's pin is below the floor that the fork's first binary release defines.
    EnginePinTooOld { pinned: String, floor: String },
}

/// Which of the refusals this is, named once so the locale catalogs can hold its two sentences.
///
/// The code, not the sentence, crosses the IPC boundary: a host that carried English too would leave
/// two copies to drift apart, one of which the user never sees in their language. `technical` stays
/// the host's own text because it is what the machine saw — a path, a pin, a metadata key — not copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockerCode {
    /// No pinned fork binary on disk.
    EngineNotInstalled,
    /// A stock binary was offered where only the fork is allowed.
    StockEngineRejected,
    /// The chosen pack has never been fetched, which is a task and not a corrupted file.
    WeightsNotDownloaded,
    /// The file on disk disagrees with what the publisher shipped.
    IntegrityMismatch,
    /// A rotated-basis file without its `prism.hadamard.*` keys.
    RotationMetadataMissing,
    /// The lock's pin predates the fork's first binary release.
    EnginePinTooOld,
}

impl BlockerCode {
    pub const ALL: [Self; 6] = [
        Self::EngineNotInstalled,
        Self::StockEngineRejected,
        Self::WeightsNotDownloaded,
        Self::IntegrityMismatch,
        Self::RotationMetadataMissing,
        Self::EnginePinTooOld,
    ];

    /// The card's headline: which problem this is, in a few words.
    pub fn title_key(self) -> &'static str {
        match self {
            Self::EngineNotInstalled => "blocker.engine-not-installed.title",
            Self::StockEngineRejected => "blocker.stock-engine-rejected.title",
            Self::WeightsNotDownloaded => "blocker.weights-not-downloaded.title",
            Self::IntegrityMismatch => "blocker.integrity-mismatch.title",
            Self::RotationMetadataMissing => "blocker.rotation-metadata-missing.title",
            Self::EnginePinTooOld => "blocker.engine-pin-too-old.title",
        }
    }

    /// The card's instruction: what to press, in the order the app offers it.
    pub fn detail_key(self) -> &'static str {
        match self {
            Self::EngineNotInstalled => "blocker.engine-not-installed.detail",
            Self::StockEngineRejected => "blocker.stock-engine-rejected.detail",
            Self::WeightsNotDownloaded => "blocker.weights-not-downloaded.detail",
            Self::IntegrityMismatch => "blocker.integrity-mismatch.detail",
            Self::RotationMetadataMissing => "blocker.rotation-metadata-missing.detail",
            Self::EnginePinTooOld => "blocker.engine-pin-too-old.detail",
        }
    }

    /// The fields the two sentences may interpolate, checked against both locales' `{{braces}}` by
    /// `verify-transcript-i18n`. Only the refusals about one particular file can name it.
    pub fn params(self) -> &'static [&'static str] {
        match self {
            Self::WeightsNotDownloaded | Self::IntegrityMismatch => &["file"],
            _ => &[],
        }
    }

    /// The stable id a log line uses for this refusal. Diagnostics name the code rather than
    /// repeating a sentence the catalogs own.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EngineNotInstalled => "engine-not-installed",
            Self::StockEngineRejected => "stock-engine-rejected",
            Self::WeightsNotDownloaded => "weights-not-downloaded",
            Self::IntegrityMismatch => "integrity-mismatch",
            Self::RotationMetadataMissing => "rotation-metadata-missing",
            Self::EnginePinTooOld => "engine-pin-too-old",
        }
    }
}

/// The one action that clears a blocker, as the catalog key of its button label plus the URL it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CtaCode {
    InstallEngine,
    DownloadOfficialFiles,
    RedownloadFile,
}

impl CtaCode {
    pub const ALL: [Self; 3] = [
        Self::InstallEngine,
        Self::DownloadOfficialFiles,
        Self::RedownloadFile,
    ];

    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::InstallEngine => "cta.install-engine",
            Self::DownloadOfficialFiles => "cta.download-official-files",
            Self::RedownloadFile => "cta.redownload-file",
        }
    }
}

/// The single action that clears a blocker, as the UI's call-to-action.
#[derive(Debug, Clone, PartialEq)]
pub struct CallToAction {
    pub code: CtaCode,
    pub url: String,
}

impl Blocker {
    /// Which refusal this is, as the code both locale catalogs write sentences against.
    pub fn code(&self) -> BlockerCode {
        match self {
            Blocker::EngineNotInstalled { .. } => BlockerCode::EngineNotInstalled,
            Blocker::StockEngineRejected { .. } => BlockerCode::StockEngineRejected,
            // A file that has never arrived is not a forged file. Telling the user their download
            // disagrees with the publisher would send them looking for corruption in the wrong place.
            Blocker::IntegrityFailed { status, .. } if status == "not downloaded" => {
                BlockerCode::WeightsNotDownloaded
            }
            Blocker::IntegrityFailed { .. } => BlockerCode::IntegrityMismatch,
            Blocker::RotationMetadataMissing { .. } => BlockerCode::RotationMetadataMissing,
            Blocker::EnginePinTooOld { .. } => BlockerCode::EnginePinTooOld,
        }
    }

    /// The values behind the fields [`BlockerCode::params`] names, read off this blocker.
    pub fn params(&self) -> Vec<(&'static str, String)> {
        match self {
            Blocker::IntegrityFailed { file, .. } => vec![("file", file.clone())],
            _ => Vec::new(),
        }
    }

    /// The evidence behind the refusal: paths, hashes, metadata prefixes, pins, upstream issues. It
    /// goes under a disclosure rather than in the sentence above it, because a user who is trying to
    /// get an answer needs the instruction first — and a user who is filing a bug needs exactly this.
    pub fn technical(&self) -> String {
        match self {
            Blocker::EngineNotInstalled { message, .. } => message.clone(),
            Blocker::StockEngineRejected { path } => format!(
                "{} is not the pinned PrismML build. The ternary packings (PTQ1_0, PQ2_0) are \
                 unknown to stock llama.cpp, and the F16 reference would load but produce garbled \
                 output because the Hadamard activation transform is fork-only. There is no \
                 automatic fallback to a system llama.cpp.",
                path.display()
            ),
            Blocker::IntegrityFailed { file, status } => {
                format!("{file} does not match the published file: {status}")
            }
            Blocker::RotationMetadataMissing {
                path,
                required_prefixes,
                found_keys,
            } => format!(
                "{} carries none of the required {} metadata (keys found: {}). A ternary \
                 checkpoint must be exported with Prism's blockwise Hadamard rotation; anything \
                 else loads silently and generates nonsense (llama.cpp#242).",
                path.display(),
                required_prefixes.join(", "),
                if found_keys.is_empty() {
                    "none".into()
                } else {
                    found_keys.join(", ")
                }
            ),
            Blocker::EnginePinTooOld { pinned, floor } => format!(
                "engines/lock.toml pins {pinned}, but the fork's first release with binaries is \
                 {floor} (prism-b10687 published no assets at all). Raise the pin before launching."
            ),
        }
    }

    /// The one link that clears this blocker. `model_repo` is the Hugging Face repo the weights come
    /// from, which is NOT `lock.repo`: that is the llama.cpp fork on GitHub, and sending someone
    /// there to re-fetch a `.gguf` is how a wrong URL ends up printed under the right diagnosis.
    pub fn cta(&self, lock: &EngineLock, model_repo: &str) -> CallToAction {
        let releases = format!(
            "https://github.com/{}/releases/tag/{}",
            lock.repo, lock.pinned_tag
        );
        match self {
            Blocker::EngineNotInstalled {
                download_url,
                message: _,
            } => CallToAction {
                code: CtaCode::InstallEngine,
                url: if download_url.is_empty() {
                    releases
                } else {
                    download_url.clone()
                },
            },
            Blocker::StockEngineRejected { .. } | Blocker::EnginePinTooOld { .. } => CallToAction {
                code: CtaCode::InstallEngine,
                url: releases,
            },
            Blocker::RotationMetadataMissing { .. } => CallToAction {
                code: CtaCode::DownloadOfficialFiles,
                url: format!("https://huggingface.co/{model_repo}"),
            },
            Blocker::IntegrityFailed { file, .. } => CallToAction {
                code: CtaCode::RedownloadFile,
                url: format!("https://huggingface.co/{model_repo}/resolve/main/{file}"),
            },
        }
    }
}

/// A catalog caveat that matches this host. Never blocks a launch.
#[derive(Debug, Clone, PartialEq)]
pub struct Warning {
    pub code: WarningCode,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningCode {
    /// A `[[model.packs.caveats]]` row whose condition matched this host.
    Caveat,
    /// A caveat row whose condition uses an atom this build cannot evaluate, so it is shown
    /// unconditionally rather than silently dropped.
    CaveatUnevaluated,
    /// The catalog has no sha256 for this file, so integrity could not be confirmed.
    UnverifiableHash,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuardReport {
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<Warning>,
}

impl GuardReport {
    pub fn is_allowed(&self) -> bool {
        self.blockers.is_empty()
    }

    pub fn first_blocker(&self) -> Option<&Blocker> {
        self.blockers.first()
    }
}

pub fn check(req: &GuardRequest) -> GuardReport {
    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    // 1. The pin itself. Anything older than the fork's first binary release cannot be launched,
    //    and no code path may substitute a system build instead.
    if let Err(e) = EngineBinary::assert_tag_allowed(req.lock) {
        blockers.push(match e {
            crate::LlamaError::EngineTooOld { pinned, floor } => {
                Blocker::EnginePinTooOld { pinned, floor }
            }
            other => Blocker::EnginePinTooOld {
                pinned: format!("{other}"),
                floor: req.lock.min_allowed_tag.clone(),
            },
        });
    }

    // 2. The engine.
    match req.engine {
        None => {
            let resolved = EngineBinary::resolve_for(
                req.root,
                req.lock,
                host_os(),
                std::env::consts::ARCH,
                req.host.map(|h| h.backend).unwrap_or(Backend::Cpu),
            );
            match resolved {
                Ok(bin) => blockers.push(Blocker::EngineNotInstalled {
                    message: bin.missing_message(req.lock),
                    download_url: bin.download_url(req.lock),
                }),
                Err(e) => blockers.push(Blocker::EngineNotInstalled {
                    message: e.to_string(),
                    download_url: String::new(),
                }),
            }
        }
        Some(bin) => {
            if !bin.installed {
                blockers.push(Blocker::EngineNotInstalled {
                    message: bin.missing_message(req.lock),
                    download_url: bin.download_url(req.lock),
                });
            }
            if bin.assert_not_stock_engine(req.root).is_err() {
                blockers.push(Blocker::StockEngineRejected {
                    path: bin.server_path.clone(),
                });
            }
        }
    }

    // 3. The weights.
    match &req.state {
        FileState::Absent => blockers.push(Blocker::IntegrityFailed {
            file: req.pack.file.clone(),
            status: "not downloaded".into(),
        }),
        FileState::Mismatch { status } => blockers.push(Blocker::IntegrityFailed {
            file: req.pack.file.clone(),
            status: status.clone(),
        }),
        FileState::Unverifiable => warnings.push(Warning {
            code: WarningCode::UnverifiableHash,
            severity: Severity::Moderate,
            message: format!(
                "{} has no catalog sha256, so its integrity could not be confirmed.",
                req.pack.file
            ),
        }),
        FileState::Verified => {}
    }

    // 4. Proof of the rotated basis — required for every pack of a ternary model, F16 included.
    //    An absent file is refused as "not downloaded" above; calling a file that is not on disk
    //    "not a Prism export" would send the user checking a metadata prefix instead of pressing
    //    Download.
    if req.pack.needs_fork_engine(&req.model.weight_format)
        && !matches!(req.state, FileState::Absent)
    {
        let required = &req.model.engine.required_metadata_prefixes;
        let found: &[String] = req
            .header
            .map(|h| h.rotation_keys.as_slice())
            .unwrap_or(&[]);
        let missing: Vec<String> = required
            .iter()
            .filter(|pfx| !found.iter().any(|k| k.starts_with(pfx.as_str())))
            .cloned()
            .collect();
        if !missing.is_empty() {
            blockers.push(Blocker::RotationMetadataMissing {
                path: req.file.to_path_buf(),
                required_prefixes: missing,
                found_keys: found.to_vec(),
            });
        }
    }

    // 5. KNOWN_ISSUES rows recorded as caveats, matched against this host.
    if let Some(host) = req.host {
        for c in &req.pack.caveats {
            match caveat_matches(&c.condition, host) {
                Some(true) => warnings.push(Warning {
                    code: WarningCode::Caveat,
                    severity: c.severity,
                    message: format!(
                        "{} — {}. Workaround: {}",
                        c.symptom, c.condition, c.workaround
                    ),
                }),
                // An atom we do not understand must not quietly hide a documented crash.
                None => warnings.push(Warning {
                    code: WarningCode::CaveatUnevaluated,
                    severity: c.severity,
                    message: format!(
                        "{} (condition: {}). Workaround: {}",
                        c.symptom, c.condition, c.workaround
                    ),
                }),
                Some(false) => {}
            }
        }
    }

    GuardReport { blockers, warnings }
}

/// Atoms the catalog's `condition` strings are written in. A condition is a ` and ` joined list;
/// `None` means it contains an atom this build cannot evaluate.
pub fn caveat_matches(condition: &str, host: &HostHardware) -> Option<bool> {
    condition
        .to_ascii_lowercase()
        .split(" and ")
        .map(|atom| atom.trim())
        .try_fold(true, |acc, atom| {
            let this = match atom {
                "true" => Some(true),
                "false" => Some(false),
                a if a.starts_with("backend ==") => {
                    let want = a.trim_start_matches("backend ==").trim();
                    Some(host.backend.key() == want)
                }
                "host_has_avx512" => Some(host.avx512),
                "windows" => Some(std::env::consts::OS == "windows"),
                "macos" => Some(std::env::consts::OS == "macos"),
                "linux" => Some(std::env::consts::OS == "linux"),
                // gpu == <name>, engine_release < ...: predicates this build does not evaluate.
                _ => None,
            }?;
            Some(acc && this)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_models::hardware::GpuArch;
    use smollm_models::Catalog;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn fixture() -> (PathBuf, EngineLock, ModelEntry) {
        let root = repo_root();
        let lock = EngineLock::load_default(&root).unwrap();
        let catalog = Catalog::load_default(&root).unwrap();
        let model = catalog.models.into_iter().next().unwrap();
        (root, lock, model)
    }

    fn host(backend: Backend) -> HostHardware {
        HostHardware {
            backend,
            arch: GpuArch::Unknown,
            gpu_name: None,
            total_ram_bytes: 32 << 30,
            gpu_ram_bytes: None,
            available_disk_bytes: 200 << 30,
            avx512: false,
            degraded_to_cpu: false,
            prebuilt_engine: true,
        }
    }

    fn header_with(keys: Vec<String>) -> GgufHeader {
        GgufHeader {
            path: PathBuf::from("/models/fake.gguf"),
            version: 3,
            n_tensors: 1,
            n_kv: keys.len() as u64,
            architecture: Some("qwen35".into()),
            display_name: None,
            context_length: Some(262144),
            quant_type_ids: vec![49],
            rotation_keys: keys,
        }
    }

    fn request<'a>(
        root: &'a Path,
        lock: &'a EngineLock,
        model: &'a ModelEntry,
        pack: &'a Pack,
        header: Option<&'a GgufHeader>,
        engine: Option<&'a EngineBinary>,
        host: Option<&'a HostHardware>,
    ) -> GuardRequest<'a> {
        GuardRequest {
            root,
            lock,
            model,
            pack,
            file: Path::new("/models/fake.gguf"),
            state: FileState::Verified,
            header,
            engine,
            host,
        }
    }

    #[test]
    fn a_prism_export_on_the_fork_engine_is_allowed() {
        let (root, lock, model) = fixture();
        let pack = model.packs.iter().find(|p| p.key == "PQ2_0").unwrap();
        let header = header_with(vec!["prism.hadamard.block_size".into()]);
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header),
            Some(&engine),
            Some(&host(Backend::Metal)),
        ));
        assert!(
            report.is_allowed(),
            "blockers: {:?}",
            report
                .blockers
                .iter()
                .map(|b| format!("{} — {}", b.code().as_str(), b.technical()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_ternary_file_without_rotation_metadata_is_blocked_even_on_the_fork() {
        let (root, lock, model) = fixture();
        let pack = model.packs.iter().find(|p| p.key == "PQ2_0").unwrap();
        let header = header_with(vec!["general.architecture".into()]);
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header),
            Some(&engine),
            Some(&host(Backend::Metal)),
        ));
        assert!(matches!(
            report.first_blocker(),
            Some(Blocker::RotationMetadataMissing { .. })
        ));
        // The evidence must name the prefix, so a user who opens the disclosure can check their
        // download against the file themselves.
        assert!(report
            .first_blocker()
            .unwrap()
            .technical()
            .contains("prism.hadamard."));
    }

    #[test]
    fn the_f16_reference_is_guarded_too_because_its_basis_is_rotated() {
        let (root, lock, model) = fixture();
        let pack = model.packs.iter().find(|p| p.key == "F16").unwrap();
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            // No rotation keys: the fork would load it and produce garbage.
            Some(&header_with(vec![])),
            Some(&engine),
            Some(&host(Backend::Metal)),
        ));
        assert!(
            matches!(
                report.first_blocker(),
                Some(Blocker::RotationMetadataMissing { .. })
            ),
            "F16 must not sail through: {:?}",
            report.blockers
        );
    }

    #[test]
    fn a_file_that_is_not_downloaded_yet_is_refused_as_absent_not_as_forged() {
        // The same host with no weights on disk must say "not downloaded", because "this is not a
        // Prism export" would send the user at a metadata prefix instead of the Download button.
        let (root, lock, model) = fixture();
        let pack = model.packs.first().unwrap();
        let metal = host(Backend::Metal);
        let mut req = request(&root, &lock, &model, pack, None, None, Some(&metal));
        req.state = FileState::Absent;
        let report = check(&req);
        assert!(
            !report
                .blockers
                .iter()
                .any(|b| matches!(b, Blocker::RotationMetadataMissing { .. })),
            "{:?}",
            report.blockers
        );
        assert!(report.blockers.iter().any(|b| matches!(
            b,
            Blocker::IntegrityFailed { status, .. } if status == "not downloaded"
        )));
    }

    #[test]
    fn a_missing_engine_blocks_with_a_download_url_and_no_stock_fallback() {
        let (root, lock, model) = fixture();
        let pack = model.packs.first().unwrap();
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header_with(vec!["prism.hadamard.block_size".into()])),
            None,
            Some(&host(Backend::Metal)),
        ));
        let blocker = report.first_blocker().expect("engine must be required");
        assert!(matches!(blocker, Blocker::EngineNotInstalled { .. }));
        let cta = blocker.cta(&lock, &model.repo_id);
        assert_eq!(cta.code, CtaCode::InstallEngine);
        assert!(
            cta.url
                .starts_with("https://github.com/PrismML-Eng/llama.cpp/releases/"),
            "{}",
            cta.url
        );
        assert!(!cta.url.contains("ggerganov"));
    }

    /// The two blockers about the weights must send the user to the weights. `lock.repo` is the
    /// llama.cpp fork on GitHub, so a URL built from it prints the right diagnosis over a link
    /// nowhere near the file they need.
    #[test]
    fn a_weights_refusal_points_at_the_model_repo_not_the_engine_releases() {
        let (_, lock, model) = fixture();
        let file = "Ternary-Bonsai-2-27B-PQ2_0.gguf";
        let cta = Blocker::IntegrityFailed {
            file: file.into(),
            status: "not downloaded".into(),
        }
        .cta(&lock, &model.repo_id);
        assert_eq!(
            cta.url,
            format!(
                "https://huggingface.co/{}/resolve/main/{file}",
                model.repo_id
            )
        );
        let cta = Blocker::RotationMetadataMissing {
            path: PathBuf::from(file),
            required_prefixes: vec!["prism.hadamard.".into()],
            found_keys: vec![],
        }
        .cta(&lock, &model.repo_id);
        assert_eq!(cta.url, format!("https://huggingface.co/{}", model.repo_id));
    }

    #[test]
    fn a_path_binary_is_rejected_as_stock_llama() {
        let (root, lock, model) = fixture();
        let pack = model.packs.first().unwrap();
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        engine.server_path = PathBuf::from("/opt/homebrew/bin/llama-server");
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header_with(vec!["prism.hadamard.".into()])),
            Some(&engine),
            Some(&host(Backend::Metal)),
        ));
        assert!(report.blockers.iter().any(|b| {
            matches!(b, Blocker::StockEngineRejected { .. })
                && b.technical().contains("no automatic fallback")
        }));
    }

    #[test]
    fn an_unpinned_engine_tag_blocks_before_launch() {
        let (root, mut lock, model) = fixture();
        lock.pinned_tag = "prism-b10687-9a9394a".into();
        let pack = model.packs.first().unwrap();
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header_with(vec!["prism.hadamard.".into()])),
            None,
            Some(&host(Backend::Metal)),
        ));
        assert!(report.blockers.iter().any(|b| matches!(
            b,
            Blocker::EnginePinTooOld { pinned, .. } if pinned == "prism-b10687-9a9394a"
        )));
    }

    #[test]
    fn host_matching_caveats_become_warnings_not_blockers() {
        let (root, lock, model) = fixture();
        let pack = model.packs.iter().find(|p| p.key == "PQ2_0").unwrap();
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        let header = header_with(vec!["prism.hadamard.".into()]);

        // Vulkan is the documented silent-CPU-fallback case for PQ2_0 (#238).
        let vulkan = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header),
            Some(&engine),
            Some(&host(Backend::Vulkan)),
        ));
        assert!(vulkan.is_allowed(), "{:?}", vulkan.blockers);
        assert!(vulkan
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::Caveat && w.message.contains("backend == vulkan")));

        // Metal is not the condition, so that caveat must not appear.
        let metal = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header),
            Some(&engine),
            Some(&host(Backend::Metal)),
        ));
        assert!(!metal
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::Caveat && w.message.contains("vulkan")));
    }

    #[test]
    fn conditions_this_build_cannot_evaluate_are_surfaced_not_dropped() {
        let (root, lock, model) = fixture();
        let pack = model.packs.iter().find(|p| p.key == "PTQ1_0").unwrap();
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        let report = check(&request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header_with(vec!["prism.hadamard.".into()])),
            Some(&engine),
            Some(&host(Backend::Metal)),
        ));
        // "gpu == intel_arc" is not evaluable from the backend alone, so that row stays visible.
        assert!(report
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::CaveatUnevaluated));
        assert!(report.is_allowed());
    }

    #[test]
    fn the_avx512_cpu_condition_only_fires_on_avx512_hosts() {
        let mut h = host(Backend::Cpu);
        assert_eq!(
            caveat_matches("backend == cpu and host_has_avx512", &h),
            Some(false)
        );
        h.avx512 = true;
        assert_eq!(
            caveat_matches("backend == cpu and host_has_avx512", &h),
            Some(true)
        );
        // Unknown atoms must not be coerced into a confident false.
        assert_eq!(
            caveat_matches("backend == cpu and engine_release < fix-245", &h),
            None
        );
    }

    #[test]
    fn an_absent_file_is_blocked_rather_than_launched_as_zero_bytes() {
        let (root, lock, model) = fixture();
        let pack = model.packs.first().unwrap();
        let mut engine = EngineBinary::from_target(&root, &lock, &lock.targets[0]).unwrap();
        engine.installed = true;
        let header = header_with(vec!["prism.hadamard.".into()]);
        let h = host(Backend::Metal);
        let mut req = request(
            &root,
            &lock,
            &model,
            pack,
            Some(&header),
            Some(&engine),
            Some(&h),
        );
        req.state = FileState::Mismatch {
            status: "size 7206168927 != 7206168928".into(),
        };
        let report = check(&req);
        assert!(matches!(
            report.first_blocker(),
            Some(Blocker::IntegrityFailed { .. })
        ));
    }
}
