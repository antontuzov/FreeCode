//! The transcript data model — what a Coding-view step row *is*, before it becomes pixels.
//!
//! Rows are emitted as an i18n key plus parameters rather than English text, because every one of
//! these labels appears in the reference screenshots and has to render in Russian too: the frontend
//! catalog owns the wording, this crate owns the meaning. [`StepKind::i18n_key`] values are
//! therefore a stable interface — CI asserts the frontend catalog contains each key in
//! [`all_i18n_keys`].
//!
//! The set of rows reproduces the reference shot: the collapsed "Processed" header, the "Ran N
//! tools" batch, a live "Running… Ns" loader line, and the per-tool rows ("Thought", "Read file",
//! "Search content", "Edit file", "Terminal command ran"), plus the "+N −M files changed" pill and
//! the "Context compressed" divider.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The row type. Each variant is one line of the transcript, in the wording the screenshots use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    /// The collapsed header above a finished turn's reasoning block.
    Processed,
    /// "Ran N tools" — a batch of tool rows under one header.
    ToolBatch,
    /// The live loader line, "Running… 3s".
    Running,
    /// "Thought" — a reasoning segment, kept separate from the answer.
    Thought,
    /// "Terminal command ran".
    Terminal,
    /// "Edit file" with a +N −M pill.
    EditFile,
    /// "Create file" — a write that made a file that was not there, so every line is an addition.
    CreateFile,
    /// "Read file".
    ReadFile,
    /// "Search content".
    SearchContent,
    /// "Ran tool" for anything else in the registry.
    Tool,
    /// The "Context compressed" divider emitted when older turns are summarized.
    ContextCompressed,
    /// A subagent delegation row (the inspector lists its children).
    Subagent,
    /// An artifact created during the run.
    Artifact,
    /// "Searched the web" — a lookup against the provider the run was given.
    WebSearch,
    /// "Web reading" — a fetched page the run consulted.
    WebReading,
    /// A citation/source row.
    Source,
    /// A blocker rendered inline, e.g. the load-guard refusing stock llama.cpp.
    Blocked,
}

impl StepKind {
    /// Key into the frontend i18n catalog. Stable: screenshots, tests and CI all match on it.
    pub fn i18n_key(self) -> &'static str {
        match self {
            StepKind::Processed => "transcript.step.processed",
            StepKind::ToolBatch => "transcript.step.ranTools",
            StepKind::Running => "transcript.step.running",
            StepKind::Thought => "transcript.step.thought",
            StepKind::Terminal => "transcript.step.terminal",
            StepKind::EditFile => "transcript.step.editFile",
            StepKind::CreateFile => "transcript.step.createFile",
            StepKind::ReadFile => "transcript.step.readFile",
            StepKind::SearchContent => "transcript.step.searchContent",
            StepKind::Tool => "transcript.step.ranTool",
            StepKind::ContextCompressed => "transcript.step.contextCompressed",
            StepKind::Subagent => "transcript.step.subagent",
            StepKind::Artifact => "transcript.step.artifact",
            StepKind::WebSearch => "transcript.step.webSearch",
            StepKind::WebReading => "transcript.step.webReading",
            StepKind::Source => "transcript.step.source",
            StepKind::Blocked => "transcript.step.blocked",
        }
    }

    /// Whether the row belongs to the reasoning block (collapsible) or the answer body. Reasoning
    /// must never be replayed into the next prompt — re-rendering it is what makes the fork's
    /// prompt cache miss on every turn of a tool loop (Bonsai-demo#183).
    pub fn is_reasoning(self) -> bool {
        matches!(self, StepKind::Processed | StepKind::Thought)
    }

    pub fn is_live_progress(self) -> bool {
        self == StepKind::Running
    }
}

pub fn all_i18n_keys() -> Vec<&'static str> {
    use StepKind::*;
    [
        Processed,
        ToolBatch,
        Running,
        Thought,
        Terminal,
        EditFile,
        CreateFile,
        ReadFile,
        SearchContent,
        Tool,
        ContextCompressed,
        Subagent,
        Artifact,
        WebSearch,
        WebReading,
        Source,
        Blocked,
    ]
    .into_iter()
    .map(|k| k.i18n_key())
    .collect()
}

