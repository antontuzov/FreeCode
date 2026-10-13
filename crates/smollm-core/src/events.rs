//! The event stream between the agent loop and the window.
//!
//! Everything the transcript, the status loader line, the "+N −M" pill and the "Local" badge render
//! from arrives as one of these, in order, across the IPC boundary — which is why no step may run
//! silently: a side effect that produced no event would be invisible in the UI.
//!
//! Reasoning and answer text are separate deltas on purpose. The fork re-renders earlier reasoning
//! into later prompts and the prompt cache then misses on every turn of a tool loop
//! (Bonsai-demo#183), so `ThoughtDelta` feeds the collapsed block and only `AnswerDelta` is replayed
//! into history.

use serde::{Deserialize, Serialize};
use smollm_config::agent::PermissionClass;
use smollm_llama::load_guard::Blocker;
use smollm_models::EngineLock;

use crate::transcript::{DiffStat, TranscriptStep};

/// A load-guard refusal rendered in the transcript: localized copy plus the one action that clears
/// it. The acceptance criteria name this path — a stock-llama.cpp attempt must show a clear error
/// with an install-fork CTA, not a crash log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedInfo {
    pub title: String,
    pub detail: String,
    pub cta_label: String,
    pub cta_url: String,
}

impl BlockedInfo {
    /// `model_repo` is the Hugging Face repo the weights came from; the blockers about the file on
    /// disk point there, while the ones about the engine point at the GitHub fork in `lock`.
    pub fn from_blocker(blocker: &Blocker, lock: &EngineLock, model_repo: &str) -> Self {
        let cta = blocker.cta(lock, model_repo);
        Self {
            title: blocker.title().to_string(),
            detail: blocker.detail(),
            cta_label: cta.label.to_string(),
            cta_url: cta.url,
        }
    }
}

/// What a tool call produced, in the shape its transcript row shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutcome {
    pub ok: bool,
    /// One-line result text under the row label.
    pub summary: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub diff: Option<DiffStat>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

impl ToolOutcome {
    pub fn ok(summary: String) -> Self {
        Self {
            ok: true,
            summary,
            exit_code: None,
            diff: None,
            artifacts: Vec::new(),
        }
    }

