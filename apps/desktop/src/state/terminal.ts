import { create } from "zustand";

import type { AgentEvent, HostStepKind } from "../lib/ipc";

/** The rows that are a command running, whose output belongs in a terminal. */
export const COMMAND_KINDS: readonly HostStepKind[] = ["terminal", "tests", "build", "linter"];

export function isCommand(kind: HostStepKind): boolean {
  return COMMAND_KINDS.includes(kind);
}

/** The tab label a runner wears when the host named no subject for it: a command is its own name,
 *  and a project check is the thing it checked. */
export const RUNNER_LABEL_KEY: Partial<Record<HostStepKind, string>> = {
  tests: "transcript.step.tests",
  build: "transcript.step.build",
  linter: "transcript.step.linter",
};

/** One command the run started, as the panel keeps it: the command, the output so far, how it ended. */
export type TerminalRun = {
  /** The transcript row's id. The panel is keyed by it so a row and its tab are the same call. */
  id: string;
  kind: HostStepKind;
  /** The command as the host worded it, or empty for a runner the manifest chose without a subject. */
  label: string;
  output: string;
  status: "running" | "ok" | "error";
  startedAt: number;
};

/** How many finished commands the panel keeps. Twenty is a working session's worth of checks and a
 *  tab strip that still fits beside the column; older ones scroll out of the strip rather than the
 *  memory of the window. */
export const MAX_RUNS = 20;

/** How much output one run keeps. The host stops reporting at the same bound, so this is the second
 *  guard for a chunk that arrives twice across a reconnected window. The tail is kept: a build's
 *  errors are at its end. */
export const MAX_OUTPUT = 256 * 1024;

export function keepTail(text: string): string {
  if (text.length <= MAX_OUTPUT) return text;
  let cut = text.length - MAX_OUTPUT;
  // Do not split a surrogate pair: half a character renders as a replacement mark for good.
  if (cut > 0 && (text.charCodeAt(cut) & 0xfc00) === 0xdc00) cut += 1;
  return text.slice(cut);
}

/** Whether the window opened with the panel already showing — the dev deep link, since the native
 *  window takes no injected clicks and a closed panel is a surface no capture can reach. */
function seededOpen(): boolean {
  if (typeof window === "undefined") return false;
  const asked = new URLSearchParams(window.location.search).get("terminal");
  return asked === "1" || asked === "show";
}

/**
 * The id of the tab that is the panel's own shell. It is not a run and has no transcript row: the
 * shell outlives every command in the strip, so it gets a stable id of its own rather than one the
 * host mints.
 */
export const SHELL_TAB = "shell";

type TerminalStore = {
  /** The commands of this window, in the order they started. */
  runs: TerminalRun[];
  /** Whether the strip holds the shell tab. The shell itself lives in `state/shell.ts`. */
  shell: boolean;
  /** The tab the body is showing. */
  active: string | null;
  /** Whether the panel is in the window at all. A command opening closes it, a press closes it. */
  open: boolean;
  /** Whether the body is folded away, leaving the tab strip in place. */
  collapsed: boolean;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  setCollapsed: (collapsed: boolean) => void;
  /** Show one run — the transcript row's own press. A run this window no longer holds (a conversation
   *  reopened from disk) leaves the panel as it was rather than showing a tab that is not there. */
  focus: (id: string) => void;
  /** Drop one run from the strip — the tab's own close, for a command whose output the reader is
   *  done with. The neighbour becomes the tab, the way a browser's strip behaves. */
  dismiss: (id: string) => void;
  /** Put the shell tab in the strip and show it. The PTY itself is opened by `state/shell.ts`. */
  showShell: () => void;
  /** Take the shell tab out. The session is killed by `state/shell.ts`, which the press also calls. */
  hideShell: () => void;
  /** Empty the panel. The runs are the panel; a command still running keeps its tab, because a blank
   *  strip over a command that is still printing would say it had ended. */
  clear: () => void;
  apply: (event: AgentEvent) => void;
  /** Dev only: put scripted runs in for `?fixture=1`, the way `FIXTURE_ROWS` scripts the transcript. */
  seed: (runs: TerminalRun[]) => void;
};

/** The state the tab-strip rules need, so `closedTab` can be read on its own. */
type StripState = Pick<TerminalStore, "runs" | "shell" | "active" | "open">;

/**
 * The shell tab leaving the strip. It is the first tab, so the neighbour that takes its place is the
 * first run — and with no runs left the strip is empty, which is the panel closing. Killing the shell
 * is not this function's job: the tab's press calls the session's own close, and the panel only draws
 * what is left.
 */
