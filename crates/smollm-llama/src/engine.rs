//! Resolving *which* PrismML fork binary this machine may run — without launching it.
//!
//! Launching, supervision and the streaming client are Phase 2. Everything here is pure
//! filesystem + lock arithmetic, so the load-guard is unit-testable and CI can assert that no code
//! path can reach a stock llama.cpp build.
//!
//! Engine identity has three independent proofs, and a launch needs every one available to it:
//!   1. the binary lives under `engines/<pinned_tag>/`, i.e. we placed it there from a release
//!      asset rather than finding it on `PATH`;
//!   2. the pinned tag's build number is at or above `min_allowed_tag` — `prism-b10687` shipped no
//!      binaries at all, so an older pin cannot work;
//!   3. the binary's own `--version` banner reports the commit recorded in `engines/lock.toml`
//!      (checked via `banner_matches_pin` once Phase 2 can run the subprocess).

use std::path::{Path, PathBuf};

use smollm_models::hardware::{Backend, HostHardware};
use smollm_models::lock::{EngineLock, EngineTarget};
use smollm_models::validate::build_number;

use crate::LlamaError;

/// llama.cpp's own release CI unpacks to `<asset-dir>/build/bin/<binary>`. We also accept `bin/`
/// and the directory root so a hand-unpacked or from-source engine still resolves.
const BIN_SUBDIRS: &[&str] = &["build/bin", "bin", "."];

#[derive(Debug, Clone, PartialEq)]
pub struct EngineBinary {
    pub target: EngineTarget,
    /// `engines/<tag>/<asset-dir>/`, exactly as `EngineLock::install_dir` builds it.
    pub dir: PathBuf,
    /// Absolute path of the server binary, whether or not it exists yet.
    pub server_path: PathBuf,
    /// True when `server_path` is a file on disk.
    pub installed: bool,
}

impl EngineBinary {
    /// Resolve the fork build for the machine we are running on.
    pub fn resolve(
        root: &Path,
        lock: &EngineLock,
        host: &HostHardware,
    ) -> Result<Self, LlamaError> {
        Self::resolve_for(
            root,
            lock,
            std::env::consts::OS,
            std::env::consts::ARCH,
            host.backend,
        )
    }

    /// Resolve for an explicit triple. `UnsupportedBackend` carries the reason copied verbatim out
    /// of `engines/lock.toml`, which quotes KNOWN_ISSUES.md — SYCL cannot load PQ2_0/PTQ1_0 at all
    /// (llama.cpp#235), so it must never be chosen.
    pub fn resolve_for(
        root: &Path,
        lock: &EngineLock,
        os: &str,
        arch: &str,
        backend: Backend,
    ) -> Result<Self, LlamaError> {
        if let Some(reason) = lock.unsupported_reason(backend) {
            return Err(LlamaError::UnsupportedBackend {
                backend: backend.label(),
                reason: reason.to_string(),
            });
        }
        let target = lock
            .target_for(os, arch, backend)
            .ok_or_else(|| LlamaError::NoEngineTarget {
                os: os.to_string(),
                arch: arch.to_string(),
                backend: backend.label(),
            })?
            .clone();
        Self::from_target(root, lock, &target)
    }

    /// Build a resolution from a specific target — used when the user forces a backend in the
    /// Model Manager rather than accepting the auto-detected one.
    pub fn from_target(
        root: &Path,
        lock: &EngineLock,
        target: &EngineTarget,
    ) -> Result<Self, LlamaError> {
        Self::assert_tag_allowed(lock)?;
        let dir = lock.install_dir(&engines_root(root), target);
        let default_path = dir.join(lock.server_binary());
        let server_path = find_server(&dir, lock.server_binary()).unwrap_or(default_path);
        Ok(Self {
            target: target.clone(),
            dir,
            installed: server_path.is_file(),
            server_path,
        })
    }

