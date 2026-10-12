//! Resume-capable Hugging Face downloader with integrity gating.
//!
//! Downloads land in `<dest>.part` and are renamed only after the streaming sha256 matches the
//! catalog value, so a killed transfer can never leave a file that *looks* installed. Range
//! requests resume what is already on disk; a server that ignores the range (200 instead of 206)
//! restarts from zero rather than appending into a mismatch.
//!
//! Some Bonsai repos have been gated while private, and the demo's `setup.sh` documents a read-only
//! `BONSAI_TOKEN`; we take the token from an environment variable named in settings and never
//! persist it in the catalog or the settings file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::gguf::{verify, IntegrityStatus};
use crate::ModelsError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadPlan {
    pub url: String,
    pub dest: PathBuf,
    pub expected_size_bytes: u64,
    pub expected_sha256: String,
    /// Environment variable holding a read-only HF token. The value is read at call time.
    pub token_env: Option<String>,
}

impl DownloadPlan {
    pub fn part_path(&self) -> PathBuf {
        let mut p = self.dest.clone().into_os_string();
        p.push(".part");
        PathBuf::from(p)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub transferred_bytes: u64,
    pub total_bytes: u64,
}

impl Progress {
    pub fn fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        self.transferred_bytes as f32 / self.total_bytes as f32
    }
}

/// Bytes already on disk in a resumable part file, clamped to the expected size. Returns `None`
/// when there is nothing to resume from.
pub fn resume_offset(part_len: u64, expected_len: u64) -> Option<String> {
    if part_len == 0 {
        return None;
    }
    if part_len >= expected_len {
        // A complete-but-unverified part is discarded rather than trusted.
        return None;
    }
    Some(format!("bytes={part_len}-"))
}

/// Preflight: refuse to start a download that cannot fit, instead of failing at 90% with a full
/// disk. `available_bytes == 0` means the probe failed, which the caller must surface as a warning.
pub fn disk_preflight(
    available_bytes: u64,
    plan: &DownloadPlan,
    headroom_bytes: u64,
) -> Result<(), ModelsError> {
    if available_bytes == 0 {
        return Ok(());
    }
    let needed = plan.expected_size_bytes.checked_add(headroom_bytes).ok_or(
        ModelsError::DiskSpaceUnknown {
            path: plan.dest.clone(),
        },
    )?;
    if available_bytes < needed {
        return Err(ModelsError::NotEnoughDisk {
            path: plan.dest.clone(),
            available_bytes,
            needed_bytes: needed,
        });
    }
    Ok(())
}

