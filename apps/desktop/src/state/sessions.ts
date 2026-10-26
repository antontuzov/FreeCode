// The conversations this app has filed, and the one the window is looking at.
//
// The store is a mirror of what the host wrote to disk, not a second copy of the transcript: the
// rows on screen keep streaming in `state/transcript.ts`, and this list only changes when a turn
// finishes or a conversation is reopened. That split is what keeps the sidebar honest — a row it
// shows is a conversation that survived to the file system, and nothing more.
import { create } from "zustand";

import { ipc, type Session, type SessionSummary, type SessionTurn } from "../lib/ipc";
import { isTurn, useTranscript, type Row, type Step } from "./transcript";

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

/** A conversation's filed transcript, re-keyed for this window.
 *
 * The ids a row was drawn under belonged to the window that ran it, and this one numbers its own rows
 * from zero, so a restored `turn-3` and a row the next run mints would arrive as the same key under
 * one list. Prefixing with the conversation's own name is what makes that impossible, and it is the
 * only thing rewritten: a row's content is filed exactly as it was drawn.
 */
function reid(rows: Row[], prefix: string): Row[] {
  return rows.map((row, i) => {
    const id = `${prefix}-${i}`;
    if (isTurn(row)) return { ...row, id };
    return { ...row, id, children: row.children && reidSteps(row.children, `${id}-child`) };
  });
}

/** The same, for the rows folded under a batch header — which are steps, never turns. */
function reidSteps(steps: Step[], prefix: string): Step[] {
  return steps.map((step, i) => {
    const id = `${prefix}-${i}`;
    return step.children
      ? { ...step, id, children: reidSteps(step.children, id) }
      : { ...step, id };
  });
}

/** A conversation's turns as the transcript draws them.
 *
 * A filed conversation comes back with the rows the window held when it wrote it — the tool calls, the
 * edits and their hunks, the notices — because those are the record of what the run did, and a
 * transcript of only the sentences reads as an answer that arrived from nowhere. A conversation filed
 * by an older build has no rows, and its exchange is rebuilt from the turns alone.
 *
 * Either way the pictures come back from `turns`, which is where they are stored: a row names its
 * turn's image by its place in that list rather than carrying a second copy of the bytes.
 */
function asRows(session: Session): Row[] {
  if (session.rows?.length) return withPictures(reid(session.rows, session.id), session.turns);
  return session.turns.map((turn, i) => ({
    id: `${session.id}-${i}`,
    kind: "turn" as const,
    role: turn.role,
    text: turn.text,
    image: turn.image,
  }));
}

/** The rows as the window drew them, minus the fields the conversation already stores elsewhere.
 *
 * A turn row's picture and its reasoning go without: the first is in `turns` and would be written
 * twice, and the second is deliberately never filed at all, because replaying it makes the fork's
 * prompt cache miss on every turn (Bonsai-demo#183).
 */
export function fileable(rows: Row[]): Row[] {
  return rows.map((row) => {
    if (!isTurn(row)) return row;
    const filed = { ...row };
    delete filed.image;
    delete filed.thought;
    return filed;
  });
}

/** Put a filed picture back on the turn row that earned it, walking the turns in order. */
function withPictures(rows: Row[], turns: SessionTurn[]): Row[] {
  let spoken = 0;
  return rows.map((row) => {
    if (!isTurn(row)) return row;
    const turn = turns[spoken];
    // The cursor moves only when the row is the turn it points at. A prompt the host refused never
    // became a turn, and stepping past a row that does not match would hand the next picture to a
    // message that was not given one — so a row that does not match keeps no picture at all.
    if (!turn || turn.role !== row.role || turn.text !== row.text) return row;
    spoken += 1;
    return turn.image ? { ...row, image: turn.image } : row;
  });
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
  /** Rename one, and the row with it. */
  rename: (id: string, title: string) => Promise<void>;
  /** Hold one at the head of the list, or let it fall back into the order. */
  togglePinned: (id: string) => Promise<void>;
  /** Write one out as a markdown document to a path the host's chooser picked. */
  exportTo: (id: string) => Promise<void>;
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
   * Move a filed conversation off the list. The host unlinks nothing: the file goes to the Trash
   * beside the store, where Settings → Storage lists it and offers it back — which is why the row
   * still asks for the press twice, since what the second one costs is the conversation's place in
   * this window's history, not the record itself.
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
  /**
   * Name a conversation what the user calls it. The row is replaced in place rather than re-listed,
   * because a rename is the one change to a conversation that cannot move it: the name is not a field
   * the list sorts by, so the host's answer belongs exactly where the row already sits.
   */
  rename: async (id, title) => {
    set({ error: null });
    try {
      const after = await ipc.renameSession(id, title);
      set((s) => ({ items: s.items.map((item) => (item.id === id ? after : item)) }));
    } catch (error) {
      // The row keeps the name it had, because that is still the name the file holds.
      set({ error: String(error) });
    }
  },
  /**
   * Pin a conversation, or let it go. The whole list is re-read here rather than the one row patched,
   * because this is the change that moves the row and the order it moves into is the host's to decide
   * — a local guess at where a pinned row belongs would be a second sort rule, free to drift from the
   * first.
   */
  togglePinned: async (id) => {
    set({ error: null });
    const was = get().items.find((item) => item.id === id)?.pinned ?? false;
    try {
      await ipc.setSessionPinned(id, !was);
    } catch (error) {
      set({ error: String(error) });
      return;
    }
    await get().refresh();
  },
  /**
   * Write a conversation out as a document. A cancelled chooser is not a failure and the host says so
   * by answering with nothing, so this says nothing either — the row stays as it was and the sidebar
   * keeps quiet, which is what a change of mind should look like.
   */
  exportTo: async (id) => {
    set({ error: null });
    try {
      await ipc.exportSession(id);
    } catch (error) {
      set({ error: String(error) });
    }
  },
}));
