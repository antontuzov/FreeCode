// The transcript row model. `StepKind` mirrors `smollm_core::transcript::StepKind` one-for-one and
// its i18n keys match Rust's `StepKind::i18n_key`; `verify-transcript-i18n` fails the build if the
// two lists or the catalogs drift, because a row that renders a raw key mid-sentence is a bug the
// user sees in every language.
//
// A transcript is one ordered list of two row shapes: labelled steps (what a tool did) and turns
// (what the user asked and what the model answered). The reference shot interleaves them — the
// prompt bubble, "Processed", the answer paragraph, "Ran 8 tools" — so they share one order rather
// than two lists the UI would have to guess between.
import { create } from "zustand";

import { translate, type Locale } from "../i18n";
import type { AgentEvent } from "../lib/ipc";

export type StepKind =
  | "processed"
  | "toolBatch"
  | "running"
  | "thought"
  | "terminal"
  | "editFile"
  | "readFile"
  | "searchContent"
  | "tool"
  | "contextCompressed"
  | "subagent"
  | "artifact"
  | "webReading"
  | "source"
  | "blocked";

const I18N_KEYS: Record<StepKind, string> = {
  processed: "transcript.step.processed",
  toolBatch: "transcript.step.ranTools",
  running: "transcript.step.running",
  thought: "transcript.step.thought",
  terminal: "transcript.step.terminal",
  editFile: "transcript.step.editFile",
  readFile: "transcript.step.readFile",
  searchContent: "transcript.step.searchContent",
  tool: "transcript.step.ranTool",
  contextCompressed: "transcript.step.contextCompressed",
  subagent: "transcript.step.subagent",
  artifact: "transcript.step.artifact",
  webReading: "transcript.step.webReading",
  source: "transcript.step.source",
  blocked: "transcript.step.blocked",
};

export type StepStatus = "ok" | "running" | "stopped" | "error";

export type DiffStat = { added: number; removed: number };

export type Step = {
  id: string;
  kind: StepKind;
  /** The command, path or snippet the row ran against, truncated by the UI. */
  detail?: string;
  /** Shown as a monospace chip after the label, e.g. the file an edit touched. */
  subject?: string;
  status?: StepStatus;
  diff?: DiffStat;
  /** For `toolBatch` and `running`: the rows nested underneath the header. */
  children?: Step[];
  /** Seconds elapsed, only meaningful on a `running` header. */
  seconds?: number;
  /** For a `blocked` row: the single action that clears the refusal, in the row's own words. */
  cta?: { label: string; url: string };
};

/** One side of the conversation. The reference renders the prompt as a right-aligned bubble and the
 * answer as plain text, so neither is a labelled step row. */
export type Turn = {
  id: string;
  kind: "turn";
  role: "user" | "assistant";
  text: string;
  /**
   * The reasoning that streamed before the answer. Display only: the host replays the answer text
   * into the next request and never this, because re-rendered reasoning makes the fork's prompt
   * cache miss on every turn (Bonsai-demo#183).
   */
  thought?: string;
  /** Deltas are still arriving for this turn. */
  streaming?: boolean;
  /** The refusal badge on an answer that never came: the "Local" badge says what the run was. */
  local?: boolean;
};

export type Row = Step | Turn;

export function isTurn(row: Row): row is Turn {
  return row.kind === "turn";
}

export function stepLabel(locale: Locale, step: Step): string {
  if (step.kind === "toolBatch") {
    return translate(locale, I18N_KEYS.toolBatch, { count: step.children?.length ?? 0 });
  }
  if (step.kind === "running") {
    return translate(locale, I18N_KEYS.running, { seconds: step.seconds ?? 0 });
  }
  return translate(locale, I18N_KEYS[step.kind]);
}