fn token_from_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Stream one file to disk, resuming a `.part` if present, then verify.
pub async fn download(
    client: &reqwest::Client,
    plan: &DownloadPlan,
    mut on_progress: impl FnMut(Progress),
) -> Result<IntegrityStatus, ModelsError> {
    if plan.dest.exists() {
        let status = verify(&plan.dest, plan.expected_size_bytes, &plan.expected_sha256)?;
        if status.is_ok() {
            return Ok(status);
        }
    }

    let part = plan.part_path();
    let already = tokio::fs::metadata(&part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let range = resume_offset(already, plan.expected_size_bytes);
    if range.is_none() && already > 0 {
        tokio::fs::remove_file(&part)
            .await
            .map_err(|source| ModelsError::WriteModel {
                path: part.clone(),
                source,
            })?;
    }
    let resume_at = if range.is_some() { already } else { 0 };

    let mut request = client
        .get(&plan.url)
        .header(reqwest::header::USER_AGENT, "smollm-studio")
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if let Some(env_name) = plan.token_env.as_deref() {
        if let Some(token) = token_from_env(env_name) {
            request = request.bearer_auth(token);
        }
    }
    if let Some(range) = &range {
        request = request.header(reqwest::header::RANGE, range);
    }

    let response = request.send().await.map_err(|source| ModelsError::Http {
        url: plan.url.clone(),
        source,
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(ModelsError::HttpStatus {
            url: plan.url.clone(),
            status: status.as_u16(),
        });
    }
    // 206 means the range was honoured and we append; 200 means we start over.
    let append = status.as_u16() == 206;
    let mut file = if append {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .await
            .map_err(|source| ModelsError::WriteModel {
                path: part.clone(),
                source,
            })?
    } else {
        tokio::fs::File::create(&part)
            .await
            .map_err(|source| ModelsError::WriteModel {
                path: part.clone(),
                source,
            })?
    };

    use futures::StreamExt as _;
    let mut streamed = response.bytes_stream();
    let mut transferred = resume_at;
    on_progress(Progress {
        transferred_bytes: transferred,
        total_bytes: plan.expected_size_bytes,
    });
    while let Some(chunk) = streamed.next().await {
        let chunk = chunk.map_err(|source| ModelsError::Http {
            url: plan.url.clone(),
            source,
        })?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|source| ModelsError::WriteModel {
                path: part.clone(),
                source,
            })?;
        transferred += chunk.len() as u64;
        on_progress(Progress {
            transferred_bytes: transferred,
            total_bytes: plan.expected_size_bytes,
        });
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|source| ModelsError::WriteModel {
            path: part.clone(),
            source,
        })?;
    drop(file);

    let final_status = verify(&part, plan.expected_size_bytes, &plan.expected_sha256)?;
    if !final_status.is_ok() {
        return Err(ModelsError::Integrity {
            path: part,
            status: Box::new(final_status),
        });
    }
    if let Some(parent) = plan.dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| ModelsError::WriteModel {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    tokio::fs::rename(&part, &plan.dest)
        .await
        .map_err(|source| ModelsError::WriteModel {
            path: plan.dest.clone(),
            source,
        })?;
    Ok(final_status)
}

/// Build the plan for one catalog pack.
impl crate::catalog::ModelEntry {
    pub fn plan_for(
        &self,
        pack: &crate::catalog::Pack,
        model_dir: &Path,
        token_env: Option<String>,
    ) -> DownloadPlan {
        DownloadPlan {
            url: self.url_for(&pack.file),
            dest: model_dir.join(&pack.file),
            expected_size_bytes: pack.size_bytes,
            expected_sha256: pack.sha256.clone(),
            token_env,
        }
    }

    pub fn plan_for_vision(
        &self,
        vision: &crate::catalog::VisionPack,
        model_dir: &Path,
        token_env: Option<String>,
    ) -> DownloadPlan {
        DownloadPlan {
            url: self.url_for(&vision.file),
            dest: model_dir.join(&vision.file),
            expected_size_bytes: vision.size_bytes,
            expected_sha256: vision.sha256.clone(),
            token_env,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> DownloadPlan {
        DownloadPlan {
            url: "https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf/resolve/main/Ternary-Bonsai-2-27B-PQ2_0.gguf"
                .into(),
            dest: PathBuf::from("/tmp/models/pq2.gguf"),
            expected_size_bytes: 1_000,
            expected_sha256: "ab".repeat(32),
            token_env: None,
        }
    }

    #[test]
    fn resume_header_only_for_partial_files() {
        assert_eq!(resume_offset(0, 1_000), None);
        assert_eq!(resume_offset(400, 1_000).as_deref(), Some("bytes=400-"));
        // A part that is already the full size still failed verification, so it must not be trusted.
        assert_eq!(resume_offset(1_000, 1_000), None);
        assert_eq!(resume_offset(1_500, 1_000), None);
    }

    #[test]
    fn part_files_cannot_collide_with_the_final_name() {
        assert_eq!(
            plan().part_path(),
            PathBuf::from("/tmp/models/pq2.gguf.part")
        );
    }

    #[test]
    fn disk_preflight_blocks_a_download_that_cannot_finish() {
        let p = plan();
        assert!(disk_preflight(10_000, &p, 5_000).is_ok());
        let err = disk_preflight(2_000, &p, 5_000).unwrap_err();
        assert!(matches!(
            err,
            ModelsError::NotEnoughDisk {
                needed_bytes: 6_000,
                ..
            }
        ));
        // Unknown free space must not silently pass as "enough"; the caller warns.
        assert!(disk_preflight(0, &p, 5_000).is_ok());
    }

    #[test]
    fn url_matches_the_hf_resolve_contract() {
        let catalog =
            crate::Catalog::from_toml_str(include_str!("../../../catalog/models.toml")).unwrap();
        let model = catalog.model("ternary-bonsai-2-27b").unwrap();
        let pack = model.pack("PQ2_0").unwrap();
        let p = model.plan_for(pack, Path::new("/models"), Some("HF_TOKEN".into()));
        assert_eq!(p.url, "https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf/resolve/main/Ternary-Bonsai-2-27B-PQ2_0.gguf");
        assert_eq!(p.expected_size_bytes, 7_206_168_928);
        assert_eq!(
            p.dest,
            PathBuf::from("/models/Ternary-Bonsai-2-27B-PQ2_0.gguf")
        );
        assert_eq!(p.token_env.as_deref(), Some("HF_TOKEN"));
    }
}
