//! The Extensions page: what the agent can reach, and what this build actually runs.
//!
//! An extensions screen in a cloud app installs add-ons. This app has no add-on format and no store to
//! install from, so the local answer to the same row is the list the agent itself is handed — the tool
//! registry — plus the two facts a user cannot get from anywhere else: which of those calls run without
//! interrupting, and which the sandbox would refuse. Both are computed by the engine that decides them
//! at run time ([`smollm_core::approvals::evaluate`]) against the settings in force, so the page cannot
//! tell a friendlier story than the loop acts. A tool the registry lists but this build has no executor
//! for is said so rather than hidden, because the transcript will show a row for it either way.

use serde::Serialize;
use serde_json::json;
use smollm_config::agent::{ApprovalMode, PermissionClass};
use smollm_config::Settings;
use smollm_core::approvals::{evaluate, ApprovalDecision, Pending};
use smollm_tools::{is_executable, registry, SandboxPolicy, ToolDescriptor};
use std::path::Path;

/// One tool as the page shows it. The class is what the tool can do to the machine; the decision is
/// what the current approval dial says about it. Both travel as the catalog keys the page looks its
/// wording up by, so the sentences live in the locale files like every other label — and the class
/// goes out as `PermissionClass`'s own serde spelling rather than a second copy of it, because the
/// UI's union is written against that spelling and a hand-typed one could drift from it silently.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRow {
    pub id: String,
    pub title: String,
    pub description: String,
    pub permission: PermissionClass,
    pub requires_network: bool,
    /// Whether a call reaches an executor in this build. A tool that is only declared still gets a
    /// transcript row; what it does not get is an outcome.
    pub runnable: bool,
    pub decision: &'static str,
}

/// Everything the page renders, in one read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub tools: Vec<ToolRow>,
    /// How many of them a call can actually run through. Printed beside the count of what is declared
    /// so the gap is the page's headline rather than a footnote.
    pub runnable: usize,
    /// The directory a runnable call may read, as the host resolved it. A path the user needs to be
    /// able to check, which is why it is on the page rather than only in the sandbox.
    pub workspace_root: String,
    /// The dial in force, as the settings document spells it — the same wire spelling the Settings
    /// surface writes, so the page and the control cannot report different modes.
    pub approval_mode: ApprovalMode,
    pub network_allowed: bool,
}

/// What the run would do with a call to this tool right now: run it, stop to ask, or refuse it. An
/// empty argument object is enough, because nothing in the policy reads an argument except the shell
/// command lists — and those only apply to a tool that takes a command.
fn decision_of(
    tool: &ToolDescriptor,
    sandbox: &SandboxPolicy,
    settings: &Settings,
) -> &'static str {
    match evaluate(tool, &json!({}), &settings.approvals, sandbox, "inventory") {
        Pending::Prompt(_) => "ask",
        Pending::Decided(ApprovalDecision::AutoAllowed | ApprovalDecision::UserApproved) => "auto",
        Pending::Decided(_) => "blocked",
    }
}

impl Inventory {
    pub fn build(root: &Path, settings: &Settings) -> Self {
        // The same policy the loop builds for a run, so the page reports the sandbox the next prompt
        // will meet rather than a description of it.
        let sandbox = SandboxPolicy {
            workspace_root: root.to_path_buf(),
            read_outside_workspace: false,
            network_allowed: settings.privacy.allow_remote_providers,
            extra_denied: Vec::new(),
        };
        let tools: Vec<ToolRow> = registry()
            .into_iter()
            .map(|tool| ToolRow {
                id: tool.id.to_string(),
                title: tool.title.to_string(),
                description: tool.description.to_string(),
                permission: tool.permission,
                requires_network: tool.requires_network,
                runnable: is_executable(tool.id),
                decision: decision_of(&tool, &sandbox, settings),
            })
            .collect();
        let runnable = tools.iter().filter(|row| row.runnable).count();
        Inventory {
            tools,
            runnable,
            workspace_root: root.display().to_string(),
            approval_mode: settings.approvals.mode,
            network_allowed: settings.privacy.allow_remote_providers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smollm_config::Settings;

    fn inventory(settings: &Settings) -> Inventory {
        Inventory::build(Path::new("/tmp"), settings)
    }

    fn row<'a>(inv: &'a Inventory, id: &str) -> &'a ToolRow {
        inv.tools.iter().find(|r| r.id == id).expect("tool")
    }

