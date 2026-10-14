// The live engine: one supervised fork server, and the prompt path that streams an answer from it.
// The transcript owns what a run looks like; this store owns the two facts only the host has —
// whether a server is up, and whether a reply is in flight.
import { create } from "zustand";

import { ipc, type EngineView } from "../lib/ipc";
import { useTranscript } from "./transcript";

type ChatStore = {
  engine: EngineView | null;
  sending: boolean;
  /** A failure the transcript has no row for: a status poll that could not be answered, a Stop that
   * refused. A send's own failure belongs to the transcript, and only to it. */
  error: string | null;
  /** Set once the host has announced anything about the current send. */
  announced: boolean;
  refresh: () => Promise<void>;
  send: (text: string) => Promise<void>;
  stop: () => Promise<void>;
};

export const useChat = create<ChatStore>((set, get) => ({
  engine: null,
  sending: false,
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
  send: async (text) => {
    const trimmed = text.trim();
    if (!trimmed || get().sending) return;
    const transcript = useTranscript.getState();
    // The bubble appears on the click, not on the reply: the engine may be about to spend ten
    // seconds paging 5.95 GB of weights in, and a window that shows nothing looks frozen.
    transcript.pushPrompt(trimmed);
    transcript.setRunning(true);
    set({ sending: true, error: null, announced: false });
    try {
      await ipc.chatSend(trimmed);
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
    }
  },
  stop: async () => {
    try {
      await ipc.stopEngine();
    } catch (error) {
      set({ error: String(error) });
    } finally {
      void get().refresh();
    }
  },
}));

/**
 * Wire the host's event channel into the transcript for the lifetime of the window. Subscribed once
 * at mount rather than per send, so an event that arrives while the command is still settling — a
 * relaunch warning, a refusal — is never missed by a listener that already went away.
 */
export function connectAgentEvents(): Promise<() => void> {
  return ipc.onAgentEvent((event) => {
    useChat.setState({ announced: true });
    useTranscript.getState().apply(event);
  });
}
