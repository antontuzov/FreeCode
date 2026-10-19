// The conversations this app has filed, and the one the window is looking at.
//
// The store is a mirror of what the host wrote to disk, not a second copy of the transcript: the
// rows on screen keep streaming in `state/transcript.ts`, and this list only changes when a turn
// finishes or a conversation is reopened. That split is what keeps the sidebar honest — a row it
// shows is a conversation that survived to the file system, and nothing more.
import { create } from "zustand";

import { ipc, type Session, type SessionSummary } from "../lib/ipc";
import { useTranscript, type Row } from "./transcript";

/**
 * Drop every claim the window is making about a run it has stopped watching. The meter measured a
 * context window this conversation does not fill, and the two pills counted the writes and the rounds
 * of an answer this transcript no longer holds. Both are true of the run that happened, not of the
 * conversation now on screen, so neither survives the switch.
 */
function forgetRun() {
  const transcript = useTranscript.getState();
  transcript.setContextUsed(null);
  transcript.setLastRun(null);
  transcript.setRunDiff(null);
  transcript.setRunRounds(null);
}

/** A conversation's turns as the transcript draws them.
 *
 * Only the exchange comes back, in the ids the store hands out: a reopened conversation shows what
 * was said rather than the notices, refusals and timings of the runs that said it, because those
 * were never filed and inventing them now would be a transcript of things that did not happen.
 */
function asRows(session: Session): Row[] {
  return session.turns.map((turn, i) => ({
    id: `${session.id}-${i}`,
    kind: "turn" as const,
    role: turn.role,
    text: turn.text,
    image: turn.image,
  }));
}

type SessionStore = {
  /** Newest first, exactly as the host listed them. */
  items: SessionSummary[];
  /** The conversation on screen, or null while the window is holding one the store has no row for. */
  activeId: string | null;
  error: string | null;
  refresh: () => Promise<void>;
  open: (id: string) => Promise<void>;
  /** Delete one's file, and the row with it. */
  remove: (id: string) => Promise<void>;
};

export const useSessions = create<SessionStore>((set, get) => ({
  items: [],
  activeId: null,
  error: null,
  refresh: async () => {
    try {
      set({ items: await ipc.listSessions(), error: null });
    } catch (error) {
      // A list the host could not read is an empty sidebar, not a stale one: rows that are no longer
      // on disk would open into a "conversation not found" the user has to click through to learn.
      set({ items: [], error: String(error) });
    }
  },
  open: async (id) => {
    set({ error: null });
    let session: Session;
    try {
      session = await ipc.openSession(id);
    } catch (error) {
      // Refused mid-reply, or filed by a build that has since deleted the file. Either way the
      // transcript keeps what it was showing, because the engine's history was not touched.
      set({ error: String(error) });
      return;
    }
    const transcript = useTranscript.getState();
    transcript.setRows(asRows(session));
    set({ activeId: session.id });
    // Both of these measured the conversation that was on screen a moment ago. The window this one
    // fills is unknown until its next run reports, and the last run's rates are not this
    // conversation's rates.
    forgetRun();
  },
  /**
   * Throw a filed conversation away for good. There is no undo and no second copy: the file was the
   * record, which is why the row asks for the press twice before it comes here.
   */
  remove: async (id) => {
    set({ error: null });
    try {
      await ipc.deleteSession(id);
    } catch (error) {
      // Refused mid-reply, or the file would not go. The row stays, because it still describes a
      // conversation that is on disk — deleting it locally would hide a conversation the host kept.
      set({ error: String(error) });
      return;
    }
    // The row goes now rather than at the next list: the file is gone, and a row left standing until
    // something else refreshed would offer to reopen a conversation that cannot be read.
    set((s) => ({ items: s.items.filter((item) => item.id !== id) }));
    if (get().activeId !== id) return;
    // The host forgot the history along with the file, so the rows built from it have to go with it: a
    // transcript still showing that answer would claim a memory the engine no longer has. Unlike New
    // task, nothing is filed to fall back to — this is the row being thrown away.
    useTranscript.getState().setRows([]);
    forgetRun();
    set({ activeId: null });
  },
}));
