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
  root.style.setProperty("--font-ui", `${a.uiFontSize}px`);
  root.style.setProperty("--font-code", `${a.codeFontSize}px`);
  root.dataset.platform = platform();
}

export const useAppearance = create<AppearanceStore>((set, get) => ({
  ...FALLBACK,
  loaded: false,
  load: async () => {
    try {
      const loaded = await ipc.loadAppearance();
      applyToDocument(loaded);
      set({ ...loaded, loaded: true });
    } catch {
      // No host yet (plain `vite dev`): keep the fallback so the shell still renders.
      applyToDocument(get());
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