function closedTab(s: StripState): Partial<TerminalStore> {
  const active = s.active === SHELL_TAB ? (s.runs[0]?.id ?? null) : s.active;
  return s.runs.length === 0 ? { shell: false, active, open: false } : { shell: false, active };
}

export const useTerminal = create<TerminalStore>((set) => ({
  runs: [],
  shell: false,
  active: null,
  open: seededOpen(),
  collapsed: false,
  setOpen: (open) => set({ open }),
  toggle: () => set((s) => ({ open: !s.open })),
  setCollapsed: (collapsed) => set({ collapsed }),
  focus: (id) =>
    set((s) =>
      s.runs.some((run) => run.id === id) || (id === SHELL_TAB && s.shell)
        ? { active: id, open: true, collapsed: false }
        : {},
    ),
  dismiss: (id) =>
    set((s) => {
      if (id === SHELL_TAB) return closedTab(s);
      const at = s.runs.findIndex((run) => run.id === id);
      if (at < 0) return {};
      const runs = s.runs.filter((run) => run.id !== id);
      // The tab that takes the closed one's place is the next one, or the previous when the last tab
      // was the one closed — the strip shrinks by one rather than jumping to its other end.
      const active =
        s.active === id
          ? (runs[Math.min(at, runs.length - 1)]?.id ?? (s.shell ? SHELL_TAB : null))
          : s.active;
      // The strip is the panel: a press that closed the last tab meant the panel, not an empty body
      // below a strip with nothing left to switch to.
      return runs.length === 0 && !s.shell ? { runs, active, open: false } : { runs, active };
    }),
  showShell: () => set({ shell: true, active: SHELL_TAB, open: true, collapsed: false }),
  hideShell: () => set((s) => closedTab(s)),
  clear: () =>
    set((s) => {
      const running = s.runs.filter((run) => run.status === "running");
      const keepShell = s.shell && s.active === SHELL_TAB;
      return {
        runs: running,
        active: keepShell
          ? SHELL_TAB
          : running.some((run) => run.id === s.active)
            ? s.active
            : (running[running.length - 1]?.id ?? (s.shell ? SHELL_TAB : null)),
      };
    }),
  seed: (runs) => set({ runs: runs.slice(-MAX_RUNS), active: runs[runs.length - 1]?.id ?? null }),
  apply: (event) =>
    set((s) => {
      switch (event.type) {
        case "toolStarted": {
          const step = event.step;
          if (!isCommand(step.kind) || s.runs.some((run) => run.id === step.id)) return {};
          const run: TerminalRun = {
            id: step.id,
            kind: step.kind,
            label: step.detail,
            output: "",
            status: "running",
            startedAt: Date.now(),
          };
          // A command is a thing to watch, so the panel comes up on it: a close press meant "not now",
          // and the next command is the next thing the user would want to see. A folded body stays
          // folded — the strip already shows the new tab.
          return {
            runs: [...s.runs, run].slice(-MAX_RUNS),
            active: step.id,
            open: true,
          };
        }
        case "terminalOutput": {
          return {
            runs: s.runs.map((run) =>
              run.id === event.stepId
                ? { ...run, output: keepTail(run.output + event.chunk) }
                : run,
            ),
          };
        }
        case "toolFinished": {
          const step = event.step;
          if (!s.runs.some((run) => run.id === step.id)) return {};
          return {
            runs: s.runs.map((run) =>
              run.id === step.id ? { ...run, status: event.outcome.ok ? "ok" : "error" } : run,
            ),
          };
        }
        default:
          return {};
      }
    }),
}));

/** A scripted run for `?fixture=1`: the shape a check leaves, so the panel's chrome and its tab
 *  strip are surfaces a native capture can reach. It is dev-only and never says a command ran that
 *  did not — nothing reads it without the fixture link. */
export const FIXTURE_RUNS: TerminalRun[] = [
  {
    id: "f4a",
    kind: "tests",
    label: "",
    status: "ok",
    startedAt: 0,
    output: [
      "$ pnpm test",
      "",
      " RUN  v3.2.7 /Users/dev/project",
      "",
      " ✓ src/state/transcript.test.ts (8 tests) 41ms",
      " ✓ src/state/sessions.test.ts (6 tests) 18ms",
      " ✓ src/components/Sidebar.test.ts (3 tests) 12ms",
      "",
      " Test Files  3 passed (3)",
      "      Tests  17 passed (17)",
      "   Duration  312ms",
      "",
    ].join("\n"),
  },
];
