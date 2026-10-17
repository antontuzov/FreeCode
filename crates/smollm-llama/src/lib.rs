//! The PrismML llama.cpp fork: which build this machine may run, whether a given weight file is
//! allowed to be handed to it, and how to run it once it is.
//!
//! Resolution, guarding and flag assembly are deliberately free of subprocess calls so they can be
//! unit-tested and asserted in CI:
//!   * [`engine`] — resolve the pinned fork asset for this OS/arch/backend, and prove the pin is
//!     new enough to exist.
//!   * [`install`] — fetch that exact asset and unpack it, checksum-verified, into the directory the
//!     resolver looks in.
//!   * [`load_guard`] — refuse everything else: stock llama.cpp, an unpinned build, or a
//!     rotated-basis GGUF that lacks the `prism.hadamard.*` metadata proving it is a Prism export.
//!   * [`launch`] — assemble the `llama-server` argument vector the demo scripts are known to work
//!     with, for this backend, packing and RAM.
//!
//! Only [`server`] and [`client`] touch a process or a socket: the first starts the plan and kills it
//! on drop, the second speaks OpenAI to it under the constraints the model's own issue list imposes.
//!
//! Nothing here falls back to a system llama.cpp, because for these packings a fallback produces
//! *silently wrong output* rather than an error.

pub mod client;
pub mod engine;
pub mod install;
pub mod launch;
pub mod load_guard;
pub mod server;

use std::path::PathBuf;

pub use client::{
    chat_body, clamp_budget, normalize, prompt_tokens, stream_chat, wire_effort, ChatRequest,
    Message, Reply, ReplyDelta, RequestError, Role, SseDecoder, SseEvent, StreamControl, Timings,
    ToolCall,
};
pub use engine::{
    banner_matches_pin, engines_root, models_root, parse_version, EngineBinary, EngineVersion,
};
pub use install::{
    install, plan_for, prepare_scratch, tag_dir, unpack, Installed, Phase, PARTS_DIR, STAGING_DIR,
};
pub use launch::{
    context_from_ram, layers_to_offload, LaunchPlan, LaunchRequest, LaunchTuning, IMAGE_TOKEN_CAP,
};
pub use load_guard::{
    caveat_matches, check, Blocker, BlockerCode, CallToAction, CtaCode, FileState, GuardReport,
    GuardRequest, Warning, WarningCode,
};
pub use server::{
    kill_spawned_servers, pick_free_port, reap_orphaned_servers, start, verify_version,
    ServerHandle,
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
    #[error("cannot launch: {path} is not downloaded yet")]
    WeightsNotInstalled { path: PathBuf },
    #[error(
        "refusing to launch with the {kind} KV cache: it is not one of the types the model card recommends ({allowed:?})"
    )]
    KvCacheRejected { kind: String, allowed: Vec<String> },
    #[error("cannot start {}: {source}", .program.display())]
    Spawn {
        program: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{program} does not report the pinned engine commit {expected} (it reports {found}), so it is not the build the catalog was verified against:\n{banner}"
    )]
    VersionMismatch {
        program: String,
        expected: String,
        found: String,
        banner: String,
    },
    #[error("{program} did not answer `--version` within {after_secs}s")]
    VersionProbeStalled { program: PathBuf, after_secs: u64 },
    #[error("llama-server exited ({status}) before it became healthy.\n{log}")]
    ServerExited { status: String, log: String },
    #[error(
        "llama-server never reported a healthy /health at {url} within {after_secs}s (last answer: {status}).\n{log}"
    )]
    ServerUnhealthy {
        url: String,
        after_secs: u64,
        status: String,
        log: String,
    },
    #[error("the llama.cpp server could not be reached: {0}")]
    Http(#[from] reqwest::Error),
    #[error(
        "the llama.cpp server answered {status}:\n{body}\nThis is the template's own refusal — see the model's KNOWN_ISSUES.md."
    )]
    ServerError { status: u16, body: String },
    #[error(transparent)]
    Request(#[from] client::RequestError),
}
