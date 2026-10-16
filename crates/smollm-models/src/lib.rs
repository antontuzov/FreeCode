//! Model catalog, hardware-aware packing choice, downloads and integrity for SmoLLM Studio.
//!
//! This crate answers three questions and nothing else:
//!   * which models can we run, from `catalog/models.toml` (`catalog`)
//!   * which packing should this machine get, and can it fit (`hardware`, `select`)
//!   * is the file on disk the file Hugging Face published (`gguf`, `download`)
//!
//! The engine that actually loads these files is `smollm-llama`; nothing here launches a process.

pub mod catalog;
pub mod download;
pub mod gguf;
pub mod hardware;
pub mod lock;
pub mod select;
pub mod validate;

use std::path::{Path, PathBuf};

pub use catalog::{Catalog, ModelEntry, Pack, VisionPack};
pub use download::{DownloadPlan, Progress};
pub use gguf::{GgufHeader, IntegrityStatus};
pub use hardware::{host_os, Backend, GpuArch, HostHardware};
pub use lock::{EngineLock, EngineTarget};
pub use select::{recommend, Recommendation, DEFAULT_HEADROOM_BYTES};
pub use validate::{AvailableTarget, Violation};

/// Repo-relative default locations, so the desktop app and CI read the same files.
pub const CATALOG_REL: &str = "catalog/models.toml";
pub const ENGINE_LOCK_REL: &str = "engines/lock.toml";

#[derive(Debug, thiserror::Error)]
pub enum ModelsError {
    #[error("cannot read catalog {path}: {source}")]
    ReadCatalog {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse catalog {path}: {source}")]
    ParseCatalog {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot read model file {}: {source}", .path.display())]
    ReadModel {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot write model file {}: {source}", .path.display())]
    WriteModel {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not a readable GGUF: {reason}", .path.display())]
    NotGguf { path: PathBuf, reason: String },
    #[error("cannot check free space for {}: the size arithmetic overflowed", .path.display())]
    DiskSpaceUnknown { path: PathBuf },
    #[error(
        "not enough disk space for {}: {} free, {} needed",
        .path.display(),
        crate::format_bytes(*.available_bytes),
        crate::format_bytes(*.needed_bytes)
    )]
    NotEnoughDisk {
        path: PathBuf,
        available_bytes: u64,
        needed_bytes: u64,
    },
    #[error("download of {url} failed: {source}")]
    Http { url: String, source: reqwest::Error },
    #[error("download of {url} failed: HTTP {status}")]
    HttpStatus { url: String, status: u16 },
    #[error("{} failed integrity verification: {}", .path.display(), .status.describe())]
    Integrity {
        path: PathBuf,
        status: Box<IntegrityStatus>,
    },
    #[error("catalog validation found {0} violation(s)")]
    Invalid(usize),
}

/// Human-readable size for error strings and the Model Manager UI.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

impl Catalog {
    /// Load the repo's catalog from a workspace root.
    pub fn load_default(root: &Path) -> Result<Self, ModelsError> {
        Self::load(&root.join(CATALOG_REL))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_formatting_is_stable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(5_946_648_928), "5.54 GiB");
    }

    #[test]
    fn catalog_loads_from_the_repo_root() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = Catalog::load_default(&root).expect("seeded catalog loads");
        let ids: Vec<&str> = catalog.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "ternary-bonsai-2-27b",
                "ternary-bonsai-1-7b",
                "ternary-bonsai-2-4b"
            ]
        );
        assert_eq!(catalog.models[0].packs.len(), 3);

        // The 1.7B lists four files but offers two: the F16 reference is a baseline, and the
        // card-recommended legacy Q2_0 pack is the one the pinned engine refuses to load.
        let small = catalog.model("ternary-bonsai-1-7b").unwrap();
        assert_eq!(small.packs.len(), 4);
        let offered: Vec<&str> = small
            .downloadable_packs()
            .iter()
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(offered, ["PQ2_0", "Q2_0_g64"]);

        // The 4B is the vision size of the Bonsai 2 family: three language packs with the group-64
        // band registered but not offered (this app only ever launches the fork, where PQ2_0 wins),
        // and the projector the card recommends rather than the one that can crash.
        let mid = catalog.model("ternary-bonsai-2-4b").unwrap();
        assert_eq!(mid.packs.len(), 3);
        let offered: Vec<&str> = mid
            .downloadable_packs()
            .iter()
            .map(|p| p.key.as_str())
            .collect();
        assert_eq!(offered, ["PQ2_0", "PTQ1_0"]);
        assert_eq!(
            mid.recommended_vision().map(|v| v.file.as_str()),
            Some("Ternary-Bonsai-2-4B-mmproj-Q8_0.gguf")
        );
        assert_eq!(
            mid.engine.required_metadata_prefixes,
            ["prism.hadamard."],
            "a rotated export is only provably Prism's if the metadata says so"
        );
        assert!(
            mid.benchmarks.bonsai.is_none() && mid.benchmarks.no_published_score.is_some(),
            "the 4B has no published suite, which has to be stated rather than left blank"
        );
    }
}
