//! The PrismML llama.cpp fork: which build this machine may run, and whether a given weight file
//! is allowed to be handed to it.
//!
//! Two responsibilities, both deliberately free of subprocess calls so they can be unit-tested and
//! asserted in CI:
//!   * [`engine`] — resolve the pinned fork asset for this OS/arch/backend, and prove the pin is
//!     new enough to exist.
//!   * [`load_guard`] — refuse everything else: stock llama.cpp, an unpinned build, or a
//!     rotated-basis GGUF that lacks the `prism.hadamard.*` metadata proving it is a Prism export.
//!
//! Launching a supervised `llama-server`, the verified flag set and the OpenAI-compatible streaming
//! client are Phase 2 and live beside this, not inside it. Nothing in this crate ever falls back to
//! a system llama.cpp, because for these packings a fallback produces *silently wrong output*
//! rather than an error.

pub mod engine;
pub mod load_guard;

use std::path::PathBuf;

pub use engine::{
    banner_matches_pin, engines_root, models_root, parse_version, EngineBinary, EngineVersion,
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
}