    /// Proof 2, checked before anything is launched.
    pub fn assert_tag_allowed(lock: &EngineLock) -> Result<(), LlamaError> {
        let pinned = build_number(&lock.pinned_tag)
            .ok_or_else(|| LlamaError::UnparseableTag(lock.pinned_tag.clone()))?;
        let floor = build_number(&lock.min_allowed_tag)
            .ok_or_else(|| LlamaError::UnparseableTag(lock.min_allowed_tag.clone()))?;
        if pinned < floor {
            return Err(LlamaError::EngineTooOld {
                pinned: lock.pinned_tag.clone(),
                floor: lock.min_allowed_tag.clone(),
            });
        }
        Ok(())
    }

    /// Refuse a binary that is not inside our `engines/` tree. This is what blocks the
    /// "just use the llama.cpp on my PATH" path: stock llama.cpp rejects PTQ1_0/PQ2_0 as unknown
    /// quant types, and loads the F16 reference into garbage because it never applies the
    /// activation transform (llama.cpp#242).
    pub fn assert_not_stock_engine(&self, root: &Path) -> Result<(), LlamaError> {
        let under_engines = self
            .server_path
            .canonicalize()
            .unwrap_or_else(|_| self.server_path.clone())
            .starts_with(engines_root(root));
        if under_engines {
            Ok(())
        } else {
            Err(LlamaError::StockEngineRejected {
                path: self.server_path.clone(),
            })
        }
    }

    /// Where to get the asset this resolution points at, for the install call-to-action.
    pub fn download_url(&self, lock: &EngineLock) -> String {
        lock.download_url(&self.target)
    }

    pub fn release_page(&self, lock: &EngineLock) -> String {
        format!(
            "https://github.com/{}/releases/tag/{}",
            lock.repo, lock.pinned_tag
        )
    }

    pub fn missing_message(&self, lock: &EngineLock) -> String {
        format!(
            "engine {} is not installed (expected {} from {}). Download and unpack it into {}, or build the {} branch from source.",
            lock.pinned_tag,
            self.server_path.display(),
            self.target.asset,
            self.dir.display(),
            lock.from_source.branch,
        )
    }
}

/// `root/engines`, the directory the lock file lives beside.
pub fn engines_root(root: &Path) -> PathBuf {
    root.join("engines")
}

/// Default weight directory; the app overrides it per workspace.
pub fn models_root(root: &Path) -> PathBuf {
    root.join("models")
}

fn find_server(dir: &Path, binary: &str) -> Option<PathBuf> {
    BIN_SUBDIRS
        .iter()
        .map(|sub| dir.join(sub).join(binary))
        .find(|p| p.is_file())
}

/// The build number and commit from an engine banner. llama.cpp prints
/// `version: <build> (<short-sha>)`; anything we cannot parse is reported as absent rather than
/// guessed, so an unrecognisable banner can never be mistaken for a verified fork build.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineVersion {
    pub build: Option<u32>,
    pub commit: Option<String>,
}

pub fn parse_version(banner: &str) -> EngineVersion {
    let build = banner
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|tok| tok.len() >= 5 && tok.chars().all(|c| c.is_ascii_digit()))
        .and_then(|tok| tok.parse().ok());
    let commit = banner
        .split('(')
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit()));
    EngineVersion { build, commit }
}

