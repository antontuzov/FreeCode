// The only module allowed to talk to Rust. Keeping the command names and payloads in one typed
// place is what stops the UI from inventing a field the host never sends.
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
// Type-only, which is what keeps the cycle it closes (the transcript store is built from the events
// declared down here) in the type graph rather than in the bundle.
import type { Row } from "../state/transcript";

/**
 * Which of the mascot tiles this launch wears, one-based, or `null` when nothing set it.
 *
 * The host decides before the page's own scripts run and hands the number over as a global, because a
 * number the UI had to `invoke` for would put one duck in the window and another in the Dock for a
 * moment, and the two are the same picture on purpose. Bounds belong to whoever has the tiles: this
 * reports the number, it does not vouch for a count it cannot see.
 */
export const hostMascot: number | null = (() => {
  const injected = (window as unknown as { freecodeMascot?: unknown }).freecodeMascot;
  return typeof injected === "number" && Number.isInteger(injected) && injected >= 1
    ? injected
    : null;
})();

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
  /** Set when Remote is the selected source; the local engine rows are then not what answers. */
  remote: {
    baseUrl: string;
    model: string;
    contextTokens: number;
    /** The Privacy consent switch. Off means a remote turn is refused, not rerouted. */
    allowed: boolean;
    /** Whether the address and model are present. A turn also needs consent and a saved key. */
    configured: boolean;
  } | null;
  /** Set when a free model is the selected source; the local engine rows are then not what answers. */
  free: {
    id: string;
    label: string;
    provider: string;
    /** False for the promotion rows, which the gateway route refuses rather than routing. */
    routable: boolean;
    /** The Privacy consent switch. Off means a free turn is refused, not rerouted. */
    allowed: boolean;
  } | null;
  /**
   * The OpenCode app's local service, which answers every free row without a gateway key. `available`
   * is a probe of this machine, not a setting: the picker's rows go live when it is true.
   */
  opencode: { available: boolean; version: string | null };
  packing: string;
  sizeGb: number;
  warnings: string[];
  /** Caveats whose condition the host could not evaluate, so they may not apply to this machine. */
  uncheckedWarnings: string[];
  packs: PackView[];
  /** Set when the load-guard refuses to run: the refusal's code plus the one action that clears it. */
  blocked:
    | (BlockedInfo & {
        /** The host can clear this refusal itself by fetching the pinned engine. */
        installable: boolean;
        /** The host can clear this refusal itself by downloading the chosen pack. */
        downloadable: boolean;
      })
    | null;
};

/**
 * One free-model registry entry, as the picker draws it. Mirrors
 * `freecode_desktop_lib::free_models::FreeModelInfo`. `routable` is about the gateway key: a row whose
 * model OpenRouter does not list cannot be sent there. `local` is the other route — the OpenCode app
 * on this Mac, which answers the whole list — so a row can be unpickable for the key and pickable
 * for the app in the same breath.
 */
export type FreeModelInfo = {
  id: string;
  label: string;
  provider: string;
  isFree: boolean;
  routable: boolean;
  local: boolean;
};

/** The OpenCode app's local service, as the host probes it. Mirrors `opencode::Status`. */
export type OpenCodeStatus = {
  available: boolean;
  version: string | null;
};

/**
 * The engine the host fetched and unpacked, and where the launcher must point.
 * Mirrors `freecode_desktop_lib::engine::InstallOutcome`.
 */
export type InstallOutcome = {
  tag: string;
  targetKey: string;
  asset: string;
  sha256: string;
  serverPath: string;
};

/** `freecode_llama::install::Phase`, emitted on the `engine-install` event while an install runs. */
export type EngineInstallProgress =
  | { phase: "downloading"; transferredBytes: number; totalBytes: number }
  | { phase: "unpacking" }
  | { phase: "done"; installed: InstallOutcome & { dir: string } };

/**
 * The pack that ended up on disk. Mirrors `freecode_desktop_lib::weights::Downloaded`.
 * `path` is where the load guard will read the weights from, so it is shown rather than assumed.
 */
export type WeightsDownloaded = {
  catalogId: string;
  packing: string;
  file: string;
  sha256: string;
  path: string;
  sizeBytes: number;
  /**
   * The vision projector that came down with this pack, by name. Absent when the model publishes none
   * and when this machine already had it — a picture cannot be answered without the file, so a press
   * that fetched it says so instead of leaving the user to work out which of the two the button did.
   */
  projector?: string;
};

