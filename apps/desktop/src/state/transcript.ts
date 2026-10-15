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
import type { AgentEvent, RunReport } from "../lib/ipc";

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
  /**
   * For a `blocked` row: what the host calls the problem. It replaces the generic "Blocked" label,
   * because a row headed "Blocked" says nothing about which of the five things went wrong and the
   * sentence under it was written to be read on its own.
   */
  headline?: string;
  /** For a `blocked` row: the evidence — paths, pins, metadata keys — folded away under the headline. */
  technical?: string;
  /**
   * A `blocked` row that reports something the run did anyway — an effort the model substituted, a
   * server that was restarted — rather than one that refuses it. It reads as a notice because a row
   * headed "Blocked" under an answer that streamed fine tells the user the answer is not to be
   * trusted, which is the opposite of what happened.
   */
  notice?: boolean;
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
  /** Cut short by the stop button. The text is what streamed before the click, and saying so is the
   * difference between a truncated answer and an answer the model chose to keep short. */
  stopped?: boolean;
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
  if (step.notice) return translate(locale, "transcript.notice");
  // "Blocked" is the name of the row shape, not of what happened. A refusal says which problem it is.
  if (step.headline) return step.headline;
  return translate(locale, I18N_KEYS[step.kind]);
}

let sequence = 0;
function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${sequence}`;
}

/** The row a refusal gets: a headline the user can act on, and the evidence kept out of the way. */
function blockedRow(refusal: {
  headline?: string;
  detail: string;
  technical?: string;
  cta?: { label: string; url: string };
}): Step {
  return {
    id: nextId("blocked"),
    kind: "blocked",
    headline: refusal.headline,
    detail: refusal.detail,
    technical: refusal.technical,
    status: "error",
    cta: refusal.cta,
  };
}

/**
 * Whether this row already announces that problem. A refusal arrives twice over — once as
 * `engineBlocked`, whose headline and sentence are separate fields, and once as the `failed`
 * message, which the host composes out of the same two — so the row has to be read back the way
 * the host wrote it, or every blocked launch would leave two rows.
 */
function carries(step: Step, message: string): boolean {
  return step.headline ? `${step.headline} — ${step.detail}` === message : step.detail === message;
}

/** Something the run worked around, on the same quiet footing as the answer it did not stop. */
function noticeRow(message: string): Step {
  return { id: nextId("notice"), kind: "blocked", detail: message, status: "error", notice: true };
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
      return onOpenTurn(rows, (turn) => ({
        ...turn,
        streaming: false,
        stopped: event.stopped || undefined,
      }));
    case "engineBlocked":
      return [
        ...rows,
        blockedRow({
          headline: event.info.title,
          detail: event.info.detail,
          technical: event.info.technical,
          cta: { label: event.info.ctaLabel, url: event.info.ctaUrl },
        }),
      ];
    case "warning":
      // Warnings are what the run adapted to, not what it refused: `failed` and `engineBlocked`
      // carry the refusals, and this case takes the substitutions and relaunches that left the
      // answer standing.
      return [...rows, noticeRow(event.message)];
    case "failed": {
      const last = rows[rows.length - 1];
      // The host both emits `failed` and rejects the command with the same text, and a refusal
      // arrives as `engineBlocked` first. One row per problem.
      if (last && !isTurn(last) && last.kind === "blocked" && carries(last, event.message)) {
        return rows;
      }
      return [...rows, blockedRow({ detail: event.message })];
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
  /** What the last finished run cost, or null while none has reported. The inspector says "no run
   * has been timed yet" rather than showing rates for a run that never sent its timings frame. */
  lastRun: RunReport | null;
  setRows: (rows: Row[]) => void;
  setRunning: (running: boolean) => void;
  /** Presets the meter for the fixture, and clears it when the conversation is thrown away. */
  setContextUsed: (percent: number | null) => void;
  /** Presets the run report for the fixture; a real run fills it from its own timings. */
  setLastRun: (run: RunReport | null) => void;
  /** The prompt's own row, added on send so the bubble appears before the engine answers. */
  pushPrompt: (text: string) => void;
  apply: (event: AgentEvent) => void;
};

export const useTranscript = create<TranscriptStore>((set) => ({
  rows: [],
  running: false,
  contextUsed: null,
  lastRun: null,
  setRows: (rows) => set({ rows }),
  setRunning: (running) => set({ running }),
  setContextUsed: (contextUsed) => set({ contextUsed }),
  setLastRun: (lastRun) => set({ lastRun }),
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
      // The server counted the tokens; this is the one place that learns them. A run that reported
      // no usage leaves the last measured figure standing rather than blanking a meter that was
      // already telling the truth about the same window.
      contextUsed:
        event.type === "runFinished" ? (event.contextUsed ?? s.contextUsed) : s.contextUsed,
      // Unlike the meter, the report is about one run rather than about the window, so a run that
      // never timed itself replaces the last one's numbers instead of leaving them up.
      lastRun: event.type === "runFinished" ? event.run : s.lastRun,
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
    // Markdown and LaTeX on purpose: this is also the row that proves the answer parser runs on
    // real model output rather than on a paragraph that happened to need no formatting. The link is
    // there for the same reason — an answer's address is a control now, not a URL to read out.
    text: "Picking up the next phase rather than waiting: wiring the loop, since that is what makes everything so far actually runnable.\n\nThe budget per turn is $O(n^2)$ in the replayed history, so the cache does most of the work:\n\n```rust\nlet ratio = cached as f32 / prompt as f32;\n```\n\nThe build that reads these weights is [the PrismML fork](https://github.com/PrismML-Eng/llama.cpp).",
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
  // A refusal, word for word as the load guard writes it, so the card can be looked at without
  // uninstalling the engine on a machine that already has one. It is the row's shape that is on
  // trial here — the copy comes from `Blocker::detail`, and this is the only place it is repeated.
  {
    id: "f5",
    kind: "blocked",
    status: "error",
    headline: "The engine isn't on this machine yet",
    detail:
      "SmoLLM Studio runs models with one specific build of llama.cpp, and this machine doesn't " +
      "have it yet. Installing it fetches exactly the build this app was pinned to — nothing else " +
      "is chosen for you.",
    technical:
      "engine prism-b10772-e8fd209 is not installed (expected " +
      "~/Library/Application Support/studio.smollm.desktop/engines/llama-server from " +
      "llama-server-aarch64-apple-darwin-metal.tar.gz)",
    cta: {
      label: "Install the PrismML engine",
      url: "https://github.com/PrismML-Eng/llama.cpp/releases/tag/prism-b10772-e8fd209",
    },
  },
];
