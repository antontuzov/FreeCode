// The navigation stack is what the header's arrows move along and what Esc falls through. Its
// contracts are the browser's: a new push forgets the branch ahead, reopening the surface you are
// standing on does nothing, and an arrow with nowhere to go is a no-op.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { currentView, previewPage, previewPathOf, previewTab, useNav } from "./nav";

beforeEach(() => {
  window.history.replaceState({}, "", "/");
  useNav.setState(useNav.getInitialState());
});

describe("the stack", () => {
  it("starts on the conversation with nothing behind or ahead of it", () => {
    const state = useNav.getState();
    expect(currentView(state)).toEqual({ kind: "chat" });
    expect(state.history).toHaveLength(1);
    expect(state.cursor).toBe(0);
    expect(state.back()).toBe(false);
    expect(state.forward()).toBe(false);
  });

  it("walks back and forward along what was opened", () => {
    const nav = useNav.getState();
    nav.push({ kind: "settings", page: "appearance" });
    nav.push({ kind: "knowledge", model: "ternary-bonsai-1-7b" });
    expect(currentView(useNav.getState())).toEqual({
      kind: "knowledge",
      model: "ternary-bonsai-1-7b",
    });

    expect(useNav.getState().back()).toBe(true);
    expect(currentView(useNav.getState())).toEqual({ kind: "settings", page: "appearance" });
    expect(useNav.getState().back()).toBe(true);
    expect(currentView(useNav.getState())).toEqual({ kind: "chat" });
    expect(useNav.getState().back()).toBe(false);

    expect(useNav.getState().forward()).toBe(true);
    expect(currentView(useNav.getState())).toEqual({ kind: "settings", page: "appearance" });
    expect(useNav.getState().forward()).toBe(true);
    expect(currentView(useNav.getState())).toEqual({
      kind: "knowledge",
      model: "ternary-bonsai-1-7b",
    });
    expect(useNav.getState().forward()).toBe(false);
  });

  it("does not stack a second copy of the surface already on screen", () => {
    useNav.getState().push({ kind: "chat" });
    useNav.getState().push({ kind: "settings", page: "appearance" });
    useNav.getState().push({ kind: "settings", page: "appearance" });
    expect(useNav.getState().history).toHaveLength(2);

    // A different page of the same surface is a different place, so it is a real push.
    useNav.getState().push({ kind: "settings", page: "model" });
    expect(useNav.getState().history).toHaveLength(3);
  });

  it("forgets the branch ahead when a new link is followed from behind it", () => {
    useNav.getState().push({ kind: "settings", page: "appearance" });
    useNav.getState().back();
    useNav.getState().push({ kind: "knowledge", model: "m" });

    expect(useNav.getState().history).toHaveLength(2);
    expect(currentView(useNav.getState())).toEqual({ kind: "knowledge", model: "m" });
    expect(useNav.getState().forward()).toBe(false);
  });

  it("caps the stack without losing the way back to where the window is", () => {
    for (let i = 0; i < 50; i += 1) useNav.getState().push({ kind: "knowledge", model: `m${i}` });

    const state = useNav.getState();
    expect(state.history).toHaveLength(40);
    expect(state.cursor).toBe(39);
    expect(currentView(state)).toEqual({ kind: "knowledge", model: "m49" });
    expect(state.history[0]).toEqual({ kind: "knowledge", model: "m10" });
  });
});

describe("the column controls", () => {
  it("folds the sidebar and brings it back", () => {
    expect(useNav.getState().collapsed).toBe(false);
    useNav.getState().toggleSidebar();
    expect(useNav.getState().collapsed).toBe(true);
    useNav.getState().toggleSidebar();
    expect(useNav.getState().collapsed).toBe(false);
  });

  it("offers the change in the right column, aimed at the file that asked for it", () => {
    expect(useNav.getState().panel).toBe("inspector");
    useNav.getState().toggleReview();
    expect(useNav.getState().panel).toBe("review");

    useNav.getState().openReviewAt("apps/desktop/src/state/nav.ts");
    expect(useNav.getState().panel).toBe("review");
    expect(useNav.getState().reviewFile).toBe("apps/desktop/src/state/nav.ts");

    useNav.getState().showInspector();
    expect(useNav.getState().panel).toBe("inspector");
    expect(useNav.getState().reviewFile).toBeNull();
  });
});

