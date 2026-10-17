// Appearance is the one setting that must be applied before first paint, or the window flashes the
// wrong mode. It loads from the host (the same JSON the Rust settings file owns) and writes back
// through the host, so there is a single source of truth on disk.
import { create } from "zustand";

import { ipc, type AppearanceState } from "../lib/ipc";

const FALLBACK: AppearanceState = {
  themeMode: "dark",
  resolvedMode: "dark",
  accent: "mint",
  glass: true,
  language: "en",
  uiFontSize: 14,
  codeFontSize: 12,
  fontStyle: "sans",
  contentWidth: "standard",
  // Same as the Rust defaults, so a window that has no host yet draws the gutter the reference shows.
  showLineNumbers: true,
  wrapLongLines: false,
  // No pin: a window with no host yet rolls its own face, as it did before there was a picker.
  appIcon: null,
};

type AppearanceStore = AppearanceState & {
  loaded: boolean;
  load: () => Promise<void>;
  set: (patch: Partial<AppearanceState>) => Promise<void>;
};

/** `system` paints whatever the host last resolved for the window, so a live OS switch shows up on
 * the next load without the UI ever reading the OS itself. */
export function effectiveMode(a: AppearanceState): "light" | "dark" {
  return a.themeMode === "system" ? a.resolvedMode : a.themeMode;
}

function platform(): "darwin" | "win32" | "linux" {
  const ua = navigator.userAgent;
  if (ua.includes("Mac")) return "darwin";
  if (ua.includes("Windows")) return "win32";
  return "linux";
}

export function applyToDocument(a: AppearanceState): void {
  const root = document.documentElement;
  root.dataset.themeMode = effectiveMode(a);
  root.dataset.accent = a.accent;
  // Glass degrades to solid where backdrop-filter is unsupported, so the choice is never a
  // half-blurred surface with no background behind it.
  const supportsBlur = CSS.supports("backdrop-filter", "blur(1px)");
  root.dataset.surface = a.glass && supportsBlur ? "glass" : "solid";
  // Both are CSS attribute keys in tokens.css, so the window's typeface and reading column are a
  // token swap rather than a prop threaded through every component that lays out text.
  root.dataset.fontStyle = a.fontStyle;
  root.dataset.contentWidth = a.contentWidth;
  // The two code-display toggles ride the same mechanism. The parser always splits an answer's fences
  // into lines, so a number appearing is an attribute change and not a reason to re-render the
  // transcript — which for a long conversation is the difference between a repaint and a stutter.
  root.dataset.codeLines = a.showLineNumbers ? "on" : "off";
  root.dataset.codeWrap = a.wrapLongLines ? "on" : "off";
  root.style.setProperty("--font-ui", `${a.uiFontSize}px`);
  root.style.setProperty("--font-code", `${a.codeFontSize}px`);
  root.dataset.platform = platform();
}

/** Dev-only URL levers — `?mode=light`, `?accent=bee`, `?font=serif`, `?width=wide`, `?lines=off`,
 * `?wrap=on`, `?glass=off` — honoured only by a dev build.
 * A screenshot comparison needs a state the native window cannot be clicked into, and the alternative
 * was rewriting the settings file the user's own instance reads, which would have flipped their window
 * to get a picture of ours. Production has no such lever: the appearance comes off disk or from
 * nowhere else. An unknown value is no value at all, so a typo paints the stored setting rather than
 * a state that does not exist. */
function devOverrides(): Partial<AppearanceState> {
  if (!import.meta.env.DEV) return {};
  const q = new URLSearchParams(window.location.search);
  const forced: Partial<AppearanceState> = {};
  // A key is set only when the URL named a value this build knows. The overrides merge over the
  // stored state, so an absent key leaves that state alone where an explicit undefined would erase it.
  const mode = q.get("mode");
  if (mode === "light" || mode === "dark" || mode === "system") forced.themeMode = mode;
  const accent = q.get("accent");
  if (accent === "forest" || accent === "mint" || accent === "bee" || accent === "parchment") {
    forced.accent = accent;
  }
  const font = q.get("font");
  if (font === "sans" || font === "serif" || font === "mono") forced.fontStyle = font;
  const width = q.get("width");
  if (width === "standard" || width === "wide" || width === "full") forced.contentWidth = width;
  const lines = q.get("lines");
  if (lines === "on" || lines === "off") forced.showLineNumbers = lines === "on";
  const wrap = q.get("wrap");
  if (wrap === "on" || wrap === "off") forced.wrapLongLines = wrap === "on";
  // The one lever here that is not about ink: glass is a surface treatment, and the difference
  // between the two states is a floating card that either lets the page show through it or does not.
  const glass = q.get("glass");
  if (glass === "on" || glass === "off") forced.glass = glass === "on";
  return forced;
}

export const useAppearance = create<AppearanceStore>((set, get) => ({
  ...FALLBACK,
  loaded: false,
  load: async () => {
    // The overrides paint the window and nothing else: the store keeps what is on disk, so a later
    // settings write cannot persist a mode the user never chose.
    const forced = devOverrides();
    const paint = (a: AppearanceState) => applyToDocument({ ...a, ...forced });
    try {
      const loaded = await ipc.loadAppearance();
      paint(loaded);
      set({ ...loaded, loaded: true });
    } catch {
      // No host yet (plain `vite dev`): keep the fallback so the shell still renders.
      paint(get());
      set({ loaded: true });
    }
  },
  set: async (patch) => {
    const next = { ...get(), ...patch, loaded: true };
    applyToDocument(next);
    set(next);
    try {
      const stored = await ipc.saveAppearance(next);
      applyToDocument(stored);
      set({ ...stored, loaded: true });
    } catch {
      // Persistence is best-effort here; the store already reflects the user's choice.
    }
  },
}));
