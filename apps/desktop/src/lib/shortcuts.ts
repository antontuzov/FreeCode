// The keys the window answers, in one table.
//
// The listener that fires them and the hint that names them both read this file, which is the whole
// point of it: until now a shortcut could be printed before it was wired, and two were — a `⌥V` on
// the composer's mic and a Live Voice chip on the Voice page, over a codebase with no key handler
// anywhere. A row here is a promise the app keeps; a control with no row says so instead of showing
// a glyph.
import { useEffect, useRef } from "react";

import type { ShortcutDoc } from "./ipc";

export type ShortcutId = "newTask" | "palette" | "settings" | "terminal" | "escape";

/**
 * `cmd` is the platform's primary modifier — ⌘ on macOS, Ctrl on Windows and Linux — because a
 * desktop app that answers only one of them is unusable on the other. `escape` carries no modifier:
 * it is the cancel key, and what it cancels depends on the state the window is in.
 */
export const SHORTCUTS: Record<ShortcutId, ShortcutDoc> = {
  newTask: { cmd: true, shift: false, alt: false, ctrl: false, key: "n" },
  // The palette is a search over what is already there, so it opens over the shell rather than
  // replacing it, and the same key closes it — a user who reaches ⌘K twice changed their mind.
  palette: { cmd: true, shift: false, alt: false, ctrl: false, key: "k" },
  settings: { cmd: true, shift: false, alt: false, ctrl: false, key: "," },
  // The reference's bottom panel key. ⇧ keeps it apart from the browser's own search keys, and the
  // panel it answers moves neither the conversation nor the right column.
  terminal: { cmd: true, shift: true, alt: false, ctrl: false, key: "j" },
  escape: { cmd: false, shift: false, alt: false, ctrl: false, key: "Escape" },
};

/** Whether this window's primary modifier is the Mac key, as the shell stated it when it painted. */
function primaryIsCommand(): boolean {
  const platform = document.documentElement.dataset.platform;
  return platform !== "win32" && platform !== "linux";
}

/**
 * The binding as its user would press it, one entry per key, modifiers first.
 *
 * Every spelling of a key this app prints comes through here — the chip row a control shows and the
 * single string a tooltip carries are built from the same list. They used to be two rules, and the
 * chip rule had no platform in it, so a Windows build read `⌘` for a key its keyboard has no key for.
 */
export function shortcutChips(shortcut: ShortcutDoc): string[] {
  const mac = primaryIsCommand();
  return [
    shortcut.cmd && (mac ? "⌘" : "Ctrl"),
    shortcut.ctrl && (mac ? "⌃" : "Ctrl"),
    shortcut.alt && (mac ? "⌥" : "Alt"),
    shortcut.shift && (mac ? "⇧" : "Shift"),
    shortcut.key === "Escape" ? "Esc" : shortcut.key.toUpperCase(),
  ].filter((chip): chip is string => Boolean(chip));
}

/** The whole key as one string: `⌘K` where the primary modifier is ⌘, `Ctrl+K` where it is not. */
export function shortcutLabel(id: ShortcutId): string {
  return shortcutChips(SHORTCUTS[id]).join(primaryIsCommand() ? "" : "+");
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