describe("deep links", () => {
  async function seeded(search: string) {
    window.history.replaceState({}, "", `/${search}`);
    vi.resetModules();
    return (await import("./nav")).useNav.getState();
  }

  it("opens a settings page over the conversation it was captured from", async () => {
    const state = await seeded("?settings=model");
    expect(state.history.map((view) => view.kind)).toEqual(["chat", "settings"]);
    expect(currentView(state)).toEqual({ kind: "settings", page: "model" });
    expect(state.cursor).toBe(1);
  });

  it("opens a model's knowledge page", async () => {
    const state = await seeded("?knowledge=ternary-bonsai-1-7b");
    expect(currentView(state)).toEqual({ kind: "knowledge", model: "ternary-bonsai-1-7b" });
  });

  it("reads ?knowledge=1 as the landing page rather than as a model named 1", async () => {
    const state = await seeded("?knowledge=1");
    expect(currentView(state)).toEqual({ kind: "knowledge", model: "" });
  });

  it("opens the Extensions surface", async () => {
    const state = await seeded("?extensions=1");
    expect(currentView(state)).toEqual({ kind: "extensions" });
  });

  it("lets a settings capture win over the other surfaces in the same URL", async () => {
    const state = await seeded("?settings=appearance&knowledge=ternary-bonsai-1-7b");
    expect(currentView(state)).toEqual({ kind: "settings", page: "appearance" });
    expect(state.history).toHaveLength(2);
  });

  it("carries the collapsed sidebar, the review column and the file it opens on", async () => {
    const state = await seeded("?sidebar=collapsed&review=1&file=apps/desktop/src/App.tsx");
    expect(state.collapsed).toBe(true);
    expect(state.panel).toBe("review");
    expect(state.reviewFile).toBe("apps/desktop/src/App.tsx");
  });

  it("folds the two strips the header's own toggles fold", async () => {
    const state = await seeded("?right=hidden&status=hidden");
    expect(state.rightOpen).toBe(false);
    expect(state.statusOpen).toBe(false);
  });

  it("opens the browser on a page-less tab for a capture", async () => {
    const state = await seeded("?browser=blank");
    expect(state.previewTabs).toHaveLength(1);
    expect(state.previewActive).toBe(0);
    expect(previewPage(previewTab(state))).toBeNull();
  });

  it("starts on the conversation with the inspector when the URL says nothing", async () => {
    const state = await seeded("");
    expect(currentView(state)).toEqual({ kind: "chat" });
    expect(state.collapsed).toBe(false);
    expect(state.panel).toBe("inspector");
    expect(state.reviewFile).toBeNull();
    expect(state.rightOpen).toBe(true);
    expect(state.statusOpen).toBe(true);
    expect(state.previewTabs).toHaveLength(0);
    expect(state.previewActive).toBe(-1);
  });
});

describe("the panel toggles", () => {
  it("fold and reopen each strip without touching the other", () => {
    const nav = useNav.getState();
    nav.toggleRight();
    expect(useNav.getState().rightOpen).toBe(false);
    expect(useNav.getState().statusOpen).toBe(true);
    nav.toggleStatus();
    expect(useNav.getState().statusOpen).toBe(false);
    nav.toggleRight();
    nav.toggleStatus();
    expect(useNav.getState().rightOpen).toBe(true);
    expect(useNav.getState().statusOpen).toBe(true);
  });
});

