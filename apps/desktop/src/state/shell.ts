import { create } from "zustand";

import {
  ipc,
  type ShellEvent,
  type ShellInfo,
  type ShellLanguage,
  type ShellRefusal,
} from "../lib/ipc";
import { keepTail, useTerminal } from "./terminal";

/**
 * The panel's own session, as the window sees it.
 *
 * This is a second way to *watch* a command rather than a second way to run one: every line is handed
 * to the host, which admits it through the same approval engine a model's `terminal` call goes
 * through, and only then writes it to the shell or to Python. So the state here is a log and a status,
 * never a command queue — a line that the dial refused leaves no `$` line behind, because the echo
 * comes back from the host after the admission.
 */
export type ShellStatus = "closed" | "opening" | "live" | "ended";

type ShellStore = {
  status: ShellStatus;
  /** What the last start asked for. The tab and the prompt row are drawn from it, including while
   *  the process is still opening and `info` is not here yet. */
  language: ShellLanguage;
  /** The open session, or the last one — `id` is what tells a late event from a dead session apart. */
  info: ShellInfo | null;
  /** The readable terminal log: the lines the user sent, and what came back. */
  text: string;
  /** The session's own exit code, once it ended by itself. */
  exitCode: number | null;
  /** Why the last start or send was refused. Cleared by the next line that is admitted. */
  refusal: ShellRefusal | null;
  /**
   * Open a session, or answer false with `refusal` set. Idempotent while one of the same language is
   * live; a live session of the other language is closed first, since the panel holds one at a time.
   */
  open: (language?: ShellLanguage) => Promise<boolean>;
  /** Send one line. False means it did not run, and `refusal` says which rule said no. */
  send: (line: string) => Promise<boolean>;
  interrupt: () => void;
  /** Kill the session and forget it. The tab's own close. */
  close: () => Promise<void>;
  apply: (event: ShellEvent) => void;
};

const CLOSED = {
  status: "closed" as const,
  language: "shell" as const,
  info: null,
  text: "",
  exitCode: null,
  refusal: null,
};

/**
 * A rejection from `invoke` is the host's `Refusal`, already an object. Anything else — a command name
 * the host never registered, a window without the shell commands — is reported as a start failure
 * rather than swallowed, because a terminal that says nothing is the one failure mode that reads as
 * "the app is broken" instead of "this did not happen".
 */
function refusalOf(error: unknown): ShellRefusal {
  if (typeof error === "object" && error !== null) {
    const code = (error as { code?: unknown }).code;
    if (code === "off" || code === "plan" || code === "closed") return { code };
    if (code === "blocked") {
      const command = (error as { command?: unknown }).command;
      return { code: "blocked", command: typeof command === "string" ? command : "" };
    }
    if (code === "start") {
      const reason = (error as { reason?: unknown }).reason;
      return { code: "start", reason: typeof reason === "string" ? reason : String(error) };
    }
  }
  return { code: "start", reason: String(error) };
}

export const useShell = create<ShellStore>((set, get) => ({
  ...CLOSED,
  open: async (language = "shell") => {
    const state = get();
    if (state.status === "live" && state.language === language) return true;
    // A start already on its way is the same session; a second press must not open a second process.
    if (state.status === "opening") return false;
    if (state.status === "live") {
      // The press that named the other language is the press that ends this one. The log belongs to
      // the session that is going, so it goes with it.
      set({ status: "opening", language, info: null, text: "", exitCode: null, refusal: null });
      await ipc.shellClose().catch(() => {
        // The session is forgotten either way; a kill that failed has nothing left to keep alive.
      });
    } else {
      set({ status: "opening", language, refusal: null });
    }
    try {
      const info = await ipc.shellOpen(language);
      set({
        status: "live",
        language: info.language,
        info,
        text: "",
        exitCode: null,
        refusal: null,
      });
      return true;
    } catch (error) {
      set({ ...CLOSED, language, refusal: refusalOf(error) });
      return false;
    }
  },
  send: async (line) => {
    const state = get();
    const python = state.language === "python";
    // A Python suite's body is written with leading indentation, so a Python line is passed exactly
    // as typed. A shell command is trimmed the way the field always has.
    const command = python ? line.replace(/[\r\n]+$/, "") : line.trim();
    // An empty line is a real Python line — it closes a block — and nothing a shell can run.
    if (!command && !python) return false;
    try {
      await ipc.shellSend(command);
      set({ refusal: null });
      return true;
    } catch (error) {
      const refusal = refusalOf(error);
      set(refusal.code === "closed" ? { refusal, status: "ended" } : { refusal });
      return false;
    }
  },
  interrupt: () => {
    void ipc.shellInterrupt().catch(() => {
      // Nothing to interrupt: an ended shell is a fact the exit event has already reported.
    });
  },
  close: async () => {
    set(CLOSED);
    await ipc.shellClose().catch(() => {
      // The session is dropped either way; a kill that failed has nothing left to keep alive.
    });
  },
  apply: (event) =>
    set((s) => {
      // An event from a session this window has already replaced is not this terminal's output.
      if (!s.info || event.id !== s.info.id) return {};
      switch (event.kind) {
        case "sent": {
          // The echo carries the prompt the field shows, so a Python block reads as Python rather
          // than as a shell that happens to answer with a number.
          const prompt = s.language === "python" ? ">>>" : "$";
          return {
            text: keepTail(
              s.text + (event.command ? `${prompt} ${event.command}\n` : `${prompt}\n`),
            ),
            refusal: null,
          };
        }
        case "chunk":
          return { text: keepTail(s.text + event.text) };
        case "exit":
          return { status: "ended" as const, exitCode: event.code };
      }
    }),
}));

/**
 * Wire the host's shell channel once per window. Called at module scope in `main.tsx` for the same
 * reason the agent channel is: StrictMode's dev double-mount would register a second listener before
 * the first could be removed, and a chunk delivered in that gap would be applied twice.
 */
export async function connectShellEvents(): Promise<void> {
  await ipc.onShellEvent((event) => useShell.getState().apply(event));
}

/**
 * The development trigger behind `?session=python`: the panel's own session is behind a click, and
 * the native window takes no injected clicks, so the link opens the language it names. `?input=` adds
 * the lines to type — one per parameter, in order, so a Python suite can be written as a block. It
 * is dev-only — nothing in the shipped interface sets it.
 */
export async function seedShellSession(): Promise<void> {
  if (!import.meta.env.DEV) return;
  const asked = new URLSearchParams(window.location.search);
  const language = asked.get("session");
  if (language !== "shell" && language !== "python") return;
  useTerminal.getState().showShell();
  await useShell.getState().open(language);
  for (const line of asked.getAll("input")) {
    await useShell.getState().send(line);
  }
}