/** `freecode_desktop_lib::weights::Phase`, emitted on the `weights-download` event. */
export type WeightsProgress =
  | { phase: "downloading"; transferredBytes: number; totalBytes: number }
  | { phase: "done"; downloaded: WeightsDownloaded };

export type ThemeMode = "light" | "dark" | "system";
/** The interface styles, in the order the Appearance page cards them. One list rather than four:
 * the picker, the `?accent=` override, the preview swatches and the Rust enum all have to agree, and
 * a style the page offers that the store refuses to save is the bug that list-by-hand produces. */
export const ACCENTS = [
  "forest",
  "mint",
  "bee",
  "parchment",
  "rain",
  "ink",
  "rose",
  "slate",
] as const;
export type Accent = (typeof ACCENTS)[number];
/** The same eight, with the name printed on each one's card. It lives beside `ACCENTS` rather than in
 * the Appearance page because the footer's Theme submenu lists them too, and a menu that named a
 * style the page does not sell — or sells one it cannot name — is the second list-by-hand bug. */
export const INTERFACE_STYLES: { id: Accent; label: string }[] = [
  { id: "forest", label: "Forest" },
  { id: "mint", label: "Mint" },
  { id: "bee", label: "Bee" },
  { id: "parchment", label: "Parchment" },
  { id: "rain", label: "Rain" },
  { id: "ink", label: "Ink" },
  { id: "rose", label: "Rose" },
  { id: "slate", label: "Slate" },
];
export type Language = "en" | "ru";
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
  fontStyle: FontStyle;
  contentWidth: ContentWidth;
  /** Both govern the answer's fenced code, which the shell renders, so they belong to this slice
   * rather than only to the document the Settings surface edits. */
  showLineNumbers: boolean;
  wrapLongLines: boolean;
  /** Which of the mascot tiles the app wears, one-based, or null to roll one each launch.
   * The host owns the Dock tile, so the pin lives on the wire rather than only in the page. */
  appIcon: number | null;
};

/** Host event carrying `EngineInstallProgress`; must match `engine::PROGRESS_EVENT` in Rust. */
export const ENGINE_INSTALL_EVENT = "engine-install";

/** Host event carrying `WeightsProgress`; must match `weights::PROGRESS_EVENT` in Rust. */
export const WEIGHTS_DOWNLOAD_EVENT = "weights-download";

/* ---- the live engine and its streamed replies ---- */

/**
 * `freecode_desktop_lib::chat::EngineView`: the supervised `llama-server` as the host reports it.
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

/**
 * A load-guard refusal plus the one action that clears it. Mirrors `freecode_core::BlockedInfo`.
 *
 * `code` is `freecode_llama::load_guard::BlockerCode` in its kebab-case wire spelling, and it names two
 * sentences in both locale catalogs — `blocker.<code>.title` and `blocker.<code>.detail` — with
 * `params` filling the fields they interpolate. `ctaCode` names the button's own key, `cta.<code>`.
 * The host carries no copy of any of the three, which is what lets the same refusal read in the
 * interface's language; `verify-transcript-i18n` fails the build when the two sides drift.
 * `technical` stays the host's text because a path, a pin or a metadata key is a measurement.
 */
export type BlockedInfo = {
  code: string;
  params: Record<string, string>;
  /** Paths, pins and metadata keys behind the refusal, folded under the sentence that asks for them. */
  technical: string;
  ctaCode: string;
  ctaUrl: string;
};

/**
 * What one finished run cost. Mirrors `freecode_core::events::RunReport`. The rates and counts are the
 * server's own — its clock over the tokens it counted — and they are null for a stream that never got
 * to send them, while `elapsedMs` is the host's wall clock and always stands: the wait shown is at
 * least as long as the model time, and it is the half of the report a stopped run still has.
 */
export type RunReport = {
  generatedTokens: number | null;
  promptTokens: number | null;
  cachedTokens: number | null;
  generatedPerSecond: number | null;
  promptPerSecond: number | null;
  elapsedMs: number;
};