/// Proof 3: the banner must name the commit we pinned and verified the catalog against. A different
/// commit is how the AVX-512 `PQ2_0` load crash (#245) or the Vulkan silent-CPU-fallback (#238)
/// get reintroduced without anything visible changing.
pub fn banner_matches_pin(banner: &str, lock: &EngineLock) -> bool {
    parse_version(banner)
        .commit
        .as_deref()
        .is_some_and(|c| lock.pinned_commit.eq_ignore_ascii_case(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_models::Catalog;
    use std::path::Path;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn lock() -> EngineLock {
        EngineLock::load_default(&repo_root()).expect("the repo's engine lock must parse")
    }

    #[test]
    fn every_pinned_target_resolves_inside_the_pinned_tag_directory() {
        let root = repo_root();
        let lock = lock();
        assert!(!lock.targets.is_empty());
        for t in lock.targets.iter() {
            let e =
                EngineBinary::from_target(&root, &lock, t).expect("a pinned target must resolve");
            assert_eq!(&e.target.key, &t.key);
            let dir = e.dir.display().to_string();
            assert!(
                dir.contains(&lock.pinned_tag),
                "engine path must carry the tag: {dir}"
            );
            assert!(
                dir.contains(&t.key),
                "engine path must carry the target key: {dir}"
            );
            assert!(e
                .download_url(&lock)
                .starts_with("https://github.com/PrismML-Eng/llama.cpp/releases/download/"));
        }
    }

    #[test]
    fn auto_resolution_prefers_the_recommended_asset_for_a_backend() {
        let root = repo_root();
        let lock = lock();
        // Windows/CUDA has both a 12.4 and a 13.3 asset; 13.3 crashes on load (KNOWN_ISSUES #222),
        // so the recommended one must win.
        let e = EngineBinary::resolve_for(&root, &lock, "windows", "x86_64", Backend::Cuda)
            .expect("windows/cuda is pinned");
        assert!(e.target.recommended, "{}", e.target.key);
        assert!(e.target.asset.contains("cuda-12.4"), "{}", e.target.asset);
    }

    #[test]
    fn sycl_is_refused_with_the_reason_from_the_lock_file() {
        let lock = lock();
        let err =
            EngineBinary::resolve_for(Path::new("."), &lock, "linux", "x86_64", Backend::Sycl)
                .expect_err("SYCL cannot load the ternary packs");
        match err {
            LlamaError::UnsupportedBackend { reason, .. } => {
                assert!(reason.contains("235"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unknown_triple_reports_no_target_instead_of_a_fabricated_path() {
        let root = repo_root();
        let lock = lock();
        let err = EngineBinary::resolve_for(&root, &lock, "freebsd", "riscv64", Backend::Metal)
            .expect_err("no asset is pinned for this triple");
        assert!(matches!(err, LlamaError::NoEngineTarget { .. }), "{err:?}");
    }

    #[test]
    fn version_banners_are_parsed_conservatively() {
        let v = parse_version("version: 10772 (e8fd209)\nbuilt with AppleClang 17 for arm64");
        assert_eq!(v.build, Some(10772));
        assert_eq!(v.commit.as_deref(), Some("e8fd209"));

        let lock = lock();
        assert!(banner_matches_pin(
            &format!("version: 10772 ({})", lock.pinned_commit),
            &lock
        ));
        // Stock llama.cpp reports an upstream commit, so the pin check rejects it.
        assert!(!banner_matches_pin("version: 10772 (0a1b2c3)", &lock));
        assert!(!banner_matches_pin("llama-server", &lock));
        assert_eq!(parse_version("llama-server").build, None);
    }

    #[test]
    fn a_pin_below_the_floor_blocks_resolution() {
        let mut lock = lock();
        // prism-b10687 is the launch-day tag, which published no binaries at all.
        lock.pinned_tag = "prism-b10687-9a9394a".into();
        let err = EngineBinary::assert_tag_allowed(&lock).expect_err("floor must reject it");
        match err {
            LlamaError::EngineTooOld { pinned, floor } => {
                assert_eq!(pinned, "prism-b10687-9a9394a");
                assert_eq!(floor, lock.min_allowed_tag);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_binary_outside_engines_is_rejected_as_stock_llama() {
        let root = repo_root();
        let lock = lock();
        let target = lock.targets.first().expect("targets seeded").clone();
        let mut e = EngineBinary::from_target(&root, &lock, &target).unwrap();
        e.server_path = PathBuf::from("/opt/homebrew/bin/llama-server");
        let err = e
            .assert_not_stock_engine(&root)
            .expect_err("PATH binary is not the fork");
        assert!(
            matches!(err, LlamaError::StockEngineRejected { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn the_pinned_engine_satisfies_the_catalog_entrys_own_minimum() {
        let catalog = Catalog::load_default(&repo_root()).expect("catalog must load");
        let model = catalog.models.first().expect("one seeded model");
        lock()
            .satisfies(model)
            .expect("the pinned build must meet the model's minimum release tag");
    }
}