/// How a row's tool call went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StepStatus {
    Running,
    Done,
    /// Refused by the approval engine or the sandbox; the row shows the reason, not a stack trace.
    Blocked,
    Failed,
}

/// The "+N −M files changed" pill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffStat {
    pub lines_added: u32,
    pub lines_removed: u32,
    pub files_changed: u32,
}

impl DiffStat {
    /// A pill only renders when something actually changed, which is why the acceptance criterion
    /// is a *non-zero* count.
    pub fn is_empty(self) -> bool {
        self.lines_added == 0 && self.lines_removed == 0 && self.files_changed == 0
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            lines_added: self.lines_added + other.lines_added,
            lines_removed: self.lines_removed + other.lines_removed,
            files_changed: self.files_changed.max(other.files_changed),
        }
    }

    /// Signed tokens for the UI: the catalog stores `+{added} −{removed}` with a real minus sign,
    /// matching the screenshot's typography.
    pub fn parts(self) -> (String, String) {
        (
            format!("+{}", self.lines_added),
            format!("−{}", self.lines_removed),
        )
    }
}

/// One block of a change, as the tool that made it saw it: the diff lines, and the file line the
/// first of them sits on.
///
/// The count and the text come from the same write, so a review panel can number its gutter without
/// re-deriving anything — and without the caption the *model's* copy carries, which names the file
/// this row already names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub at: u32,
    pub lines: String,
}

/// One transcript row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptStep {
    pub id: String,
    pub kind: StepKind,
    /// i18n parameters (counts, paths, elapsed seconds) alongside [`StepKind::i18n_key`].
    pub params: serde_json::Value,
    pub status: StepStatus,
    /// Secondary line under the label, already localized by the producer or empty.
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub diff: Option<DiffStat>,
    /// The change a write row made, as diff lines and the line they open on. Only a row that wrote a
    /// file carries one, and its text is what the model was handed rather than a second rendering of
    /// it, so the review the window draws cannot describe a different change than the answer that asked
    /// for it.
    #[serde(default)]
    pub hunk: Option<Hunk>,
    /// Set for tool rows: the registry id whose `title` the UI shows when there is no localized
    /// label.
    #[serde(default)]
    pub tool_id: Option<String>,
    pub collapsed_by_default: bool,
}

impl TranscriptStep {
    pub fn new(kind: StepKind, params: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4().simple().to_string(),
            kind,
            params,
            status: StepStatus::Running,
            detail: String::new(),
            diff: None,
            hunk: None,
            tool_id: None,
            // Reasoning rows and finished batches start collapsed, as in the reference shot.
            collapsed_by_default: matches!(kind, StepKind::Processed | StepKind::ToolBatch),
        }
    }

    pub fn running_tool(tool_id: &str, kind: StepKind, params: serde_json::Value) -> Self {
        Self {
            tool_id: Some(tool_id.to_string()),
            ..Self::new(kind, params)
        }
    }

    pub fn finish(&mut self, status: StepStatus) {
        self.status = status;
    }

    /// The registry title when there is no localized label to show (dev fallback, never localized).
    pub fn fallback_title(&self) -> String {
        self.tool_id
            .as_deref()
            .and_then(smollm_tools::find)
            .map(|t| t.title.to_string())
            .unwrap_or_else(|| self.kind.i18n_key().to_string())
    }
}

/// The "Ran N tools" batch header, computed from the child rows so the count cannot drift.
pub fn tool_batch(count: usize) -> TranscriptStep {
    TranscriptStep::new(StepKind::ToolBatch, serde_json::json!({ "count": count }))
}

/// The live loader line: "Running… 3s".
pub fn running_line(elapsed_seconds: u32) -> TranscriptStep {
    TranscriptStep::new(
        StepKind::Running,
        serde_json::json!({ "seconds": elapsed_seconds }),
    )
}