let sequence = 0;
function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${sequence}`;
}

function blockedRow(message: string, cta?: { label: string; url: string }): Step {
  return { id: nextId("blocked"), kind: "blocked", detail: message, status: "error", cta };
}

/** The turn the next delta belongs to: the assistant turn the current run opened. */
function openTurn(rows: Row[]): Turn | null {
  for (let i = rows.length - 1; i >= 0; i -= 1) {
    const row = rows[i];
    if (row && isTurn(row) && row.role === "assistant" && row.streaming) return row;
  }
  return null;
}

/**
 * Apply a delta to the turn the run opened. A delta that arrives with no open turn is dropped
 * rather than invented into a row: the host streams inside one run, so an orphan means the window
 * re-rendered mid-run, and duplicating text would be worse than losing a fragment.
 */
function onOpenTurn(rows: Row[], mutate: (turn: Turn) => Turn): Row[] {
  const turn = openTurn(rows);
  return turn ? rows.map((row) => (row === turn ? mutate(turn) : row)) : rows;
}

function reduce(rows: Row[], event: AgentEvent): Row[] {
  switch (event.type) {
    case "runStarted":
      return [
        ...rows,
        {
          id: event.runId,
          kind: "turn",
          role: "assistant",
          text: "",
          streaming: true,
          local: event.local,
        },
      ];
    case "thoughtDelta":
      return onOpenTurn(rows, (turn) => ({
        ...turn,
        thought: `${turn.thought ?? ""}${event.text}`,
      }));
    case "answerDelta":
      return onOpenTurn(rows, (turn) => ({ ...turn, text: `${turn.text}${event.text}` }));
    case "runFinished":
      return onOpenTurn(rows, (turn) => ({ ...turn, streaming: false }));
    case "engineBlocked":
      return [
        ...rows,
        blockedRow(`${event.info.title} · ${event.info.detail}`, {
          label: event.info.ctaLabel,
          url: event.info.ctaUrl,
        }),
      ];
    case "warning":
      return [...rows, blockedRow(event.message)];
    case "failed": {
      const last = rows[rows.length - 1];
      // The host both emits `failed` and rejects the command with the same text, and a refusal
      // arrives as `engineBlocked` first. One row per problem.
      if (last && !isTurn(last) && last.kind === "blocked" && last.detail === event.message) {
        return rows;
      }
      return [...rows, blockedRow(event.message)];
    }
  }
}

type TranscriptStore = {
  rows: Row[];
  running: boolean;
  /**
   * Percent of the loaded context in use, or null while nothing has measured it. The meter renders
   * an em dash rather than a made-up number, because a real run is now the thing being shown.
   */
  contextUsed: number | null;
  setRows: (rows: Row[]) => void;
  setRunning: (running: boolean) => void;
  /** Only the fixture sets this; a real run has nothing to measure the context with yet. */
  setContextUsed: (percent: number | null) => void;
  /** The prompt's own row, added on send so the bubble appears before the engine answers. */
  pushPrompt: (text: string) => void;
  apply: (event: AgentEvent) => void;
};

export const useTranscript = create<TranscriptStore>((set) => ({
  rows: [],
  running: false,
  contextUsed: null,
  setRows: (rows) => set({ rows }),
  setRunning: (running) => set({ running }),
  setContextUsed: (contextUsed) => set({ contextUsed }),
  pushPrompt: (text) =>
    set((s) => ({ rows: [...s.rows, { id: nextId("turn"), kind: "turn", role: "user", text }] })),
  apply: (event) =>
    set((s) => ({
      rows: reduce(s.rows, event),
      running:
        event.type === "runStarted"
          ? true
          : event.type === "runFinished" || event.type === "failed"
            ? false
            : s.running,
    })),
}));

/**
 * A scripted run used only to render the shell when there is nothing to show yet: reached through
 * `?fixture=1`, never by default. A transcript that invented its own tool results would be worse
 * than an empty one, so the real rows arrive with the agent loop and this stays opt-in.
 */
export const FIXTURE_ROWS: Row[] = [
  { id: "fq", kind: "turn", role: "user", text: "continue" },
  { id: "f1", kind: "processed" },
  {
    id: "f1t",
    kind: "turn",
    role: "assistant",
    text: "Picking up the next phase rather than waiting: wiring the loop, since that is what makes everything so far actually runnable.",
  },
  {
    id: "f2",
    kind: "toolBatch",
    children: [
      { id: "f2a", kind: "thought", detail: "The catalog pins a fork-only engine." },
      { id: "f2b", kind: "readFile", subject: "agent.rs" },
      { id: "f2c", kind: "editFile", subject: "agent.rs", diff: { added: 29, removed: 12 } },
    ],
  },
  { id: "f3", kind: "contextCompressed" },
  {
    id: "f4",
    kind: "running",
    seconds: 10,
    children: [
      { id: "f4a", kind: "terminal", detail: "cargo test -p smollm-core", status: "ok" },
      { id: "f4b", kind: "searchContent", detail: "\\.run\\(" },
      { id: "f4c", kind: "editFile", subject: "lib.rs", status: "running" },
    ],
  },
];