/**
 * `freecode_core::NoticeCode`: what a run worked around rather than refused. The wire spelling is the
 * Rust enum's kebab-case and each code names a `notice.<code>` sentence in both locale catalogs, with
 * `params` filling the fields that sentence interpolates — the host measures, the catalog owns the
 * wording. `verify-transcript-i18n` fails the build when a code has no key or the two sides name the
 * fields differently, which is why the renderer can look the sentence up without a fallback.
 */
export type NoticeParams = Record<string, string>;

/**
 * `freecode_core::transcript::StepKind` in its camelCase wire spelling. The set is the UI's own row
 * vocabulary, so a kind the host adds is a type error here until the transcript and both locale
 * catalogs have it — which is the point of declaring it twice.
 */
export type HostStepKind =
  | "processed"
  | "toolBatch"
  | "running"
  | "thought"
  | "terminal"
  | "tests"
  | "build"
  | "linter"
  | "git"
  | "editFile"
  | "createFile"
  | "readFile"
  | "searchContent"
  | "todo"
  | "tool"
  | "contextCompressed"
  | "subagent"
  | "artifact"
  | "webSearch"
  | "webReading"
  | "source"
  | "blocked";

/** `freecode_core::transcript::StepStatus`. */
export type HostStepStatus = "running" | "done" | "blocked" | "failed";

/**
 * `freecode_core::transcript::DiffStat`: what a write cost, counted by the host. On a row it is that
 * call's own file, so `filesChanged` is 1; on `diffStaged` and `runFinished` it is the run's ledger,
 * where `filesChanged` counts *distinct* paths — a run that edits one file four times says one file,
 * not four.
 */
export type HostDiff = {
  linesAdded: number;
  linesRemoved: number;
  filesChanged: number;
};

/**
 * `freecode_core::transcript::Hunk`: one block of a change, as the tool that made it saw it. `at` is the
 * line in the file before the write that `lines` opens on, 1-based, which is what lets the review
 * panel number a diff gutter without re-deriving the position from the file it no longer matches.
 */
export type HostHunk = { at: number; lines: string };

/**
 * `freecode_core::transcript::EditedFile`: one file a call changed, with the lines the call cost it
 * and each block it wrote there. A row sends a list of these when one hunk cannot describe the call
 * — several files, or one file changed in several places.
 */
export type HostEditedFile = { path: string; added: number; removed: number; hunks: HostHunk[] };

/**
 * `freecode_tools::PlanItem`: one step of the list a run keeps, with the state the model last gave it.
 * `active` is the step being worked on; the tool refuses a plan with more than one.
 */
export type HostPlanItem = { text: string; status: "pending" | "active" | "done" };

/**
 * `freecode_core::transcript::TranscriptStep`: one row as the host declares it.
 *
 * The host owns which row exists and what it measured; the catalog owns the words around it. `params`
 * carries the numbers a label interpolates, and `detail` the path or pattern the call acted on, which
 * the transcript shows as the row's chip rather than inside a sentence. On an edit row `detail` is the
 * written file as the sandbox resolved it — workspace-relative, so two calls that named one file land
 * on one row of the ledger.
 */
export type HostStep = {
  id: string;
  kind: HostStepKind;
  params: Record<string, string | number>;
  status: HostStepStatus;
  detail: string;
  /** The lines this call cost, or null for a row that changed nothing. */
  diff: HostDiff | null;
  /**
   * `freecode_core::transcript::TranscriptStep::hunk`: the change as diff lines, on an `edit_file` row
   * and on no other. It is the text the model was handed, not a second rendering of it, so the review
   * a window draws cannot describe a different change than the answer that asked for it.
   */
  hunk: HostHunk | null;
  /**
   * `freecode_core::transcript::TranscriptStep::files`: every file the call changed, when there is
   * more than one block to report. `diff` is their total and `detail` is empty when no single path
   * can stand in the chip. Absent on a single-hunk row, which keeps the `hunk` shape.
   */
  files?: HostEditedFile[];
  /**
   * `freecode_tools::PlanItem`: the steps a `todo_write` row recorded, in the order the model wrote
   * them. Present only on that row, so the plan panel reads the latest non-empty list rather than
   * keeping a second copy of state that could drift from the transcript.
   */
  plan?: HostPlanItem[];
  /** The registry id, which is what tells a generic "Ran tool" row which tool it ran. */
  toolId: string | null;
  collapsedByDefault: boolean;
};

