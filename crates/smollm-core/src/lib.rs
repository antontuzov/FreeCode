//! The agent run model: what a step is, how it streams to the window, and who has to approve it.
//!
//! Deliberately contains no loop and no I/O. The pieces here are the vocabulary the loop in
//! `smollm_core::run` (Phase 3) will speak, and the types the Tauri commands serialize across the
//! IPC boundary:
//!   * [`transcript`] — step rows keyed to i18n catalog entries, plus the `+N −M` diff pill;
//!   * [`events`] — the ordered event stream the UI renders;
//!   * [`approvals`] — policy decisions, including the ones that block without asking.
//!
//! Two contract rules shape these types. Reasoning arrives as its own delta stream so it is never
//! replayed into the next prompt (Bonsai-demo#183). And a refused action carries the reason and the
//! fix: the load-guard's blocker becomes a transcript row with an "Install the PrismML engine" CTA,
//! not a stack trace.

pub mod approvals;
pub mod events;
pub mod transcript;

use std::path::PathBuf;

pub use approvals::{evaluate, request, ApprovalDecision, ApprovalRequest, Pending};
pub use events::{AgentEvent, ArtifactRef, BlockedInfo, ToolOutcome};
pub use transcript::{
    all_i18n_keys, context_compressed, running_line, tool_batch, DiffStat, StepKind, StepStatus,
    TranscriptStep,
};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("the run was stopped before it finished")]
    Stopped,
    #[error("no engine is available for {model}: {why}")]
    NoEngine { model: String, why: String },
    #[error(
        "the model file {path} may not be loaded: {why}. Only the pinned PrismML fork build can run these packings."
    )]
    LoadGuardBlocked { path: PathBuf, why: String },
    #[error("the engine stopped responding and was restarted")]
    EngineRestarted,
    #[error("the engine exited with {status}: {output}")]
    EngineExited { status: String, output: String },
    #[error("conversation state could not be persisted: {0}")]
    Persist(String),
    #[error(transparent)]
    Tools(#[from] smollm_tools::ToolsError),
    #[error(transparent)]
    Llama(#[from] smollm_llama::LlamaError),
    #[error(transparent)]
    Models(#[from] smollm_models::ModelsError),
    #[error(transparent)]
    Config(#[from] smollm_config::ConfigError),
}

/// Turn a load-guard blocker into the transcript payload the UI needs, at the one place that knows
/// both types.
pub fn blocked_info(
    blocker: &smollm_llama::load_guard::Blocker,
    lock: &smollm_models::EngineLock,
) -> BlockedInfo {
    BlockedInfo::from_blocker(blocker, lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_surface_names_the_fork_rather_than_a_generic_failure() {
        let e = CoreError::LoadGuardBlocked {
            path: PathBuf::from("/models/Ternary-Bonsai-2-27B-PQ2_0.gguf"),
            why: "stock llama.cpp cannot load PQ2_0".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("PrismML fork"), "{msg}");
        assert!(msg.contains("PQ2_0"), "{msg}");
    }

    #[test]
    fn errors_compose_from_the_lower_layers_without_losing_their_text() {
        let e: CoreError = smollm_llama::LlamaError::UnsupportedBackend {
            backend: "SYCL",
            reason: "cannot load ternary packings (llama.cpp#235)".into(),
        }
        .into();
        assert!(matches!(e, CoreError::Llama(_)));
        assert!(e.to_string().contains("SYCL"), "{e}");
    }

    #[test]
    fn the_i18n_surface_is_exportable_for_the_catalog_consistency_check() {
        assert!(!all_i18n_keys().is_empty());
    }
}
