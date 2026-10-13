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
  /** Caveats whose condition the host could not evaluate, so they may not apply to this machine. */
  uncheckedWarnings: string[];
  packs: PackView[];
  /** Set when the load-guard refuses to run: title plus the one action that clears it. */
  blocked: { title: string; detail: string; ctaLabel: string; ctaUrl: string } | null;
};

export type ThemeMode = "light" | "dark" | "system";
export type Accent = "forest" | "mint" | "bee" | "parchment";
export type Language = "en" | "zh";
export type TerminalTheme = "follow_theme" | "light" | "dark" | "custom";
export type FontStyle = "sans" | "serif" | "mono";
export type ContentWidth = "standard" | "wide" | "full";
export type FileIconSet = "qoder" | "seti" | "vscode" | "none";
export type ApprovalMode = "auto_approve" | "ask_always" | "plan_only" | "custom";
export type ReasoningEffort = "low" | "medium" | "xhigh" | "none";
/** `automatic` follows the OS default; `named` pins a specific microphone. */
export type InputDevice = "automatic" | { named: string };
/** `auto` lets the hardware probe choose; `forced` pins a packing key from the catalog. */
export type PackingChoice = "auto" | { forced: string };

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
  language: Language;
  uiFontSize: number;
  codeFontSize: number;
};

/* ---- the whole settings document, as the Settings surface edits it ----
 * Field-for-field the Rust schema in `crates/smollm-config`. The surface mutates the loaded object
 * in place and hands it straight back, so a field no control renders still round-trips untouched. */

export type AppearanceDoc = {
  themeMode: ThemeMode;
  interfaceStyle: Accent;
  glassEffects: boolean;
  terminalTheme: TerminalTheme;
  useBuiltInBrowserForTerminalLinks: boolean;
  language: Language;
  fontStyle: FontStyle;
  contentWidth: ContentWidth;
  fileIcons: FileIconSet;
  uiFontSizePx: number;
  lightCodeTheme: string;
  darkCodeTheme: string;
  showLineNumbers: boolean;
  wrapLongLines: boolean;
  codeFontSizePx: number;
};

export type SamplingProfileDoc = {
  temperature: number;
  topP: number;
  topK: number;
  minP: number;
  presencePenalty: number;
  repetitionPenalty: number;
};

export type SamplingDoc = {
  thinking: SamplingProfileDoc;
  instruct: SamplingProfileDoc;
  effort: ReasoningEffort;
  budget: { maxTokens: number; contextTokens: number };
  systemPrompt: string;
  nonThinkingMode: boolean;
};

export type ShortcutDoc = {
  cmd: boolean;
  shift: boolean;
  alt: boolean;
  ctrl: boolean;
  key: string;
};

export type VoiceDoc = {
  inputDevice: InputDevice;
  voiceInput: boolean;
  recognitionPolish: boolean;
  voiceprintRecognition: boolean;
  sendAutomatically: boolean;
  commonWordCorrections: string[];
  liveVoiceShortcut: ShortcutDoc;
};

export type ApprovalsDoc = {
  mode: ApprovalMode;
  toolOverrides: Record<string, boolean>;
  shellAllow: string[];
  shellDeny: string[];
  shellTimeoutMs: number;
};

export type SettingsDoc = {
  schemaVersion: number;
  appearance: AppearanceDoc;
  sampling: SamplingDoc;
  voice: VoiceDoc;
  approvals: ApprovalsDoc;
  privacy: { telemetry: boolean; allowRemoteProviders: boolean };
  context: { compressAtRatio: number; keepRecentTurns: number };
  localModel: {
    catalogId: string;
    packing: PackingChoice;
    modelDir: string | null;
    hfTokenEnv: string | null;
    offloadAllLayers: boolean;
    flashAttention: boolean;
    visionLazyLoad: boolean;
    dedicatedServerSlots: boolean;
  };
};

export const ipc = {
  modelStatus: () => invoke<ModelStatus>("model_status"),
  loadAppearance: () => invoke<AppearanceState>("load_appearance"),
  /** The host answers with what it stored, including a refreshed `resolvedMode`. */
  saveAppearance: (appearance: AppearanceState) =>
    invoke<AppearanceState>("save_appearance", { appearance }),
  loadSettings: () => invoke<SettingsDoc>("load_settings"),
  /** Validated by deserialising on the host side; the answer is what actually landed on disk. */
  saveSettings: (settings: SettingsDoc) => invoke<SettingsDoc>("save_settings", { settings }),
};