/**
 * `freecode_core::events::ToolOutcome`. The fields a call that reads or edits can answer with are here;
 * the rest of the Rust struct — an exit code, artifacts — belongs to a build that runs shells and
 * produces files, and this one does neither.
 *
 * `summaryKey` names the catalog sentence the row shows, and `summary` is that sentence in English —
 * which is what the model was handed inside the tool result, and what a line with no key (a page's own
 * title) falls back to. `verify-transcript-i18n` is what keeps a key that arrives from being one the
 * catalogs do not hold.
 */
export type HostToolOutcome = {
  ok: boolean;
  summary: string;
  summaryKey?: string | null;
  summaryParams?: Record<string, string | number> | null;
  diff: HostDiff | null;
};

/**
 * `freecode_core::AgentEvent` on the `agent-event` channel. Only the variants this host can emit are
 * listed. A tool step arrives as the Rust `TranscriptStep` the host built for it rather than as the
 * UI's own row shape, because which row exists and what it measured is the host's answer; how it reads
 * is the catalog's, which is why the transcript maps one onto the other instead of trusting the wire.
 */
export type AgentEvent =
  /** `maxRounds` is the loop's own budget, and 0 for a run that was never offered a tool — the step
   * pill's denominator, and the reason it can say "do not draw me" without copying a constant.
   * `plan` is the dial's Plan-only mode: the run explored and proposed, and could not write. */
  | {
      type: "runStarted";
      runId: string;
      modelLabel: string;
      local: boolean;
      /** The supervised local engine answers this run. A remote provider and the OpenCode service run
       * on this machine too, so `local` is true for them as well; this is the flag that says a server
       * has to be started before the first token. */
      engine: boolean;
      maxRounds: number;
      plan: boolean;
    }
  | { type: "thoughtDelta"; text: string }
  | { type: "answerDelta"; text: string }
  /** A tool row opens while the call runs, and closes with the outcome under it. The batch header
   * arrives last, counting the rows the round opened rather than the calls the model made. */
  | { type: "toolStarted"; step: HostStep }
  | { type: "toolFinished"; step: HostStep; outcome: HostToolOutcome }
  | { type: "toolBatchClosed"; step: HostStep }
  /** The run's ledger after one more write landed on it, so the "+N −M" pill moves while the run is
   * still going rather than arriving only once it ends. */
  | { type: "diffStaged"; diff: HostDiff }
  | { type: "engineBlocked"; info: BlockedInfo }
  | { type: "warning"; code: string; params: NoticeParams }
  /** `totalDiff` is the "+N −M" pill's closing total: null when the run wrote nothing, which is the
   * same answer as "there is nothing to review".
   * `contextUsed` is the percent of the window the finished run left filled, counted by the server;
   * null when the stream never reported its usage frame. `run` is what the run cost: always there,
   * since the host timed the wait itself, with the engine's numbers missing from a stream the user
   * stopped before its closing frame. */
  | {
      type: "runFinished";
      runId: string;
      stopped: boolean;
      contextUsed: number | null;
      totalDiff: HostDiff | null;
      run: RunReport;
    }
  | { type: "failed"; message: string };

/** Host event carrying `AgentEvent`; must match `chat::EVENT` in Rust. */
export const AGENT_EVENT = "agent-event";

/* ---- the whole settings document, as the Settings surface edits it ----
 * Field-for-field the Rust schema in `crates/freecode-config`. The surface mutates the loaded object
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
  /** One-based index into the mascot tiles; null lets each launch roll its own. Same value as
   * `AppearanceState.appIcon`, which the shell reads for the window's own face. */
  appIcon: number | null;
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
  /** Off by default: the one switch that lets the loop run a command, and the consent it stands on. */
  shellEnabled: boolean;
  /** Off by default: consent for one read-only second model run per prompt. */
  subagentsEnabled: boolean;
  shellAllow: string[];
  shellDeny: string[];
  shellTimeoutMs: number;
};

/**
 * Where prompts are answered: this machine's supervised engine, or an OpenAI-compatible provider the
 * user configured. No key is here — the key lives in the macOS Keychain and this document never holds
 * it. `allowRemoteProviders` under Privacy is the consent that lets the Remote source be used at all.
 */
export type RemoteProviderDoc = {
  source: "local" | "remote";
  baseUrl: string;
  model: string;
  contextTokens: number;
};

