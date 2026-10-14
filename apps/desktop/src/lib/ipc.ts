// The only module allowed to talk to Rust. Keeping the command names and payloads in one typed
// place is what stops the UI from inventing a field the host never sends.
import { listen } from "@tauri-apps/api/event";
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
  blocked: {
    title: string;
    detail: string;
    ctaLabel: string;
    ctaUrl: string;
    /** The host can clear this refusal itself by fetching the pinned engine. */
    installable: boolean;
    /** The host can clear this refusal itself by downloading the chosen pack. */
    downloadable: boolean;
  } | null;
};

/**
 * The engine the host fetched and unpacked, and where the launcher must point.
 * Mirrors `smollm_desktop_lib::engine::InstallOutcome`.
 */
export type InstallOutcome = {
  tag: string;
  targetKey: string;
  asset: string;
  sha256: string;
  serverPath: string;
};

/** `smollm_llama::install::Phase`, emitted on the `engine-install` event while an install runs. */
export type EngineInstallProgress =
  | { phase: "downloading"; transferredBytes: number; totalBytes: number }
  | { phase: "unpacking" }
  | { phase: "done"; installed: InstallOutcome & { dir: string } };

/**
 * The pack that ended up on disk. Mirrors `smollm_desktop_lib::weights::Downloaded`.
 * `path` is where the load guard will read the weights from, so it is shown rather than assumed.
 */
export type WeightsDownloaded = {
  catalogId: string;
  packing: string;
  file: string;
  sha256: string;
  path: string;
  sizeBytes: number;
};

/** `smollm_desktop_lib::weights::Phase`, emitted on the `weights-download` event. */
export type WeightsProgress =
  | { phase: "downloading"; transferredBytes: number; totalBytes: number }
  | { phase: "done"; downloaded: WeightsDownloaded };

export type ThemeMode = "light" | "dark" | "system";
export type Accent = "forest" | "mint" | "bee" | "parchment";
export type Language = "en" | "zh";
export type TerminalTheme = "follow_theme" | "light" | "dark" | "custom";
export type FontStyle = "sans" | "serif" | "mono";
export type ContentWidth = "standard" | "wide" | "full";
export type FileIconSet = "qoder" | "seti" | "vscode" | "none";
export type ApprovalMode = "auto_approve" | "ask_always" | "plan_only" | "custom";
export type ReasoningEffort = "auto" | "low" | "medium" | "xhigh" | "none";
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

/** Host event carrying `EngineInstallProgress`; must match `engine::PROGRESS_EVENT` in Rust. */
export const ENGINE_INSTALL_EVENT = "engine-install";

/** Host event carrying `WeightsProgress`; must match `weights::PROGRESS_EVENT` in Rust. */
export const WEIGHTS_DOWNLOAD_EVENT = "weights-download";

/* ---- the live engine and its streamed replies ---- */

/**
 * `smollm_desktop_lib::chat::EngineView`: the supervised `llama-server` as the host reports it.
 * `command` is the argument vector that actually went to the binary. No control shows it — the
 * reference layout has nowhere for a command line — so it is what a failure can quote rather than
 * what the window claims.
 */
export type EngineView = {
  baseUrl: string;
  backend: string;
  engineTag: string;
  packing: string;
  modelFile: string;
  contextTokens: number;
  layersOffloaded: number;
  command: string;
};

export type EngineStatus = { running: boolean; engine: EngineView | null };

/** A load-guard refusal plus the one action that clears it. Mirrors `smollm_core::BlockedInfo`. */
export type BlockedInfo = {
  title: string;
  detail: string;
  ctaLabel: string;
  ctaUrl: string;
};

/**
 * `smollm_core::AgentEvent` on the `agent-event` channel. Only the variants this host can emit are
 * listed: the tool-step variants arrive with the agent loop, and their payloads are Rust
 * `TranscriptStep`s rather than the UI's own row shape, so they get their mapping then.
 */
export type AgentEvent =
  | { type: "runStarted"; runId: string; modelLabel: string; local: boolean }
  | { type: "thoughtDelta"; text: string }
  | { type: "answerDelta"; text: string }
  | { type: "engineBlocked"; info: BlockedInfo }
  | { type: "warning"; code: string; message: string }
  /** `totalDiff` is the "+N −M" pill's running total, which the tool loop is what produces. */
  | { type: "runFinished"; runId: string; stopped: boolean; totalDiff: null }
  | { type: "failed"; message: string };

/** Host event carrying `AgentEvent`; must match `chat::EVENT` in Rust. */
export const AGENT_EVENT = "agent-event";

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
  /** Fetch and unpack the pinned fork engine. Progress arrives on `onEngineInstall`. */
  installEngine: () => invoke<InstallOutcome>("install_engine"),
  onEngineInstall: (onProgress: (progress: EngineInstallProgress) => void): Promise<() => void> =>
    listen<EngineInstallProgress>(ENGINE_INSTALL_EVENT, (event) => onProgress(event.payload)),
  /** Fetch the packing the status strip chose. Progress arrives on `onWeightsDownload`. */
  downloadWeights: () => invoke<WeightsDownloaded>("download_weights"),
  onWeightsDownload: (onProgress: (progress: WeightsProgress) => void): Promise<() => void> =>
    listen<WeightsProgress>(WEIGHTS_DOWNLOAD_EVENT, (event) => onProgress(event.payload)),
  loadAppearance: () => invoke<AppearanceState>("load_appearance"),
  /** The host answers with what it stored, including a refreshed `resolvedMode`. */
  saveAppearance: (appearance: AppearanceState) =>
    invoke<AppearanceState>("save_appearance", { appearance }),
  loadSettings: () => invoke<SettingsDoc>("load_settings"),
  /** Validated by deserialising on the host side; the answer is what actually landed on disk. */
  saveSettings: (settings: SettingsDoc) => invoke<SettingsDoc>("save_settings", { settings }),
  /** Launch the supervised fork server, or refuse with the load guard's own reason. */
  startEngine: () => invoke<EngineView>("start_engine"),
  stopEngine: () => invoke<void>("stop_engine"),
  engineStatus: () => invoke<EngineStatus>("engine_status"),
  /**
   * Send one prompt and stream the answer on `agent-event`. The promise settles when the run is
   * over, with the finished text — so a listener that missed a delta can still close the turn.
   */
  chatSend: (text: string) => invoke<string>("chat_send", { text }),
  onAgentEvent: (onEvent: (event: AgentEvent) => void): Promise<() => void> =>
    listen<AgentEvent>(AGENT_EVENT, (event) => onEvent(event.payload)),
};
