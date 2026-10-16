// The conversations this app has filed, and the one the window is looking at.
//
// The store is a mirror of what the host wrote to disk, not a second copy of the transcript: the
// rows on screen keep streaming in `state/transcript.ts`, and this list only changes when a turn
// finishes or a conversation is reopened. That split is what keeps the sidebar honest — a row it
// shows is a conversation that survived to the file system, and nothing more.
import { create } from "zustand";

import { ipc, type Session, type SessionSummary } from "../lib/ipc";
import { useTranscript, type Row } from "./transcript";

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
};

export const useSessions = create<SessionStore>((set) => ({
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
    transcript.setContextUsed(null);
    transcript.setLastRun(null);
  },
}));