    /// The four reads and the one workspace write are what the Coding control actually offers today, and
    /// the page has to say so: a list that showed all eighteen as live would describe an agent this
    /// build does not have.
    #[test]
    fn the_reads_and_the_workspace_write_are_the_tools_a_call_can_run_through() {
        let inv = inventory(&Settings::default());
        assert_eq!(inv.runnable, 5, "{:?}", inv.tools);
        for id in ["read_file", "list_dir", "glob", "grep_search"] {
            assert!(row(&inv, id).runnable, "{id}");
            assert_eq!(row(&inv, id).decision, "auto", "{id}");
        }
        // The write is offered, and it is auto under the default dial because the sandbox bounds it to
        // the workspace — which is the whole reason it can be.
        assert!(row(&inv, "edit_file").runnable);
        assert_eq!(row(&inv, "edit_file").decision, "auto");
        assert!(!row(&inv, "write_file").runnable);
    }

    /// A destructive call under the default dial asks rather than running, and the page says "ask" —
    /// which is the difference between an agent and a script, so it is not something to smooth over.
    #[test]
    fn a_side_effecting_tool_under_auto_approve_still_has_to_ask() {
        let inv = inventory(&Settings::default());
        assert_eq!(row(&inv, "terminal").decision, "ask");
    }

    /// Plan-only is the dial that turns the agent into a reader. The page has to move with it, or it
    /// is describing a permission the user took away in Settings.
    #[test]
    fn plan_only_blocks_a_mutating_tool_without_the_page_arguing_about_it() {
        let mut settings = Settings::default();
        settings.approvals.mode = ApprovalMode::PlanOnly;
        let inv = inventory(&settings);
        assert_eq!(inv.approval_mode, ApprovalMode::PlanOnly);
        assert_eq!(row(&inv, "write_file").decision, "blocked");
        assert_eq!(row(&inv, "read_file").decision, "auto");
    }

    /// A network tool with no remote provider configured is refused before it is offered, which is
    /// what keeps the "Local" badge a statement rather than an intention.
    #[test]
    fn a_network_tool_is_blocked_while_the_machine_stays_local() {
        let inv = inventory(&Settings::default());
        assert!(!inv.network_allowed);
        let web = inv
            .tools
            .iter()
            .find(|r| r.requires_network)
            .expect("a network-class tool");
        assert_eq!(web.decision, "blocked");
    }

    #[test]
    fn every_declared_tool_is_listed_once() {
        let inv = inventory(&Settings::default());
        assert_eq!(inv.tools.len(), registry().len());
        let mut ids: Vec<String> = inv.tools.iter().map(|r| r.id.clone()).collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }

    /// The page's TypeScript unions are written against these exact spellings, and the compiler on
    /// either side cannot see the other. Serde is the only source of them, so the test is what notices
    /// a rename — a renamed class would otherwise render as a missing label rather than an error.
    #[test]
    fn the_wire_spells_every_field_the_page_reads_it_by() {
        let value = serde_json::to_value(inventory(&Settings::default())).expect("serialisable");
        assert_eq!(value["approvalMode"], "auto_approve");
        assert_eq!(value["networkAllowed"], false);
        assert_eq!(value["workspaceRoot"], "/tmp");
        let classes: Vec<&str> = value["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|t| t["permission"].as_str().expect("a class"))
            .collect();
        for expected in ["read_only", "workspace_write", "side_effect", "network"] {
            assert!(
                classes.contains(&expected),
                "{expected} missing from {classes:?}"
            );
        }
        let first = &value["tools"][0];
        assert!(
            first.get("requiresNetwork").is_some() && first.get("runnable").is_some(),
            "{first}"
        );
    }
}
