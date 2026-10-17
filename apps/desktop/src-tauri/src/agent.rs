//! The tool round: run what one answer asked for, and say in the transcript what each call did.
//!
//! This is the half of the agent loop that touches the machine, and it is deliberately the read-only
//! half. [`smollm_tools::EXECUTABLE`] names the four calls that have executors; anything else the
//! model asks for is answered with a notice that says so rather than run, which is why the loop can
//! keep the "Local" badge true and the composer's Coding control can stay an opt-in.
//!
//! Every call takes the same road: [`smollm_core::approvals`] decides, [`smollm_tools::authorize`]
//! resolves its paths against the workspace sandbox inside [`smollm_tools::execute`], and only then
//! does anything read. The row opens before the read and closes after it, so a call that walks a large
//! tree shows up as work in progress rather than as a window that stopped answering.
//!
//! A refusal is said twice, on purpose: the row carries it for the user, and the tool result carries
//! it for the model, which is the only reader that can act on it. A model handed an empty result
//! instead of "that path is protected" tries the same call again, and the run spends a round to learn
//! nothing.

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use smollm_config::Settings;
use smollm_core::approvals::{evaluate, ApprovalDecision, Pending};
use smollm_core::events::{AgentEvent, NoticeCode, ToolOutcome};
use smollm_core::transcript::{tool_batch, StepKind, StepStatus, TranscriptStep};
use smollm_llama::ToolCall;
use smollm_tools::{execute, is_executable, SandboxPolicy};

/// One call's answer to the model, with the id the replay has to name.
#[derive(Debug, Clone)]
pub struct Ran {
    pub call_id: String,
    /// The tool result text. A refusal is in here too, in words the model can act on.
    pub result: String,
}

/// Run one round of calls, emitting the rows as they open and close.
///
/// Results come back in the order the model asked for them: the replay pairs each `tool` message with
/// the `call_id` it answers, and a mismatched pair is what makes the fork's template refuse the next
/// prompt.
pub fn run_batch(
    app: &AppHandle,
    calls: &[ToolCall],
    settings: &Settings,
    policy: &SandboxPolicy,
) -> Vec<Ran> {
    calls
        .iter()
        .map(|call| run_one(app, call, settings, policy))
        .collect()
}

/// The batch header, emitted once a round's calls have all closed. The count comes from the rows the
/// round opened, so a batch never claims a tool it did not run — and a round whose every call was for
/// something this build cannot do opens no rows and so claims no header.
pub fn close_batch(app: &AppHandle, calls: &[ToolCall]) {
    let rows = rows_for(calls);
    if rows == 0 {
        return;
    }
    emit(
        app,
        AgentEvent::ToolBatchClosed {
            step: tool_batch(rows),
        },
    );
}

/// How many of these calls open a row: exactly the ones [`run_one`] will run, which is the same test
/// it applies before emitting anything. A call for a tool with no executor is answered with a notice
/// and never becomes a row, so counting it here would put a number in the header that the batch below
/// it cannot show rows for.
fn rows_for(calls: &[ToolCall]) -> usize {
    calls
        .iter()
        .filter(|call| is_executable(&call.name))
        .count()
}

/// The sandbox every read in this build goes through: the workspace the window is open on, with no
/// reads outside it, and the network gated on the one privacy setting that could make the "Local"
/// badge false. Writes are not this module's to allow, and the policy does not claim to.
pub fn policy_for(app: &AppHandle, settings: &Settings) -> Result<SandboxPolicy, String> {
    Ok(SandboxPolicy {
        workspace_root: crate::paths::workspace_root(Some(app))?,
        // A model that names `/etc/hosts` gets the sandbox's sentence about it, which is a better
        // answer than the file. The workspace is what the task is about.
        read_outside_workspace: false,
        network_allowed: settings.privacy.allow_remote_providers,
        extra_denied: Vec::new(),
    })
}

fn run_one(app: &AppHandle, call: &ToolCall, settings: &Settings, policy: &SandboxPolicy) -> Ran {
    // A call whose arguments never parsed is answered as an empty object: the executor's own
    // "path is required" names the field to fix, which a JSON error does not.
    let arguments: Value = serde_json::from_str(&call.arguments).unwrap_or_else(|_| json!({}));
    let name = call.name.as_str();

    // Not in the offered set — either the registry has no such tool or it has one this build cannot
    // run. Either way no row opens, because a row would promise work that never happens.
    if !is_executable(name) {
        emit(
            app,
            AgentEvent::notice(NoticeCode::ToolNotOffered, &[("name", name)]),
        );
        return Ran {
            call_id: call.id.clone(),
            result: format!(
                "{name} is not available in this build: it reads nothing and changes nothing. The \
                 tools it has are read_file, list_dir, glob and grep_search."
            ),
        };
    }

    let mut row = started_row(name, &arguments);
    emit(app, AgentEvent::ToolStarted { step: row.clone() });

    let outcome = match cleared(name, &arguments, &call.id, settings, policy) {
        Ok(()) => match execute(name, &arguments, policy) {
            Ok(done) => (ToolOutcome::ok(done.summary.clone()), done.content),
            Err(why) => {
                let reason = why.to_string();
                (
                    ToolOutcome::blocked(&reason),
                    format!("{name} did not run: {reason}"),
                )
            }
        },
        Err(reason) => (
            ToolOutcome::blocked(&reason),
            format!("{name} did not run: {reason}"),
        ),
    };
    let (outcome, result) = outcome;
    row.finish(if outcome.ok {
        StepStatus::Done
    } else {
        StepStatus::Blocked
    });
    emit(app, AgentEvent::ToolFinished { step: row, outcome });
    Ran {
        call_id: call.id.clone(),
        result,
    }
}

