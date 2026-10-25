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

/** One page the in-app browser is showing: the address the host's `preview` scheme serves, and the
 * project path it came from, which the controls that leave the app hand to the desktop. */
export type PreviewPage = { url: string; path: string };

/**
 * One tab of the in-app browser.
 *
 * The trail is the tab's own, because Back in one tab must not move another: a history shared across
 * tabs would be an arrow that lands somewhere the user was not standing.
 */
export type PreviewTab = {
  /** Where this tab has been. Empty for a tab the plus opened, which has no page to show yet. */
  trail: PreviewPage[];
  /** Where along its own trail the tab stands; `-1` when the trail is empty. */
  cursor: number;
  /** Reload presses. The frame's key carries it, so a reload asks the host again rather than showing
   * the document the webview already rendered. */
  reloads: number;
};

/** The tab the column is showing, or null when the browser is closed. */
export function previewTab(
  state: Pick<NavStore, "previewTabs" | "previewActive">,
): PreviewTab | null {
  return state.previewTabs[state.previewActive] ?? null;
}

/** The page a tab stands on, or null for a tab that has not been given one. */
export function previewPage(tab: PreviewTab | null): PreviewPage | null {
  if (!tab || tab.cursor < 0) return null;
  return tab.trail[tab.cursor] ?? null;
}

/**
 * The project path an address means, or null when it names nothing this panel can show.
 *
 * The field takes the path the review names *and* the `preview://` address the same open control
 * hands the frame, because they are one page said two ways. A web address is neither: the panel reads
 * the project folder, and framing the wider web would be a claim the scheme cannot keep. An empty
 * field is not a page either — Enter on it asks for nothing.
 */
export function previewPathOf(address: string): string | null {
  const raw = address.trim();
  if (!raw) return null;
  const prefix = "preview://localhost/";
  if (raw.startsWith(prefix)) {
    const rest = raw.slice(prefix.length);
    try {
      return decodeURIComponent(rest);
    } catch {
      return rest;
    }
  }
  if (/^(https?|file|ftp):/i.test(raw)) return null;
  return raw;
}

/** A tab replaced in a copy of the list, so a store write never mutates the array it read. */
function withTab(tabs: PreviewTab[], index: number, tab: PreviewTab): PreviewTab[] {
  const next = tabs.slice();
  next[index] = tab;
  return next;
}

const blankTab = (): PreviewTab => ({ trail: [], cursor: -1, reloads: 0 });

function query(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}

/** The deep links the native captures go through (`?settings=model`, `?knowledge=<id>`,
 * `?extensions=1`, `?sidebar=collapsed`, `?review=1`) seed the stack with the conversation underneath
 * them, which is where Esc from that capture would have gone. */
function seed(): {
  history: View[];
  cursor: number;
  collapsed: boolean;
  rightOpen: boolean;
  statusOpen: boolean;
  panel: Panel;
  reviewFile: string | null;
} {
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
    // `?right=hidden` and `?status=hidden` fold the two strips the header's own toggles fold. The
    // native window takes no injected clicks, so a state only a press can reach is a state a capture
    // can never show — the same reason `?sidebar=collapsed` exists.
    rightOpen: query("right") !== "hidden",
    statusOpen: query("status") !== "hidden",
    // `?review=1` puts the change in the right column: the press that opens it is a click on the
    // pill, which the native window cannot be given, so the panel would otherwise never be seen.
    panel: query("review") === "1" ? "review" : "inspector",
    // `?file=<path>` is the same device for the jump an edit row's chip does: the panel opens scrolled
    // to that file, which is the only proof the two columns are wired to each other.
    reviewFile: query("file"),
  };
}

type NavStore = {
  history: View[];
  cursor: number;
  /** The sidebar's column folded away to its rail. */
  collapsed: boolean;
  /** Whether the right column is showing. The header's right-panel toggle folds it, and the transcript
   * takes the width it gives back. */
  rightOpen: boolean;
  /** Whether the strip under the composer is showing. It repeats the engine state the run's own rows
   * carry, which is why it can be put away. */
  statusOpen: boolean;
  /** Which of the two reports the right column is holding. */
  panel: Panel;
  /** The file the review column was asked to show, and which it scrolls to when it does. Cleared by
   * either way out of the column, so a jump is not replayed on a panel the reader already looked at. */
  reviewFile: string | null;
  /** The in-app browser's tabs, in the order their strip draws them. Empty means the column is not
   * showing the browser at all: the tabs are the panel, and the last one closing is the panel
   * closing. Not on the navigation stack — it is opened from the review column and left with one
   * press, the way the palette is. */
  previewTabs: PreviewTab[];
  /** Which tab the column is showing; `-1` when there is none. */
  previewActive: number;
  /** Open a surface over the ones already on the stack. */
  push: (view: View) => void;
  /** Step back along the stack. Reports whether there was anything behind the current view, so the
   * one control that means two things — Esc — can fall through to the other when there was not. */
  back: () => boolean;
  /** Step forward again. Same answer for the same reason: an arrow with nowhere to go is a no-op. */
  forward: () => boolean;
  toggleSidebar: () => void;
  toggleRight: () => void;
  toggleStatus: () => void;
  /** The diff pill and the panel's own close control are the same press seen from either side. */
  toggleReview: () => void;
  showInspector: () => void;
  /** An edit row's file chip: the column switches to the change and opens on that file, because one
   * press that lands on the section is the answer to what was clicked. */
  openReviewAt: (path: string) => void;
  /** Show a page the run built. A copy of it already on the strip is brought forward rather than
   * duplicated; a blank tab takes it; any other tab keeps its page and the new one opens beside it. */
  openPreview: (url: string, path: string) => void;
  /** A fresh tab with no page, which is what the plus means. */
  newPreviewTab: () => void;
  closePreviewTab: (index: number) => void;
  selectPreviewTab: (index: number) => void;
  /** Follow a page inside the active tab, forgetting whatever was ahead of it — the same contract the
   * window's own stack keeps. */
  navigatePreview: (url: string, path: string) => void;
  previewBack: () => void;
  previewForward: () => void;
  reloadPreview: () => void;
  /** The whole panel, back to the review that opened it. */
  closePreview: () => void;
};

