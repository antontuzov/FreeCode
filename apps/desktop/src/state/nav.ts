// Which surface the window is showing, how it got there, and whether the sidebar is taking its column.
//
// The surfaces used to be three booleans in `App.tsx`, each closed by its own Esc branch, and the
// header's back and forward arrows drew themselves over that and said they were not in this build.
// They are one stack now: the arrows have something to move along, and a surface's own sub-page
// travels with the entry, so Back from a model's page lands on the machine page rather than the
// conversation two presses behind it.
import { create } from "zustand";

export type View =
  | { kind: "chat" }
  | { kind: "settings"; page: string }
  | { kind: "knowledge"; model: string }
  | { kind: "extensions" };

/**
 * What the right-hand column is holding: the run's report, or the change it made. Shot 06 has the
 * review in that column rather than under the answer, because a diff needs a code line's width and the
 * transcript column cannot give one without shrinking the answer it describes.
 */
export type Panel = "inspector" | "review";

/** Two views that paint the same thing are one view, so re-opening the surface you are standing on
 * cannot stack a copy of it behind you. */
function keyOf(view: View): string {
  switch (view.kind) {
    case "settings":
      return `settings:${view.page}`;
    case "knowledge":
      return `knowledge:${view.model}`;
    default:
      return view.kind;
  }
}

/** An entry remembers only which surface to repaint — the conversation's turns live in the transcript
 * store and a settings page's values on disk — so the stack can be capped without losing anything. */
const MAX = 40;

function query(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}

/** The deep links the native captures go through (`?settings=model`, `?knowledge=<id>`,
 * `?extensions=1`, `?sidebar=collapsed`, `?review=1`) seed the stack with the conversation underneath
 * them, which is where Esc from that capture would have gone. */
function seed(): { history: View[]; cursor: number; collapsed: boolean; panel: Panel } {
  const history: View[] = [{ kind: "chat" }];
  const settings = query("settings");
  const knowledge = query("knowledge");
  if (settings !== null) history.push({ kind: "settings", page: settings });
  else if (knowledge !== null)
    history.push({ kind: "knowledge", model: knowledge === "1" ? "" : knowledge });
  else if (query("extensions") !== null) history.push({ kind: "extensions" });
  return {
    history,
    cursor: history.length - 1,
    collapsed: query("sidebar") === "collapsed",
    // `?review=1` puts the change in the right column: the press that opens it is a click on the
    // pill, which the native window cannot be given, so the panel would otherwise never be seen.
    panel: query("review") === "1" ? "review" : "inspector",
  };
}

type NavStore = {
  history: View[];
  cursor: number;
  /** The sidebar's column folded away to its rail. */
  collapsed: boolean;
  /** Which of the two reports the right column is holding. */
  panel: Panel;
  /** Open a surface over the ones already on the stack. */
  push: (view: View) => void;
  /** Step back along the stack. Reports whether there was anything behind the current view, so the
   * one control that means two things — Esc — can fall through to the other when there was not. */
  back: () => boolean;
  /** Step forward again. Same answer for the same reason: an arrow with nowhere to go is a no-op. */
  forward: () => boolean;
  toggleSidebar: () => void;
  /** The diff pill and the panel's own close control are the same press seen from either side. */
  toggleReview: () => void;
  showInspector: () => void;
};

export const useNav = create<NavStore>((set, get) => ({
  ...seed(),
  push: (view) => {
    const { history, cursor } = get();
    const current = history[cursor];
    if (current && keyOf(current) === keyOf(view)) return;
    // Following a new link forgets what was ahead of you, the way a browser does: a stack that kept
    // both branches would need an arrow to say which one it meant.
    const past = history.slice(0, cursor + 1);
    past.push(view);
    const trimmed = past.length > MAX ? past.slice(past.length - MAX) : past;
    set({ history: trimmed, cursor: trimmed.length - 1 });
  },
  back: () => {
    const { cursor } = get();
    if (cursor === 0) return false;
    set({ cursor: cursor - 1 });
    return true;
  },
  forward: () => {
    const { cursor, history } = get();
    if (cursor >= history.length - 1) return false;
    set({ cursor: cursor + 1 });
    return true;
  },
  toggleSidebar: () => set((s) => ({ collapsed: !s.collapsed })),
  toggleReview: () => set((s) => ({ panel: s.panel === "review" ? "inspector" : "review" })),
  showInspector: () => set({ panel: "inspector" }),
}));

export function currentView(s: NavStore): View {
  return s.history[s.cursor] ?? { kind: "chat" };
}