    pub fn blocked(reason: &str) -> Self {
        Self {
            ok: false,
            summary: reason.to_string(),
            exit_code: None,
            diff: None,
            artifacts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub kind: String,
    pub title: String,
    /// Workspace-relative, never absolute: paths shown in the UI must not leak the home directory.
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum AgentEvent {
    /// A run began. `local` drives the "Local" badge and is false only when a remote provider is
    /// actually configured.
    RunStarted {
        run_id: String,
        model_label: String,
        local: bool,
    },
    /// Collapsed reasoning text, streamed.
    ThoughtDelta {
        text: String,
    },
    /// Answer text, streamed.
    AnswerDelta {
        text: String,
    },
    /// A tool row appeared in the `Running… Ns` state.
    ToolStarted {
        step: TranscriptStep,
    },
    /// A tool row finished.
    ToolFinished {
        step: TranscriptStep,
        outcome: ToolOutcome,
    },
    /// The "Ran N tools" batch header, emitted when the batch closes.
    ToolBatchClosed {
        step: TranscriptStep,
    },
    /// Approval required before the call may run.
    ApprovalRequested {
        call_id: String,
        headline: String,
        permission: PermissionClass,
        timeout_ms: u64,
    },
    ApprovalResolved {
        call_id: String,
        decision: crate::approvals::ApprovalDecision,
    },
    /// Staged edits, so the pill updates during the run rather than only at the end.
    DiffStaged {
        diff: DiffStat,
    },
    /// The "Context compressed" divider.
    ContextCompressed {
        before_tokens: u32,
        after_tokens: u32,
        summary: String,
    },
    /// A sandbox refusal, surfaced as its own row with the reason.
    SandboxBlocked {
        tool_id: String,
        code: String,
        reason: String,
    },
    /// The load-guard refused to launch: stock engine, missing pin, or a file that is not a Prism
    /// export.
    EngineBlocked {
        info: BlockedInfo,
    },
    /// A catalog caveat that matches this host (its KNOWN_ISSUES rows).
    Warning {
        code: String,
        message: String,
    },
    /// The run ended. `stopped` distinguishes the stop button from a natural finish.
    RunFinished {
        run_id: String,
        stopped: bool,
        total_diff: Option<DiffStat>,
    },
    Failed {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::{StepKind, TranscriptStep};
    use serde_json::json;
    use smollm_tools::sandbox::SandboxViolation;
    use std::path::Path;

    fn lock() -> EngineLock {
        EngineLock::load_default(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap()
    }

    #[test]
    fn events_are_self_describing_on_the_wire() {
        let e = AgentEvent::RunStarted {
            run_id: "r1".into(),
            model_label: "Ternary Bonsai 2 27B (PQ2_0)".into(),
            local: true,
        };
        let raw = serde_json::to_value(&e).unwrap();
        assert_eq!(raw["type"], json!("runStarted"), "camelCase discriminator");
        assert_eq!(raw["modelLabel"], json!("Ternary Bonsai 2 27B (PQ2_0)"));
        assert_eq!(raw["local"], json!(true));
    }

    #[test]
    fn reasoning_and_answer_deltas_are_distinct_events() {
        let t = serde_json::to_value(AgentEvent::ThoughtDelta {
            text: "checking".into(),
        })
        .unwrap();
        let a = serde_json::to_value(AgentEvent::AnswerDelta {
            text: "done".into(),
        })
        .unwrap();
        assert_eq!(t["type"], json!("thoughtDelta"));
        assert_eq!(a["type"], json!("answerDelta"));
    }

    #[test]
    fn a_blocked_launch_carries_the_action_that_fixes_it() {
        let blocker = Blocker::StockEngineRejected {
            path: Path::new("/opt/homebrew/bin/llama-server").to_path_buf(),
        };
        let info =
            BlockedInfo::from_blocker(&blocker, &lock(), "prism-ml/Ternary-Bonsai-2-27B-gguf");
        assert_eq!(info.title, "This model cannot run on standard llama.cpp");
        assert_eq!(info.cta_label, "Install the PrismML engine");
        assert!(info
            .cta_url
            .starts_with("https://github.com/PrismML-Eng/llama.cpp/releases/"));
        assert!(info.detail.contains("no automatic fallback"));
        let wire = serde_json::to_value(&info).unwrap();
        assert_eq!(wire["ctaUrl"], json!(info.cta_url));
    }

    #[test]
    fn tool_rows_round_trip_with_their_step_and_outcome() {
        let step = TranscriptStep::running_tool(
            "edit_file",
            StepKind::EditFile,
            json!({ "path": "src/app.rs" }),
        );
        let outcome = ToolOutcome {
            diff: Some(DiffStat {
                lines_added: 12,
                lines_removed: 4,
                files_changed: 1,
            }),
            ..ToolOutcome::ok("1 file edited".into())
        };
        let event = AgentEvent::ToolFinished {
            step: step.clone(),
            outcome,
        };
        let raw = serde_json::to_string(&event).unwrap();
        let back: AgentEvent = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, event);
        assert!(raw.contains("filesChanged"), "camelCase on the wire: {raw}");
        assert!(!raw.contains("files_changed"), "snake_case leaked: {raw}");
    }

    #[test]
    fn a_sandbox_refusal_serializes_its_own_reason() {
        let violation = SandboxViolation::OutsideWorkspace {
            path: "/etc/hosts".into(),
            root: "/work".into(),
        };
        let e = AgentEvent::SandboxBlocked {
            tool_id: "write_file".into(),
            code: violation.code().into(),
            reason: violation.to_string(),
        };
        let raw = serde_json::to_value(&e).unwrap();
        assert_eq!(raw["type"], json!("sandboxBlocked"));
        assert!(
            raw["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("outside the workspace"),
            "{raw}"
        );
        assert_eq!(raw["code"], json!("outside-workspace"));
        // A protected-path refusal gets its own code, so the row names the rule it enforced.
        assert_eq!(
            SandboxViolation::DeniedPath {
                path: "/work/.env".into(),
                hit: ".env".into(),
            }
            .code(),
            "denied-path"
        );
    }

    #[test]
    fn an_approval_resolution_travels_as_a_typed_decision() {
        let e = AgentEvent::ApprovalResolved {
            call_id: "c1".into(),
            decision: crate::approvals::ApprovalDecision::UserDenied,
        };
        let raw = serde_json::to_value(&e).unwrap();
        assert_eq!(raw["decision"], json!("userDenied"));
        assert_eq!(raw["callId"], json!("c1"));
    }
}
