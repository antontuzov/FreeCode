// The transcript row model. `StepKind` mirrors `smollm_core::transcript::StepKind` one-for-one and
// its i18n keys match Rust's `StepKind::i18n_key`; `verify-transcript-i18n` fails the build if the
// two lists or the catalogs drift, because a row that renders a raw key mid-sentence is a bug the
// user sees in every language.
import { create } from "zustand";

import { translate, type Locale } from "../i18n";

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
};

export function stepLabel(locale: Locale, step: Step): string {
  if (step.kind === "toolBatch") {
    return translate(locale, I18N_KEYS.toolBatch, { count: step.children?.length ?? 0 });
  }
  if (step.kind === "running") {
    return translate(locale, I18N_KEYS.running, { seconds: step.seconds ?? 0 });
  }
  return translate(locale, I18N_KEYS[step.kind]);
}

type TranscriptStore = {
  steps: Step[];
  running: boolean;
  setSteps: (steps: Step[]) => void;
  setRunning: (running: boolean) => void;
};

export const useTranscript = create<TranscriptStore>((set) => ({
  steps: [],
  running: false,
  setSteps: (steps) => set({ steps }),
  setRunning: (running) => set({ running }),
}));

/**
 * A scripted run used only to render the shell when there is nothing to show yet: reached through
 * `?fixture=1`, never by default. A transcript that invented its own tool results would be worse
 * than an empty one, so the real rows arrive with the agent loop and this stays opt-in.
 */
export const FIXTURE_STEPS: Step[] = [
  { id: "f1", kind: "processed" },
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
