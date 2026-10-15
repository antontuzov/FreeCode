// The keys the window answers, in one table.
//
// The listener that fires them and the hint that names them both read this file, which is the whole
// point of it: until now a shortcut could be printed before it was wired, and two were — a `⌥V` on
// the composer's mic and a Live Voice chip on the Voice page, over a codebase with no key handler
// anywhere. A row here is a promise the app keeps; a control with no row says so instead of showing
// a glyph.
import { useEffect, useRef } from "react";

import type { ShortcutDoc } from "./ipc";

export type ShortcutId = "newTask" | "settings" | "escape";

/**
 * `cmd` is the platform's primary modifier — ⌘ on macOS, Ctrl on Windows and Linux — because a
 * desktop app that answers only one of them is unusable on the other. `escape` carries no modifier:
 * it is the cancel key, and what it cancels depends on the state the window is in.
 */
export const SHORTCUTS: Record<ShortcutId, ShortcutDoc> = {
  newTask: { cmd: true, shift: false, alt: false, ctrl: false, key: "n" },
  settings: { cmd: true, shift: false, alt: false, ctrl: false, key: "," },
  escape: { cmd: false, shift: false, alt: false, ctrl: false, key: "Escape" },
};

/** The key as the user would have to press it, modifiers first. */
export function shortcutLabel(id: ShortcutId): string {
  const s = SHORTCUTS[id];
  // The shell states the platform when it paints the window, so a Windows build reads `Ctrl+N`
  // rather than a Mac glyph the keyboard there has no key for.
  const platform = document.documentElement.dataset.platform;
  const mac = platform !== "win32" && platform !== "linux";
  const modifiers = [
    s.cmd && (mac ? "⌘" : "Ctrl"),
    s.ctrl && (mac ? "⌃" : "Ctrl"),
    s.alt && (mac ? "⌥" : "Alt"),
    s.shift && (mac ? "⇧" : "Shift"),
  ].filter((m): m is string => Boolean(m));
  const key = s.key === "Escape" ? "Esc" : s.key.toUpperCase();
  return mac ? [...modifiers, key].join("") : [...modifiers, key].join("+");
}

export function matches(event: KeyboardEvent, id: ShortcutId): boolean {
  const s = SHORTCUTS[id];
  if ((event.metaKey || event.ctrlKey) !== s.cmd) return false;
  if (event.altKey !== s.alt || event.shiftKey !== s.shift) return false;
  return event.key.toLowerCase() === s.key.toLowerCase();
}

export type ShortcutHandlers = Partial<Record<ShortcutId, () => void>>;

/**
 * One window-level listener for the whole shell. It is on `window` rather than a focused control
 * because these keys work from anywhere in the window — including while the caret is in the
 * composer, which is where a user most wants ⌘N to answer.
 */
export function useShortcuts(handlers: ShortcutHandlers): void {
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat) return;
      for (const id of Object.keys(latest.current) as ShortcutId[]) {
        if (!matches(event, id)) continue;
        // Both browsers and macOS claim ⌘N and ⌘, for their own menus; a key this app advertises
        // has to be taken from them or the hint is a lie about which window answers.
        event.preventDefault();
        latest.current[id]?.();
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
