//! The PrismML llama.cpp fork: which build this machine may run, and whether a given weight file
//! is allowed to be handed to it.
//!
//! Three responsibilities, all deliberately free of subprocess calls so they can be unit-tested and
//! asserted in CI:
//!   * [`engine`] — resolve the pinned fork asset for this OS/arch/backend, and prove the pin is
//!     new enough to exist.
//!   * [`install`] — fetch that exact asset and unpack it, checksum-verified, into the directory the
//!     resolver looks in.
//!   * [`load_guard`] — refuse everything else: stock llama.cpp, an unpinned build, or a
//!     rotated-basis GGUF that lacks the `prism.hadamard.*` metadata proving it is a Prism export.
//!
//! Launching a supervised `llama-server`, the verified flag set and the OpenAI-compatible streaming
//! client are Phase 2 and live beside this, not inside it. Nothing in this crate ever falls back to
//! a system llama.cpp, because for these packings a fallback produces *silently wrong output*
//! rather than an error.

pub mod engine;
pub mod install;
pub mod load_guard;

use std::path::PathBuf;

pub use engine::{
    banner_matches_pin, engines_root, models_root, parse_version, EngineBinary, EngineVersion,
};
pub use install::{
    install, plan_for, prepare_scratch, tag_dir, unpack, Installed, Phase, PARTS_DIR, STAGING_DIR,
};
pub use load_guard::{
    caveat_matches, check, Blocker, CallToAction, FileState, GuardReport, GuardRequest, Warning,
    WarningCode,
};

#[derive(Debug, thiserror::Error)]
pub enum LlamaError {
    #[error("{backend} has no usable engine build for the ternary packings: {reason}")]
    UnsupportedBackend {
        backend: &'static str,
        reason: String,
    },
    #[error("no fork asset is pinned for {os}/{arch}/{backend}; add it to engines/lock.toml")]
    NoEngineTarget {
        os: String,
        arch: String,
        backend: &'static str,
    },
    #[error("engine tag {0} carries no parseable build number")]
    UnparseableTag(String),
    #[error("pinned engine {pinned} is older than the minimum {floor}")]
    EngineTooOld { pinned: String, floor: String },
    #[error(
        "refusing to run {path}: the ternary packings require the pinned PrismML fork build under engines/"
    )]
    StockEngineRejected { path: PathBuf },
    #[error(
        "engine download failed: {0}\nThe asset is fetched from the GitHub release named in engines/lock.toml."
    )]
    Download(#[from] smollm_models::ModelsError),
    #[error("downloaded engine archive {path} did not pass its checksum: {detail}")]
    ArchiveUnverified { path: PathBuf, detail: String },
    #[error("installing {asset} was interrupted: {reason}")]
    InstallInterrupted { asset: String, reason: String },
    #[error("cannot install {asset}: unsupported archive format (expected .tar.gz or .zip)")]
    UnknownArchiveFormat { asset: String },
    #[error("cannot read the downloaded engine archive {asset}")]
    ReadArchive {
        asset: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot unpack {asset}: {source}")]
    UnpackTar {
        asset: String,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot unpack {asset}: {detail}")]
    UnpackZip { asset: String, detail: String },
    #[error("cannot write engine file {path}")]
    WriteEngine {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot move the unpacked engine into place at {path}")]
    SwapEngine {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "{asset} unpacked to {dir} but contains no {binary} \u{2014} refusing to install an engine we could not launch"
    )]
    ServerBinaryMissing {
        asset: String,
        dir: PathBuf,
        binary: String,
    },
}
