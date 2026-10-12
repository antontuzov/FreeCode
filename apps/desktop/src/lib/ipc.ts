// The only module allowed to talk to Rust. Keeping the command names and payloads in one typed
// place is what stops the UI from inventing a field the host never sends.
import { invoke } from "@tauri-apps/api/core";

/** A packing the app can offer, as chosen for this machine by the catalog + hardware probe. */
export type PackView = {
  key: string;
  file: string;
  bitsPerWeight: number;
  sizeGb: number;
  selected: boolean;
  viable: boolean;
  reason: string;
};

export type ModelStatus = {
  catalogId: string;
  displayName: string;
  repoId: string;
  backend: string;
  engineTag: string;
  engineInstalled: boolean;
  weightsPresent: boolean;
  local: boolean;
  packing: string;
  sizeGb: number;
  warnings: string[];
  packs: PackView[];
  /** Set when the load-guard refuses to run: title plus the one action that clears it. */
  blocked: { title: string; detail: string; ctaLabel: string; ctaUrl: string } | null;
};

export type ThemeMode = "light" | "dark" | "system";
export type Accent = "forest" | "mint" | "bee" | "parchment";

/**
 * The shell-facing slice of the Rust `Appearance` document. `themeMode` is the user's choice and
 * may be `system`; `resolvedMode` is always a concrete light/dark and is computed by the host from
 * the window's own theme, so the UI never has to guess what the OS is doing.
 */
export type AppearanceState = {
  themeMode: ThemeMode;
  resolvedMode: Exclude<ThemeMode, "system">;
  accent: Accent;
  glass: boolean;
  uiFontSize: number;
  codeFontSize: number;
};

export const ipc = {
  modelStatus: () => invoke<ModelStatus>("model_status"),
  loadAppearance: () => invoke<AppearanceState>("load_appearance"),
  /** The host answers with what it stored, including a refreshed `resolvedMode`. */
  saveAppearance: (appearance: AppearanceState) =>
    invoke<AppearanceState>("save_appearance", { appearance }),
};
