// The live engine: one supervised fork server, and the prompt path that streams an answer from it.
// The transcript owns what a run looks like; this store owns the two facts only the host has —
// whether a server is up, and whether a reply is in flight.
import { create } from "zustand";

import { ipc, type EngineView } from "../lib/ipc";
import { fileable, useSessions } from "./sessions";
import { useTranscript } from "./transcript";

type ChatStore = {
  engine: EngineView | null;
  sending: boolean;
  /** A server start is in flight: the load takes seconds, and a card that shows nothing looks frozen. */
  starting: boolean;
  /** A failure the transcript has no row for: a status poll that could not be answered, a release
   * the host refused. A send's own failure belongs to the transcript, and only to it. */
  error: string | null;
  /** Set once the host has announced anything about the current send. */
  announced: boolean;
  refresh: () => Promise<void>;
  /** A picture is a `data:` URL the composer shrank; a prompt may be nothing but one. Every prompt is
   * an agent prompt: the workspace tools go with it, because this app is an agent rather than a chat
   * window. What a run may *change* is the approval policy's answer, and the shell and the subagents
   * are Settings switches, so the tools offered are the ones the machine has actually agreed to. */
  send: (text: string, image?: string) => Promise<void>;
  stop: () => Promise<void>;
  start: () => Promise<void>;
  release: () => Promise<void>;
  newTask: () => Promise<void>;
};

export const useChat = create<ChatStore>((set, get) => ({
  engine: null,
  sending: false,
  starting: false,
  error: null,
  announced: false,
  refresh: async () => {
    try {
      set({ engine: (await ipc.engineStatus()).engine });
    } catch (error) {
      // A host that cannot answer has no server; the store must not keep claiming one.
      set({ engine: null, error: String(error) });
    }
  },
  send: async (text, image) => {
    const trimmed = text.trim();
    // A picture alone is a question: "what is in this?" needs no words.
    if ((!trimmed && !image) || get().sending) return;
    const transcript = useTranscript.getState();
    // The bubble appears on the click, not on the reply: the engine may be about to spend ten
    // seconds paging the pack into memory, and a window that shows nothing looks frozen.
    transcript.pushPrompt(trimmed, image);
    transcript.setRunning(true);
    set({ sending: true, error: null, announced: false });
    try {
      await ipc.chatSend(trimmed, image, true);
    } catch (error) {
      // The transcript is where a send's failure belongs: the composer repeating it puts the same
      // sentence on screen twice, 200 px apart. A refusal or a dead stream already announced itself on
      // the event channel; a command that refused before starting — a reply in flight, nothing to send
      // — announces nothing, and gets the row pushed here so the click is never silent.
      if (!get().announced)
        useTranscript.getState().apply({ type: "failed", message: String(error) });
    } finally {
      set({ sending: false });
      useTranscript.getState().setRunning(false);
      void get().refresh();
      // The host files each turn as it happens, so the run that just closed is the moment the filed
      // list stops matching the sidebar. Refreshed after the send rather than on an event because the
      // turn is only on disk once the command has returned.
      void useSessions.getState().refresh();
      // And this is the moment the run's own rows can be put back with it. The turns are the host's
      // doing; the steps, hunks and notices only exist in this window, so a conversation reopened
      // tomorrow shows the work it did only if the transcript is handed over now. A send that never
      // reached a conversation has nothing to file under, and the exchange it did reach is already on
      // disk — so a refusal here costs the rows and nothing else, which is what a reopen looked like
      // before this existed.
      void ipc.fileRows(fileable(useTranscript.getState().rows)).catch(() => {});
    }
  },
  stop: async () => {
    try {
      await ipc.chatStop();
    } catch (error) {
      set({ error: String(error) });
    }
    // No engine refresh: the server is the same process it was before the click, and the run's own
    // `runFinished` is what closes the answer out in the transcript.
  },
  /**
   * Put the weights back, from the status card's own control. The host answers with the server it
   * started, so the card needs no second poll to know the address it is about to print.
   */
  start: async () => {
    set({ error: null, starting: true });
    try {
      set({ engine: await ipc.startEngine() });
    } catch (error) {
      // A load the machine refused — no pack, a window too large for the memory behind it — is the
      // card's sentence to say, and the composer is where the card's failures already land.
      set({ error: String(error) });
    } finally {
      set({ starting: false });
    }
  },
  /**
   * Release the weights, from the status card's own control. The next message starts a fresh server,
   * which is the price of the click and the reason this is not what Stop does.
   */
  release: async () => {
    set({ error: null });
    try {
      await ipc.stopEngine();
    } catch (error) {
      set({ error: String(error) });
      return;
    }
    // The window the last run filled is gone with the server that held it, so the meter has nothing
    // left to report.
    useTranscript.getState().setContextUsed(null);
    await get().refresh();
  },
  newTask: async () => {
    set({ error: null });
    try {
      await ipc.newTask();
    } catch (error) {
      // Refused mid-reply. The transcript keeps its rows, because the history the host did not
      // clear is the history those rows were built from.
      set({ error: String(error) });
      return;
    }
    // The host forgot the history, so the rows built from it have to go with it: a transcript that
    // still showed the last answer would claim a memory the engine no longer has.
    const transcript = useTranscript.getState();
    transcript.setRows([]);
    // And the meter has no measurement to carry over: the window the last run filled is not the one
    // this conversation will be answered from.
    transcript.setContextUsed(null);
    // Same for the run report — it describes an answer this transcript no longer holds.
    transcript.setLastRun(null);
    // The pill counts a run's writes, and this window has no run to count now. The files the last one
    // edited stay edited on disk; saying so here would be a claim about the workspace, not about a run.
    transcript.setRunDiff(null);
    // And the step pill counts the rounds of a run that is no longer on screen.
    transcript.setRunRounds(null);
    // The conversation that was on screen is filed, which is the only reason throwing it away is a
    // control rather than a loss. Its row goes back to the list at once, so the user can see where it
    // went, and no row is highlighted because nothing is open any more.
    useSessions.setState({ activeId: null });
    void useSessions.getState().refresh();
  },
}));

/**
 * Wire the host's event channel into the transcript for the lifetime of the window. Called once at
 * module scope in main.tsx, before the React tree exists, so StrictMode's dev double-mount cannot
 * register a second listener — and so an event that arrives while a command is still settling — a
 * relaunch warning, a refusal — is never missed by a listener that already went away.
 */
export function connectAgentEvents(): Promise<() => void> {
  return ipc.onAgentEvent((event) => {
    useChat.setState({ announced: true });
    useTranscript.getState().apply(event);
  });
}