export const useNav = create<NavStore>((set, get) => ({
  ...seed(),
  previewTabs: [],
  previewActive: -1,
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
  toggleRight: () => set((s) => ({ rightOpen: !s.rightOpen })),
  toggleStatus: () => set((s) => ({ statusOpen: !s.statusOpen })),
  toggleReview: () => set((s) => ({ panel: s.panel === "review" ? "inspector" : "review" })),
  showInspector: () => set({ panel: "inspector", reviewFile: null }),
  openReviewAt: (path) => set({ panel: "review", reviewFile: path }),
  openPreview: (url, path) =>
    set((s) => {
      // The same page is already on the strip: the press asks to look at it, not to keep a second
      // copy of one document in the column.
      const already = s.previewTabs.findIndex((tab) => previewPage(tab)?.path === path);
      if (already >= 0) return { previewActive: already };
      const page = { url, path };
      const active = previewTab(s);
      // A tab the plus opened is empty on purpose, so it is where the page belongs. A tab already
      // showing a page keeps it, and the new one opens beside it.
      if (active && previewPage(active) === null) {
        return {
          previewTabs: withTab(s.previewTabs, s.previewActive, {
            trail: [page],
            cursor: 0,
            reloads: 0,
          }),
        };
      }
      return {
        previewTabs: [...s.previewTabs, { trail: [page], cursor: 0, reloads: 0 }],
        previewActive: s.previewTabs.length,
      };
    }),
  newPreviewTab: () =>
    set((s) => ({
      previewTabs: [...s.previewTabs, blankTab()],
      previewActive: s.previewTabs.length,
    })),
  closePreviewTab: (index) =>
    set((s) => {
      if (index < 0 || index >= s.previewTabs.length) return {};
      const tabs = s.previewTabs.filter((_, i) => i !== index);
      // The last tab closing is the panel closing: there is nothing left for the column to hold.
      if (tabs.length === 0) return { previewTabs: [], previewActive: -1 };
      // The tab to the right slides into the closed one's place; closing the last tab falls back to
      // the new last one.
      const active =
        s.previewActive > index
          ? s.previewActive - 1
          : s.previewActive === index
            ? Math.min(index, tabs.length - 1)
            : s.previewActive;
      return { previewTabs: tabs, previewActive: active };
    }),
  selectPreviewTab: (index) =>
    set((s) => (index < 0 || index >= s.previewTabs.length ? {} : { previewActive: index })),
  navigatePreview: (url, path) =>
    set((s) => {
      const tab = previewTab(s);
      if (!tab) return {};
      const trail = [...tab.trail.slice(0, tab.cursor + 1), { url, path }];
      return {
        previewTabs: withTab(s.previewTabs, s.previewActive, {
          ...tab,
          trail,
          cursor: trail.length - 1,
        }),
      };
    }),
  previewBack: () => {
    const s = get();
    const tab = previewTab(s);
    if (!tab || tab.cursor <= 0) return;
    set({
      previewTabs: withTab(s.previewTabs, s.previewActive, { ...tab, cursor: tab.cursor - 1 }),
    });
  },
  previewForward: () => {
    const s = get();
    const tab = previewTab(s);
    if (!tab || tab.cursor >= tab.trail.length - 1) return;
    set({
      previewTabs: withTab(s.previewTabs, s.previewActive, { ...tab, cursor: tab.cursor + 1 }),
    });
  },
  reloadPreview: () =>
    set((s) => {
      const tab = previewTab(s);
      if (!tab || previewPage(tab) === null) return {};
      return {
        previewTabs: withTab(s.previewTabs, s.previewActive, { ...tab, reloads: tab.reloads + 1 }),
      };
    }),
  closePreview: () => set({ previewTabs: [], previewActive: -1 }),
}));

export function currentView(s: NavStore): View {
  return s.history[s.cursor] ?? { kind: "chat" };
}