/// Approval, then the sandbox. `Ok(())` is the only answer that lets a read happen, and everything
/// else comes back as the sentence the row and the model both get.
///
/// A prompt this build cannot show is a refusal rather than a wait: the approval dial is in the
/// composer, and a run hanging on a decision nothing can answer reads as a stalled server.
fn cleared(
    name: &str,
    arguments: &Value,
    call_id: &str,
    settings: &Settings,
    policy: &SandboxPolicy,
) -> Result<(), String> {
    let tool = smollm_tools::find(name).ok_or_else(|| format!("no tool named {name}"))?;
    match evaluate(&tool, arguments, &settings.approvals, policy, call_id) {
        Pending::Decided(decision) if decision.is_allowed() => Ok(()),
        Pending::Decided(ApprovalDecision::UserDenied)
        | Pending::Decided(ApprovalDecision::TimedOut) => Err("the call was not approved".to_string()),
        Pending::Decided(_) => Err(format!(
            "{} is refused by the approval policy in Settings",
            tool.title
        )),
        Pending::Prompt(request) => Err(format!(
            "{} needs an approval this build does not ask for: set the approval control to Auto, or \
             ask without tools",
            request.headline
        )),
    }
}

/// The row's label argument: the path or pattern the call worked on, shown as the chip beside
/// "Read file" rather than buried in a sentence about it.
fn subject(name: &str, arguments: &Value) -> String {
    ["path", "pattern", "file"]
        .iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_str))
        .unwrap_or(name)
        .to_string()
}

fn started_row(name: &str, arguments: &Value) -> TranscriptStep {
    let kind = match name {
        "read_file" => StepKind::ReadFile,
        "grep_search" => StepKind::SearchContent,
        _ => StepKind::Tool,
    };
    let shown = subject(name, arguments);
    let mut row = TranscriptStep::running_tool(name, kind, json!({ "subject": shown }));
    row.detail = shown;
    row
}

fn emit(app: &AppHandle, event: AgentEvent) {
    let _ = app.emit(crate::chat::EVENT, event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_named_by_the_argument_it_acted_on() {
        assert_eq!(
            subject("read_file", &json!({ "path": "src/lib.rs" })),
            "src/lib.rs"
        );
        assert_eq!(subject("grep_search", &json!({ "pattern": "run" })), "run");
        // A call that named neither still gets a label rather than an empty chip.
        assert_eq!(subject("list_dir", &json!({})), "list_dir");
    }

    #[test]
    fn only_the_four_reads_open_a_row() {
        // The executors and the offer have to agree, or a call would open a row nothing closes.
        for id in smollm_tools::EXECUTABLE {
            assert!(is_executable(id), "{id} is offered but not runnable");
            let row = started_row(id, &json!({ "path": "a/b.rs" }));
            assert_eq!(row.tool_id.as_deref(), Some(*id));
            assert_eq!(row.status, StepStatus::Running);
        }
        assert_eq!(smollm_tools::EXECUTABLE.len(), 4);
    }

    #[test]
    fn a_refused_call_closes_its_row_as_blocked() {
        // The row's status is read off the outcome, so a refusal cannot leave a running row up.
        let mut row = started_row("read_file", &json!({ "path": ".env" }));
        row.finish(StepStatus::Blocked);
        assert_eq!(row.status, StepStatus::Blocked);
    }

    #[test]
    fn a_batch_counts_the_rows_it_opened() {
        let called = |names: &[&str]| {
            names
                .iter()
                .map(|name| ToolCall {
                    id: name.to_string(),
                    name: name.to_string(),
                    arguments: "{}".to_string(),
                })
                .collect::<Vec<_>>()
        };
        // A call for a tool with no executor gets a notice, not a row, so the header cannot claim it.
        assert_eq!(rows_for(&called(&["read_file", "grep_search"])), 2);
        assert_eq!(rows_for(&called(&["read_file", "write_file"])), 1);
        assert_eq!(rows_for(&called(&["delete_everything"])), 0);
    }
}