export type SettingsDoc = {
  schemaVersion: number;
  appearance: AppearanceDoc;
  sampling: SamplingDoc;
  voice: VoiceDoc;
  approvals: ApprovalsDoc;
  privacy: { telemetry: boolean; allowRemoteProviders: boolean };
  remoteProvider: RemoteProviderDoc;
  /** The free-model picker's choice, by registry id. `null` means no free model is selected, and
   * the answer order is free first, then the remote provider, then the local engine. */
  freeModel: string | null;
  context: { compressAtRatio: number; keepRecentTurns: number };
  /** The folder a run works in. Empty means the root the host resolved; anything else must be an
   * absolute path the host can open, or the run is refused by name. */
  project: { root: string };
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

/**
 * One side of an exchange, as the conversation store filed it. Mirrors `sessions::Turn`.
 *
 * `image` is a `data:<mime>;base64,…` URL, kept so a reopened conversation shows the picture the
 * answer was given to rather than a prompt about a photo that is no longer there.
 */
export type SessionTurn = {
  role: "user" | "assistant";
  text: string;
  image?: string;
};

/**
 * A filed conversation. Mirrors `sessions::Session`.
 *
 * The turns are the prompt/answer pairs the history replays. `rows` is the rest of what the window
 * drew — the run's tool calls, its edits and their hunks, the notices and the refusals — handed back
 * by the transcript at the end of a turn, so reopening a conversation shows the work and not only the
 * words. A turn row's picture is not duplicated in it: the bytes stay in `turns`, and the row is
 * joined back to them by its place in the list. The reasoning stream is still never filed, because
 * it is never replayed (Bonsai-demo#183).
 *
 * Absent on a conversation an older build filed, whose transcript is rebuilt from the turns alone.
 */
export type Session = {
  schemaVersion: number;
  id: string;
  /** The folder the conversation happened in, which is the heading the sidebar groups rows under. */
  workspace: string;
  /** The first prompt, shortened. The user's own words, not a generated summary. */
  title: string;
  /** The model that answered the newest turn. */
  model: string;
  updatedAtMs: number;
  turns: SessionTurn[];
  rows?: Row[];
};

/** What a sidebar row needs, without every answer's text crossing the boundary to get it. */
export type SessionSummary = Omit<Session, "schemaVersion" | "turns" | "rows"> & {
  turnCount: number;
};

/**
 * A summary plus the line the search matched, flattened into one object by the host. Mirrors
 * `sessions::Hit`.
 *
 * `excerpt` is a conversation's own words rather than a second title: it is what tells the user why
 * this result is theirs, and `role` says whose of the two it was found in — finding a phrase you
 * typed and finding one the model produced are different things to have found.
 */
export type SessionHit = SessionSummary & {
  role: "user" | "assistant";
  excerpt: string;
};

/**
 * A conversation the Trash holds: the row it came from, the name its file goes by there, and the
 * bytes it still takes on the machine. Mirrors `sessions::Trashed`.
 *
 * `name` rather than `id` because the two differ once a conversation has been deleted twice over —
 * the older copy is numbered instead of written over, and both parse to the same conversation.
 */
export type TrashedSession = SessionSummary & { name: string; bytes: number };

/**
 * What became of one path's revert. Mirrors `undo::Reverted`.
 *
 * A refusal is per path rather than per press: the other files in the same request can still go back,
 * and this one carries the reason it did not. `removed` says the file did not exist before the run,
 * so putting it back took it off disk.
 */
export type RevertResult = {
  path: string;
  ok: boolean;
  removed: boolean;
  why?: string;
};

/* ---- the Knowledge Center: the pinned catalog, and this machine's share of it ---- */

/**
 * One packing the catalog pins, plus whether its file is in the weights directory. Mirrors
 * `knowledge::PackRow`. `onDisk` is a size check rather than a hash, for the reason given there.
 */
export type CatalogPack = {
  key: string;
  file: string;
  quantType: string;
  bitsPerWeight: number;
  sizeGb: number;
  sizeBytes: number;
  onDisk: boolean;
  notes: string;
};

/**
 * A projector file the card publishes for images. Mirrors `knowledge::VisionRow`. It carries no
 * bits-per-weight because it is not a quantisation of the language model — it is the separate
 * network that maps picture embeddings in, so the catalog keeps it out of `CatalogPack`.
 */
export type CatalogVisionPack = {
  key: string;
  file: string;
  quantType: string;
  sizeGb: number;
  sizeBytes: number;
  /** The card's own pick, which is a claim about the pack rather than a state of this machine. */
  recommended: boolean;
  onDisk: boolean;
};

/**
 * A known issue the model card records, quoted rather than judged: the host does not evaluate the
 * condition against this machine here, which is what separates this page from the status strip.
 * Mirrors `knowledge::CaveatRow`.
 */
export type CatalogCaveat = {
  condition: string;
  symptom: string;
  workaround: string;
  severity: "moderate" | "severe" | "fatal";
};

/** Mirrors `knowledge::ModelCard`. */
export type CatalogModel = {
  id: string;
  name: string;
  repoId: string;
  repoUrl: string;
  license: string;
  baseModel: string;
  architecture: string;
  contextLength: number;
  packs: CatalogPack[];
  /** The projector files for image input, empty for a model whose card publishes none. */
  vision: CatalogVisionPack[];
  caveats: CatalogCaveat[];
  /** The catalog's own sizes summed over the packs marked present, so the figure cannot disagree
   * with the rows above it. */
  heldBytes: number;
};

/** Mirrors `knowledge::Center`. */
export type KnowledgeCenter = {
  catalogUpdated: string;
  weightsDir: string;
  engineTag: string;
  engineInstalled: boolean;
  models: CatalogModel[];
};

/* ---- the Extensions page: the tool registry as the agent is handed it ---- */

/**
 * `freecode_config::PermissionClass` in its snake_case wire spelling: what a call can do to the machine.
 */
export type PermissionClass = "read_only" | "workspace_write" | "side_effect" | "network";

/**
 * What the run would do with a call right now. `auto` runs without interrupting, `ask` pauses for the
 * user, `blocked` refuses — and all three are answered by `freecode_core::approvals::evaluate` against
 * the settings in force, which is what keeps this page from describing a policy the loop ignores.
 */
export type ToolDecision = "auto" | "ask" | "blocked";

/** One declared tool. Mirrors `extensions::ToolRow`. */
export type ExtensionTool = {
  id: string;
  title: string;
  description: string;
  permission: PermissionClass;
  requiresNetwork: boolean;
  /** Whether a call reaches an executor in this build. A declared tool with no executor still gets a
   * transcript row, so the page names the gap instead of hiding the row. */
  runnable: boolean;
  decision: ToolDecision;
};

/** Mirrors `extensions::Inventory`. */
export type ExtensionInventory = {
  tools: ExtensionTool[];
  runnable: number;
  workspaceRoot: string;
  approvalMode: ApprovalMode;
  networkAllowed: boolean;
};

/* ---- the Storage page: what this host has written on the machine ---- */

/**
 * One directory the page reports, mirroring `storage::Area`. `present` is whether the directory is
 * there at all: a pack that was never downloaded and one that was deleted are different facts, and
 * "0 MB" alone would not tell them apart.
 */
export type StorageArea = {
  dir: string;
  bytes: number;
  present: boolean;
};

/** Mirrors `storage::Report`. `root` is the directory the free figure was read on — where the next pack would be written. */
export type StorageReport = {
  weights: StorageArea;
  engine: StorageArea;
  sessions: StorageArea;
  totalBytes: number;
  freeBytes: number;
  root: string;
};

/** Mirrors `diagnostics::Report` — the machine and the build, not the settings. */
export type DiagnosticsReport = {
  appVersion: string;
  system: string;
  backend: string;
  /** What the GPU probe named, when it reached one. */
  chip: string | null;
  totalRamBytes: number;
  /** Absent on unified-memory machines, where `totalRamBytes` is also what the GPU sees. */
  gpuRamBytes: number | null;
  /** The vector set an x86 probe settled on; null where the question is not asked. */
  simd: string | null;
  root: string;
  engineTag: string;
  engineInstalled: boolean;
  model: string;
  repoId: string;
  packing: string;
  sizeGb: number;
  warnings: string[];
  uncheckedWarnings: string[];
  blocker: ModelStatus["blocked"];
};

export const ipc = {
  modelStatus: () => invoke<ModelStatus>("model_status"),
  /** Every model and packing the catalog pins, with the ones on this machine marked. */
  knowledgeCenter: () => invoke<KnowledgeCenter>("knowledge_center"),
  /** Every tool the agent is offered, with the current approvals and sandbox applied to each row. */
  extensionsInventory: () => invoke<ExtensionInventory>("extensions_inventory"),
  /** The three directories this app writes into, measured from the files on disk, plus the room the
   * volume has left. The walk is over gigabytes, so the page asks once and shows what comes back. */
  storageReport: () => invoke<StorageReport>("storage_report"),
  /** Which machine this is and which build is on it, assembled by the same probe the status card reads. */
  diagnosticsReport: () => invoke<DiagnosticsReport>("diagnostics_report"),
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
  /**
   * Show the desktop's folder chooser over the window and answer with the path, or `null` when the
   * user cancelled. The host shows it, so the webview is never given a filesystem capability; the
   * title is passed in because the dialog is the interface's language, not the host's.
   */
  chooseProjectFolder: (title: string) => invoke<string | null>("choose_project_folder", { title }),
  /**
   * Whether the Keychain holds a remote-provider key. The key itself is never returned: the page can
   * write one, remove it, and learn whether one is saved, which is the whole IPC surface of the secret.
   */
  remoteKeyPresent: () => invoke<boolean>("remote_key_present"),
  /** Write a pasted key to the login Keychain; answers whether one is saved now. */
  saveRemoteKey: (key: string) => invoke<boolean>("save_remote_key", { key }),
  /** Remove the saved key; answers whether one is saved now, which is always `false` on success. */
  clearRemoteKey: () => invoke<boolean>("clear_remote_key"),
  /**
   * The free-model registry as the picker draws it. No secret rides along: ids, labels, providers,
   * whether the gateway can be reached with this entry, and whether the OpenCode app on this Mac is
   * answering.
   */
  freeModelsList: () => invoke<FreeModelInfo[]>("free_models_list"),
  /** Whether the Keychain holds the free gateway key. The key itself is never returned. */
  freeKeyPresent: () => invoke<boolean>("free_key_present"),
  /** Write a pasted gateway key to the login Keychain; answers whether one is saved now. */
  saveFreeKey: (key: string) => invoke<boolean>("save_free_key", { key }),
  /** Remove the saved gateway key; answers whether one is saved now, always `false` on success. */
  clearFreeKey: () => invoke<boolean>("clear_free_key"),
  /**
   * Whether the OpenCode app's local service answers, and its version. A probe of this machine, not
   * a credential: the service's own password never crosses this boundary.
   */
  opencodeStatus: () => invoke<OpenCodeStatus>("opencode_status"),
  /**
   * Put a fence's text on the system pasteboard. The host does the writing because the webview is not
   * a place the web Clipboard API can be counted on, and its promise settles only once the pasteboard
   * has actually taken the text — which is what lets the button say "Copied" truthfully.
   */
  copyText: (text: string) => invoke<void>("copy_text", { text }),
  /**
   * Hand one of the files this run changed to the desktop's default handler, so the page an agent
   * built can be looked at rather than only read as a diff.
   *
   * The host opens it and confines the path to the workspace, because the webview's own opener is
   * scoped to `http` and `https` on purpose: a `file://` capability there would let any address a
   * model prints ask the desktop to launch something. Rejects with the host's reason — the panel
   * that just said this file changed shows it rather than going quiet.
   */
  openFile: (path: string) => invoke<void>("open_file", { path }),
  /**
   * The in-app address of one of the run's pages (`preview://localhost/…`), which the host serves
   * from the project folder through its `preview` scheme. The review's open control frames it for an
   * HTML file, so the page runs inside the app instead of leaving it. Same confinement as `open_file`,
   * and the same rule for a refusal: the panel shows the host's reason rather than going quiet.
   */
  previewUrl: (path: string) => invoke<string>("preview_url", { path }),
  /**
   * Put files a run wrote back to the text they held before it touched them.
   *
   * The before-images live in the host's journal for the conversation on screen, so a reopened
   * conversation is answered per path with a refusal rather than a restore it cannot prove. The host
   * checks each file still holds what the run left there: a file edited since is reported, not
   * silently overwritten.
   */
  revertChanges: (paths: string[]) => invoke<RevertResult[]>("revert_changes", { paths }),
  /** Launch the supervised fork server, or refuse with the load guard's own reason. */
  startEngine: () => invoke<EngineView>("start_engine"),
  stopEngine: () => invoke<void>("stop_engine"),
  engineStatus: () => invoke<EngineStatus>("engine_status"),
  /**
   * Send one prompt and stream the answer on `agent-event`. The promise settles when the run is
   * over, with the finished text — so a listener that missed a delta can still close the turn.
   *
   * `image` is a `data:<mime>;base64,…` URL. Asking with a picture is what makes the host start the
   * engine with its vision projector, so the first one after a text-only run also pays a reload.
   *
   * `tools` asks for the agent loop; the composer always sends true, since every prompt here is an
   * agent prompt. False, the host sends one request and answers in prose, and a tool call the model
   * emits anyway is a notice. On, the four read-only tools are offered and each call the model asks
   * for becomes a transcript row.
   */
  chatSend: (text: string, image?: string, tools?: boolean) =>
    invoke<string>("chat_send", {
      text,
      image: image ?? null,
      tools: tools ?? false,
    }),
  /**
   * Start a new task: forget the conversation in flight and keep the server up. This is what the
   * sidebar's "New task" row is — the history the next request is built from is the only thing a new
   * task resets, and reloading the weights for it would be a cost the click never asked for.
   *
   * What was on screen is not lost by it. Each turn is filed as it happens, so the conversation the
   * click abandons is already in the list below, and reopening it continues where it stopped.
   */
  newTask: () => invoke<void>("new_task"),
  /** Every conversation this app has filed, newest first. The sidebar's Workspaces group lists these. */
  listSessions: () => invoke<SessionSummary[]>("list_sessions"),
  /**
   * The filed conversations that say what `query` says, the ones matched by a question first. The
   * host searches them, because the words live in the turns and the turns deliberately stay on disk
   * until a conversation is opened.
   */
  searchSessions: (query: string) => invoke<SessionHit[]>("search_sessions", { query }),
  /**
   * Reopen one. The host answers with the conversation it also just put back into the history, so the
   * transcript and the engine agree on what was said — a row that showed turns the model no longer
   * remembers would let the next prompt be answered out of a different conversation than it looks
   * like it is continuing.
   */
  openSession: (id: string) => invoke<Session>("open_session", { id }),
  /**
   * Move one off the list. The file is not unlinked — it waits in the Trash under the conversation's
   * own name, where Settings → Storage lists it and offers it back. When the row is the conversation
   * on screen, the host forgets the history with the file, so the next prompt is not answered out of
   * a conversation the sidebar no longer lists. Refused while a reply streams into that conversation,
   * because the run would file its answer back into the row that has just gone.
   */
  deleteSession: (id: string) => invoke<void>("delete_session", { id }),
  /**
   * File the transcript the window is holding over the conversation on screen, so a reopened one
   * comes back with the run's step rows and their hunks rather than only its sentences.
   *
   * The whole list each time, because the window is the only thing that knows the order the rows came
   * in and which of them a folded batch swallowed. Refused while a reply streams — the answer those
   * rows describe has not been filed yet — and by a window holding no conversation at all.
   */
  fileRows: (rows: Row[]) => invoke<void>("file_rows", { rows }),
  /** What the Trash holds, newest first. Mirrors `sessions::Trashed`. */
  listTrashed: () => invoke<TrashedSession[]>("list_trashed"),
  /**
   * Put a trashed conversation back on the list. `name` is the name its file goes by in the Trash,
   * which is the conversation's id unless a second copy of one is waiting there. The press restores
   * a conversation; it does not open it.
   */
  restoreTrashed: (name: string) => invoke<SessionSummary>("restore_trashed", { name }),
  /** The press that means it: take a trashed conversation's file off the machine. */
  purgeTrashed: (name: string) => invoke<void>("purge_trashed", { name }),
  /**
   * Stop the reply that is streaming. The engine keeps its weights loaded, so the next message costs
   * nothing — `stopEngine` is the one that releases them, and the composer's Stop is not that.
   */
  chatStop: () => invoke<void>("chat_stop"),
  onAgentEvent: (onEvent: (event: AgentEvent) => void): Promise<() => void> =>
    listen<AgentEvent>(AGENT_EVENT, (event) => onEvent(event.payload)),
};
