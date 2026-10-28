// The binding table is the one place a printed key comes from, and ⌥E is the reason `matches` reads the
// physical key: on macOS that chord's `key` is "Dead", so a comparison against `event.key` alone would
// advertise a shortcut the window never answers.
import { describe, expect, it } from "vitest";

import { SHORTCUTS, matches, shortcutLabel, type ShortcutId } from "./shortcuts";

function key(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent("keydown", init);
}

describe("the keys the palette's commands print", () => {
  it("builds each label from the table", () => {
    expect(shortcutLabel("files")).toBe("⌘⌥E");
    expect(shortcutLabel("browser")).toBe("⌘T");
    expect(shortcutLabel("review")).toBe("⌘⇧G");
  });

  it("answers ⌥E even though macOS calls that event's key Dead", () => {
    expect(matches(key({ key: "Dead", code: "KeyE", metaKey: true, altKey: true }), "files")).toBe(
      true,
    );
  });

  it("does not answer ⌥E without the option key", () => {
    expect(matches(key({ key: "e", code: "KeyE", metaKey: true }), "files")).toBe(false);
  });

  it("keeps ⇧ apart: ⌘G is not the review key", () => {
    expect(matches(key({ key: "g", code: "KeyG", metaKey: true }), "review")).toBe(false);
    expect(matches(key({ key: "G", code: "KeyG", metaKey: true, shiftKey: true }), "review")).toBe(
      true,
    );
  });

  it("takes ⌘T for the browser and not a bare T from the composer", () => {
    expect(matches(key({ key: "t", code: "KeyT", metaKey: true }), "browser")).toBe(true);
    expect(matches(key({ key: "t", code: "KeyT" }), "browser")).toBe(false);
  });

  it("holds a binding for every id it declares", () => {
    const ids: ShortcutId[] = [
      "newTask",
      "palette",
      "settings",
      "terminal",
      "files",
      "browser",
      "review",
      "escape",
    ];
    expect(Object.keys(SHORTCUTS).sort()).toEqual([...ids].sort());
  });
});
