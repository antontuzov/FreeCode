// The navigation stack is what the header's arrows move along and what Esc falls through. Its
// contracts are the browser's: a new push forgets the branch ahead, reopening the surface you are
// standing on does nothing, and an arrow with nowhere to go is a no-op.
import { beforeEach, describe, expect, it, vi } from "vitest";

import { currentView, useNav } from "./nav";

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

  it("starts on the conversation with the inspector when the URL says nothing", async () => {
    const state = await seeded("");
    expect(currentView(state)).toEqual({ kind: "chat" });
    expect(state.collapsed).toBe(false);
    expect(state.panel).toBe("inspector");
    expect(state.reviewFile).toBeNull();
    expect(state.rightOpen).toBe(true);
    expect(state.statusOpen).toBe(true);
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
