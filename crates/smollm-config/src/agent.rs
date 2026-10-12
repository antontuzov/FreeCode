//! Agent-side settings: the auto-approve policy behind the composer dropdown, privacy defaults,
//! context-compression thresholds and local-engine preferences.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How much side effect a tool call may cause. The sandbox engine keys off this, and the UI renders
/// every call as a transcript step regardless of class — nothing runs silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionClass {
    /// Reads that touch nothing: fs.read, fs.glob, fs.search, index.query.
    ReadOnly,
    /// Writes confined to the workspace: fs.write, fs.edit, diff.stage, artifact.emit.
    WorkspaceWrite,
    /// Anything that leaves the machine or cannot be undone: shell.run, web.fetch, git.commit,
    /// worktree.*, ext.invoke.
    SideEffect,
    /// Network egress beyond the local engine.
    Network,
}

/// Composer dropdown: ▷ Auto approve ▾.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Default: auto for reads and workspace edits, ask for destructive and network calls.
    #[default]
    AutoApprove,
    AskAlways,
    /// The agent may plan and read, but no mutating tool executes.
    PlanOnly,
    /// Per-tool overrides in `tool_overrides`.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Approvals {
    pub mode: ApprovalMode,
    /// Tool id -> explicitly allowed or blocked, used by `ApprovalMode::Custom`.
    pub tool_overrides: BTreeMap<String, bool>,
    /// Per-command allow/deny list for shell.run.
    pub shell_allow: Vec<String>,
    pub shell_deny: Vec<String>,
    pub shell_timeout_ms: u64,
}

impl Default for Approvals {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::AutoApprove,
            tool_overrides: BTreeMap::new(),
            shell_allow: Vec::new(),
            shell_deny: Vec::new(),
            shell_timeout_ms: Self::default_timeout_ms(),
        }
    }
}

impl Approvals {
    pub fn default_timeout_ms() -> u64 {
        120_000
    }

    /// Decide one call without asking the user. Returns `None` when a prompt is required.
    pub fn decide(&self, tool_id: &str, class: PermissionClass) -> Option<bool> {
        if let Some(explicit) = self.tool_overrides.get(tool_id) {
            return Some(*explicit);
        }
        match self.mode {
            ApprovalMode::AskAlways => None,
            ApprovalMode::PlanOnly => Some(matches!(class, PermissionClass::ReadOnly)),
            ApprovalMode::AutoApprove | ApprovalMode::Custom => match class {
                PermissionClass::ReadOnly | PermissionClass::WorkspaceWrite => Some(true),
                PermissionClass::SideEffect | PermissionClass::Network => None,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Privacy {
    /// OFF unless the user turns it on: false is derivable here, and a test asserts it stays false.
    pub telemetry: bool,
    pub allow_remote_providers: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Context {
    /// Fraction of the model's 262144-token window at which older turns are summarized and the
    /// "Context compressed" divider is emitted.
    pub compress_at_ratio: f32,
    /// Recent turns, plus every open file handle, are kept verbatim.
    pub keep_recent_turns: usize,
}

impl Context {
    pub fn compress_threshold(&self, window: u32) -> u32 {
        let ratio = self.compress_at_ratio.clamp(0.1, 0.95) as f64;
        (f64::from(window) * ratio).round() as u32
    }
}

/// Which local pack to load, and where the weights live.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackingChoice {
    /// Detect Metal / CUDA-Ada / CUDA-Hopper-Blackwell / Vulkan / CPU and pick accordingly.
    #[default]
    Auto,
    Forced(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalModel {
    pub catalog_id: String,
    pub packing: PackingChoice,
    pub model_dir: Option<String>,
    /// Name of an environment variable holding a read-only HF token, for gated repos. We store the
    /// variable name, never the secret.
    pub hf_token_env: Option<String>,
    pub offload_all_layers: bool,
    pub flash_attention: bool,
    pub vision_lazy_load: bool,
    /// Single-user server tuning: `-np 1 --cache-ram 24576` keeps the prompt cache from thrashing
    /// across slots (Bonsai-demo#183).
    pub dedicated_server_slots: bool,
}

impl Default for LocalModel {
    fn default() -> Self {
        Self {
            catalog_id: crate::DEFAULT_MODEL_ID.into(),
            packing: PackingChoice::Auto,
            model_dir: None,
            hf_token_env: None,
            offload_all_layers: true,
            flash_attention: true,
            vision_lazy_load: true,
            dedicated_server_slots: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_approve_allows_reads_and_edits_asks_for_side_effects() {
        let a = Approvals::default();
        assert_eq!(a.decide("fs.read", PermissionClass::ReadOnly), Some(true));
        assert_eq!(
            a.decide("fs.edit", PermissionClass::WorkspaceWrite),
            Some(true)
        );
        assert_eq!(a.decide("shell.run", PermissionClass::SideEffect), None);
    }

    #[test]
    fn plan_only_blocks_mutations_and_custom_override_wins() {
        let mut a = Approvals {
            mode: ApprovalMode::PlanOnly,
            ..Default::default()
        };
        assert_eq!(
            a.decide("fs.edit", PermissionClass::WorkspaceWrite),
            Some(false)
        );
        a.mode = ApprovalMode::Custom;
        a.tool_overrides.insert("shell.run".into(), true);
        assert_eq!(
            a.decide("shell.run", PermissionClass::SideEffect),
            Some(true)
        );
    }

    #[test]
    fn telemetry_is_off_by_default() {
        assert!(!Privacy::default().telemetry);
        assert!(!Privacy::default().allow_remote_providers);
    }

    #[test]
    fn compression_threshold_tracks_the_262k_window() {
        let c = Context {
            compress_at_ratio: 0.75,
            keep_recent_turns: 8,
        };
        assert_eq!(c.compress_threshold(262_144), 196_608);
        assert_eq!(
            Context {
                compress_at_ratio: 9.0,
                ..c
            }
            .compress_threshold(1_000),
            950
        );
    }
}
