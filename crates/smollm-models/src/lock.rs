//! `engines/lock.toml`: which fork build we ship per platform, and where to get it.
//!
//! Parsing lives here because the lock is catalog data — it is the other half of "what can we run".
//! Resolution, fetching and launching live in `smollm-llama`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::ModelEntry;
use crate::hardware::Backend;
use crate::validate::AvailableTarget;
use crate::ModelsError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineLock {
    pub schema_version: u32,
    pub pinned_tag: String,
    pub pinned_commit: String,
    pub published_at: String,
    pub min_allowed_tag: String,
    pub repo: String,
    pub download_url_template: String,
    #[serde(default)]
    pub demo_pinned_tag: Option<String>,
    pub binaries: BTreeMap<String, String>,
    #[serde(default, rename = "target")]
    pub targets: Vec<EngineTarget>,
    #[serde(default)]
    pub unsupported_backends: BTreeMap<String, String>,
    pub from_source: FromSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineTarget {
    pub key: String,
    pub os: String,
    pub arch: String,
    pub backend: Backend,
    pub asset: String,
    pub size_bytes: u64,
    /// GitHub's own `assets[].digest` for the pinned release. Required rather than defaulted: an
    /// asset we cannot checksum is an asset the installer must refuse to unpack.
    pub sha256: String,
    pub recommended: bool,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FromSource {
    pub repo: String,
    pub branch: String,
    pub macos: String,
    pub cuda: String,
    #[serde(default)]
    pub cuda_arch_note: String,
    #[serde(default)]
    pub known_blocker: String,
}

impl EngineLock {
    pub fn load(path: &Path) -> Result<Self, ModelsError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ModelsError::ReadCatalog {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&raw).map_err(|source| ModelsError::ParseCatalog {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn load_default(root: &Path) -> Result<Self, ModelsError> {
        Self::load(&root.join(crate::ENGINE_LOCK_REL))
    }

    /// Flatten for catalog validation.
    pub fn available_targets(&self) -> Vec<AvailableTarget> {
        self.targets
            .iter()
            .map(|t| AvailableTarget {
                os: t.os.clone(),
                arch: t.arch.clone(),
                backend: t.backend,
                asset: t.asset.clone(),
                recommended: t.recommended,
            })
            .collect()
    }

    /// The asset for this machine. `os`/`arch` come from `std::env::consts`, so a build for one
    /// platform never selects another platform's binary.
    pub fn target_for(&self, os: &str, arch: &str, backend: Backend) -> Option<&EngineTarget> {
        let mut matches: Vec<&EngineTarget> = self
            .targets
            .iter()
            .filter(|t| t.os == os && t.arch == arch && t.backend == backend)
            .collect();
        matches.sort_by_key(|t| !t.recommended);
        matches.first().copied()
    }

    pub fn download_url(&self, target: &EngineTarget) -> String {
        self.download_url_template
            .replace("{tag}", &self.pinned_tag)
            .replace("{asset}", &target.asset)
    }

    /// Directory the resolved engine is unpacked into: `engines/<tag>/<target key>`.
    pub fn install_dir(&self, engines_root: &Path, target: &EngineTarget) -> PathBuf {
        engines_root.join(&self.pinned_tag).join(&target.key)
    }

    pub fn server_binary(&self) -> &str {
        self.binaries
            .get("server")
            .map(String::as_str)
            .unwrap_or("llama-server")
    }

    /// Pinned release must be at or above the tag the catalog demands, and must be a tag we can
    /// compare at all.
    pub fn satisfies(&self, model: &ModelEntry) -> Result<(), String> {
        let want =
            crate::validate::build_number(&model.engine.min_release_tag).ok_or_else(|| {
                format!(
                    "catalog min_release_tag {:?} has no build number",
                    model.engine.min_release_tag
                )
            })?;
        let have = crate::validate::build_number(&self.pinned_tag)
            .ok_or_else(|| format!("pinned_tag {:?} has no build number", self.pinned_tag))?;
        let floor = crate::validate::build_number(&self.min_allowed_tag).unwrap_or(want);
        if have < want {
            return Err(format!(
                "pinned engine {} (build {have}) is older than the catalog requirement {} (build {want})",
                self.pinned_tag, model.engine.min_release_tag
            ));
        }
        if have < floor {
            return Err(format!(
                "pinned engine {} (build {have}) is below this lock's own floor {} (build {floor})",
                self.pinned_tag, self.min_allowed_tag
            ));
        }
        Ok(())
    }

    /// The reason a backend has no path, for a user-facing error instead of a bare "not found".
    pub fn unsupported_reason(&self, backend: Backend) -> Option<&str> {
        self.unsupported_backends
            .get(backend.key())
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> EngineLock {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        EngineLock::load_default(&root).expect("seeded engine lock loads")
    }

    fn model() -> ModelEntry {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        Catalog::load_default(&root)
            .unwrap()
            .model("ternary-bonsai-2-27b")
            .unwrap()
            .clone()
    }

    use crate::catalog::Catalog;

    #[test]
    fn every_distributable_backend_resolves_on_macos_and_windows_or_linux() {
        let l = lock();
        assert_eq!(
            l.target_for("darwin", "aarch64", Backend::Metal)
                .map(|t| t.backend),
            Some(Backend::Metal)
        );
        assert_eq!(
            l.target_for("windows", "x86_64", Backend::Cuda)
                .map(|t| t.recommended),
            Some(true)
        );
        assert!(
            l.target_for("linux", "x86_64", Backend::Vulkan).is_none(),
            "we ship no Vulkan Linux asset, so none is claimed"
        );
        assert!(
            l.target_for("darwin", "x86_64", Backend::Metal).is_none(),
            "Intel Macs have no Metal path"
        );
    }

    #[test]
    fn sycl_has_an_explained_absence() {
        let l = lock();
        assert!(l.target_for("linux", "x86_64", Backend::Sycl).is_none());
        assert!(l.unsupported_reason(Backend::Sycl).is_some());
    }

    #[test]
    fn pinned_release_meets_the_catalog_floor() {
        let l = lock();
        assert!(l.satisfies(&model()).is_ok(), "{:?}", l.satisfies(&model()));
    }

    #[test]
    fn an_older_pin_is_rejected() {
        let l = EngineLock {
            pinned_tag: "prism-b10687-abcdef0".into(),
            ..lock()
        };
        let err = l.satisfies(&model()).unwrap_err();
        assert!(err.contains("older than the catalog requirement"), "{err}");
    }

    #[test]
    fn urls_and_paths_are_built_from_the_lock() {
        let l = lock();
        let t = l.target_for("darwin", "aarch64", Backend::Metal).unwrap();
        assert_eq!(
            l.download_url(t),
            "https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b10787-73bfe6c/llama-prism-b10787-73bfe6c-bin-macos-arm64.tar.gz"
        );
        assert_eq!(
            l.install_dir(Path::new("engines"), t),
            PathBuf::from("engines/prism-b10787-73bfe6c/macos-arm64")
        );
        assert_eq!(l.server_binary(), "llama-server");
    }

    #[test]
    fn every_locked_asset_carries_a_sha256_the_installer_can_check() {
        let l = lock();
        for t in l.targets.iter() {
            assert_eq!(
                t.sha256.len(),
                64,
                "{} must record GitHub's sha256 digest, not a placeholder",
                t.key
            );
            assert!(
                t.sha256.chars().all(|c| c.is_ascii_hexdigit())
                    && t.sha256 == t.sha256.to_lowercase(),
                "{}: {:?} is not a lowercase hex sha256",
                t.key,
                t.sha256
            );
            assert!(
                t.size_bytes > 1_000_000,
                "{}: an engine asset is not {} bytes",
                t.key,
                t.size_bytes
            );
        }
    }

    #[test]
    fn cuda_12_4_is_the_recommended_windows_build() {
        let l = lock();
        let preferred = l
            .targets
            .iter()
            .filter(|t| t.os == "windows" && t.backend == Backend::Cuda && t.recommended)
            .collect::<Vec<_>>();
        assert_eq!(preferred.len(), 1);
        assert!(
            preferred[0].asset.contains("cuda-12.4"),
            "{}",
            preferred[0].asset
        );
    }
}