describe("the in-app browser", () => {
  const open = (name: string) => useNav.getState().openPreview(`preview://localhost/${name}`, name);
  const shown = () => previewPage(previewTab(useNav.getState()))?.path ?? null;
  const trails = () => useNav.getState().previewTabs.map((tab) => previewPage(tab)?.path ?? null);

  it("opens the page the review named, in a tab of its own", () => {
    expect(useNav.getState().previewTabs).toHaveLength(0);
    open("tetris/index.html");
    expect(trails()).toEqual(["tetris/index.html"]);
    expect(shown()).toBe("tetris/index.html");
    // It is not on the navigation stack: the conversation underneath is untouched.
    expect(currentView(useNav.getState())).toEqual({ kind: "chat" });
  });

  it("opens a second page beside the first rather than in place of it", () => {
    open("a.html");
    open("b.html");
    expect(trails()).toEqual(["a.html", "b.html"]);
    expect(shown()).toBe("b.html");
  });

  it("brings a page that is already on the strip forward instead of copying it", () => {
    open("a.html");
    open("b.html");
    open("a.html");
    expect(useNav.getState().previewTabs).toHaveLength(2);
    expect(shown()).toBe("a.html");
  });

  it("gives a blank tab the page it was opened for", () => {
    useNav.getState().newPreviewTab();
    expect(trails()).toEqual([null]);
    open("a.html");
    expect(trails()).toEqual(["a.html"]);
  });

  it("walks a tab's own trail and stays put at either end", () => {
    open("a.html");
    useNav.getState().navigatePreview("preview://localhost/b.html", "b.html");
    useNav.getState().navigatePreview("preview://localhost/c.html", "c.html");
    useNav.getState().previewBack();
    expect(shown()).toBe("b.html");
    useNav.getState().previewBack();
    expect(shown()).toBe("a.html");
    useNav.getState().previewBack();
    expect(shown()).toBe("a.html");
    useNav.getState().previewForward();
    expect(shown()).toBe("b.html");
  });

  it("forgets the pages ahead when it follows a new one from behind", () => {
    open("a.html");
    useNav.getState().navigatePreview("preview://localhost/b.html", "b.html");
    useNav.getState().previewBack();
    useNav.getState().navigatePreview("preview://localhost/c.html", "c.html");
    const tab = previewTab(useNav.getState());
    expect(tab?.trail.map((page) => page.path)).toEqual(["a.html", "c.html"]);
    expect(tab?.cursor).toBe(1);
  });

  it("steps back in one tab without moving another", () => {
    open("a.html");
    useNav.getState().navigatePreview("preview://localhost/b.html", "b.html");
    useNav.getState().navigatePreview("preview://localhost/c.html", "c.html");
    useNav.getState().newPreviewTab();
    open("x.html");
    // The new tab has nowhere behind it, so the arrow is a no-op rather than a jump to another tab.
    useNav.getState().previewBack();
    expect(shown()).toBe("x.html");
    useNav.getState().selectPreviewTab(0);
    expect(shown()).toBe("c.html");
  });

  it("closes one tab and stands on the one that slid into its place", () => {
    open("a.html");
    open("b.html");
    open("c.html");
    useNav.getState().closePreviewTab(1);
    expect(trails()).toEqual(["a.html", "c.html"]);
    expect(shown()).toBe("c.html");
    useNav.getState().closePreviewTab(1);
    expect(shown()).toBe("a.html");
  });

  it("closes the browser when its last tab goes", () => {
    open("a.html");
    useNav.getState().closePreviewTab(0);
    expect(useNav.getState().previewTabs).toHaveLength(0);
    expect(useNav.getState().previewActive).toBe(-1);
  });

  it("closes every tab on one press, which is what Esc does", () => {
    open("a.html");
    open("b.html");
    useNav.getState().closePreview();
    expect(useNav.getState().previewTabs).toHaveLength(0);
    expect(useNav.getState().previewActive).toBe(-1);
  });

  it("reloads the page a tab is standing on, and only that", () => {
    useNav.getState().newPreviewTab();
    useNav.getState().reloadPreview();
    expect(previewTab(useNav.getState())?.reloads).toBe(0);
    open("a.html");
    useNav.getState().reloadPreview();
    expect(previewTab(useNav.getState())?.reloads).toBe(1);
  });
});

describe("the address field", () => {
  it("reads a project path as itself", () => {
    expect(previewPathOf("tetris/index.html")).toBe("tetris/index.html");
    expect(previewPathOf("  game.html  ")).toBe("game.html");
  });

  it("reads the scheme's own address as the path underneath it", () => {
    expect(previewPathOf("preview://localhost/tetris/index.html")).toBe("tetris/index.html");
    expect(previewPathOf("preview://localhost/my%20game.html")).toBe("my game.html");
  });

  it("refuses a web address, which this panel cannot frame", () => {
    expect(previewPathOf("https://example.com")).toBeNull();
    expect(previewPathOf("HTTP://example.com")).toBeNull();
    expect(previewPathOf("file:///etc/passwd")).toBeNull();
    expect(previewPathOf("ftp://example.com")).toBeNull();
  });

  it("reads an empty field as no page at all", () => {
    expect(previewPathOf("")).toBeNull();
    expect(previewPathOf("   ")).toBeNull();
  });
});