/// A "Context compressed" divider, carrying how much was summarized so the UI can say so.
pub fn context_compressed(before_tokens: u32, after_tokens: u32) -> TranscriptStep {
    let mut s = TranscriptStep::new(
        StepKind::ContextCompressed,
        serde_json::json!({ "beforeTokens": before_tokens, "afterTokens": after_tokens }),
    );
    s.status = StepStatus::Done;
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_kind_has_a_unique_stable_i18n_key() {
        let keys = all_i18n_keys();
        assert!(keys.len() >= 14, "{keys:?}");
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "i18n keys must be unique");
        assert!(keys.iter().all(|k| k.starts_with("transcript.step.")));
        // The rows the reference screenshots show must exist by name.
        for k in [
            "transcript.step.processed",
            "transcript.step.ranTools",
            "transcript.step.running",
            "transcript.step.thought",
            "transcript.step.editFile",
            "transcript.step.readFile",
            "transcript.step.searchContent",
            "transcript.step.terminal",
            "transcript.step.contextCompressed",
        ] {
            assert!(keys.contains(&k), "missing {k}");
        }
    }

    #[test]
    fn reasoning_rows_collapse_and_are_marked_as_not_replayable() {
        let processed = TranscriptStep::new(StepKind::Processed, serde_json::json!({}));
        assert!(processed.collapsed_by_default);
        assert!(StepKind::Processed.is_reasoning());
        assert!(!StepKind::EditFile.is_reasoning());
        // The answer body and tool rows start expanded so progress is visible.
        let edit = TranscriptStep::new(StepKind::EditFile, serde_json::json!({}));
        assert!(!edit.collapsed_by_default);
    }

    #[test]
    fn the_diff_pill_only_renders_for_real_changes_and_uses_a_true_minus_sign() {
        assert!(DiffStat::default().is_empty());
        let d = DiffStat {
            lines_added: 8,
            lines_removed: 2,
            files_changed: 3,
        };
        assert!(!d.is_empty());
        let (plus, minus) = d.parts();
        assert_eq!(plus, "+8");
        assert_eq!(minus, "−2");
    }

    #[test]
    fn merging_diffs_accumulates_lines_without_double_counting_files() {
        let a = DiffStat {
            lines_added: 5,
            lines_removed: 1,
            files_changed: 2,
        };
        let b = DiffStat {
            lines_added: 3,
            lines_removed: 4,
            files_changed: 1,
        };
        let m = a.merge(b);
        assert_eq!(
            m,
            DiffStat {
                lines_added: 8,
                lines_removed: 5,
                files_changed: 2
            }
        );
    }

    #[test]
    fn batch_and_progress_rows_carry_their_parameters() {
        let batch = tool_batch(4);
        assert_eq!(batch.params["count"], serde_json::json!(4));
        assert_eq!(batch.kind.i18n_key(), "transcript.step.ranTools");
        let live = running_line(3);
        assert_eq!(live.params["seconds"], serde_json::json!(3));
        assert!(live.kind.is_live_progress());
        let divider = context_compressed(180_000, 42_000);
        assert_eq!(divider.status, StepStatus::Done);
        assert_eq!(divider.params["beforeTokens"], serde_json::json!(180_000));
    }

    #[test]
    fn a_tool_row_falls_back_to_the_registry_title() {
        let row =
            TranscriptStep::running_tool("read_file", StepKind::ReadFile, serde_json::json!({}));
        assert_eq!(row.fallback_title(), "Read file");
        // An unknown id must not panic; it degrades to the key so the row still renders.
        let ghost = TranscriptStep::running_tool("nope", StepKind::Tool, serde_json::json!({}));
        assert_eq!(ghost.fallback_title(), "transcript.step.ranTool");
    }

    #[test]
    fn steps_round_trip_through_json_for_the_ipc_boundary() {
        let row = TranscriptStep::running_tool(
            "edit_file",
            StepKind::EditFile,
            serde_json::json!({ "path": "src/app.rs" }),
        );
        let mut row = row;
        row.diff = Some(DiffStat {
            lines_added: 12,
            lines_removed: 4,
            files_changed: 1,
        });
        let raw = serde_json::to_string(&row).expect("serializable");
        let back: TranscriptStep = serde_json::from_str(&raw).expect("deserializable");
        assert_eq!(back, row);
        assert!(raw.contains("linesAdded"), "camelCase on the wire: {raw}");
    }
}
