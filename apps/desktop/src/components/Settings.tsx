// Settings, laid out control-for-control against docs/reference-ui/01-03: a searchable grouped nav
// on the left, and one page of labelled cards on the right. Every row here binds to a field the
// host actually persists — a setting with no storage is not rendered.
import { useEffect, useRef, useState } from "react";

import {
  IconCard,
  NumberField,
  Row,
  Section,
  Select,
  ShortcutChips,
  StyleCard,
  TextField,
  Toggle,
} from "./controls";
import { Icon, type IconName } from "./icons";
import { MASCOT_TILES, launchTile } from "./BrandMark";
import { has, translate, type Locale } from "../i18n";
import { formatBytes, formatMemory } from "../lib/bytes";
import {
  ipc,
  INTERFACE_STYLES,
  type DiagnosticsReport,
  type SettingsDoc,
  type StorageArea,
  type StorageReport,
  type TrashedSession,
} from "../lib/ipc";
import { SHORTCUTS, shortcutLabel } from "../lib/shortcuts";
import { useModel } from "../state/model";
import { useSettings } from "../state/settings";

const PAGE_IDS = [
  "appearance",
  "voice",
  "agent",
  "model",
  "reasoning",
  "shortcuts",
  "storage",
  "diagnostics",
] as const;
type Page = (typeof PAGE_IDS)[number];

const GROUPS: { key: string; items: { id: Page; icon: IconName }[] }[] = [
  {
    key: "basics",
    items: [
      { id: "appearance", icon: "wand" },
      { id: "voice", icon: "voice" },
      // Not the sidebar's `automations`, which this page borrows nothing from: what is set here is
      // what the agent may do to the machine, so the glyph is the one that means a boundary.
      { id: "agent", icon: "shield" },
      { id: "shortcuts", icon: "keyboard" },
    ],
  },
  {
    key: "model",
    items: [
      { id: "model", icon: "cube" },
      { id: "reasoning", icon: "thought" },
    ],
  },
  // Its own group because it sets nothing: this page answers a question about the machine, which is
  // not a preference, and filing it under Model would imply a control that is not there.
  {
    key: "machine",
    items: [
      { id: "storage", icon: "storage" },
      { id: "diagnostics", icon: "pulse" },
    ],
  },
];

/**
 * Which controls each page holds, by the stem of their `settings.row.*` label.
 *
 * Search has to answer "where do I set the context window?", and a page name is no help with that: the
 * row is on the Model page, which says nothing about windows. So each page declares the rows it renders
 * and the query is tested against those labels and their descriptions. `check-settings-index.mjs` fails
 * the build when a catalog row is missing from this list or a stem here names no row, which is what
 * makes it a copy of the page rather than a note about it.
 */
const CONTROLS: Record<Page, string[]> = {
  appearance: [
    "themeMode",
    "appIcon",
    "appIconRoll",
    "terminalTheme",
    "builtInBrowser",
    "language",
    "fontStyle",
    "contentWidth",
    "fileIcons",
    "glass",
    "lightCodeTheme",
    "darkCodeTheme",
    "showLineNumbers",
    "wrapLongLines",
    "uiFontSize",
    "codeFontSize",
  ],
  voice: [
    "inputDevice",
    "voiceInput",
    "recognitionPolish",
    "voiceprint",
    "sendAutomatically",
    "corrections",
    "liveVoiceShortcut",
  ],
  agent: [
    "approvalMode",
    "shellEnabled",
    "shellTimeout",
    "compressAt",
    "keepRecentTurns",
    "allowRemote",
    "telemetry",
  ],
  model: [
    "packing",
    "modelDir",
    "hfTokenEnv",
    "offloadAllLayers",
    "flashAttention",
    "visionLazyLoad",
    "dedicatedServerSlots",
  ],
  reasoning: [
    "effort",
    "nonThinking",
    "maxTokens",
    "contextTokens",
    "temperature",
    "topP",
    "topK",
    "minP",
    "presencePenalty",
    "repetitionPenalty",
    "systemPrompt",
  ],
  shortcuts: ["shortcutNewTask", "shortcutPalette", "shortcutSettings", "shortcutEscape"],
  storage: ["storageWeights", "storageEngine", "storageSessions", "storageFree"],
  diagnostics: [
    "diagVersion",
    "diagSystem",
    "diagRoot",
    "diagBackend",
    "diagChip",
    "diagMemory",
    "diagGpuMemory",
    "diagSimd",
    "diagModel",
    "diagEngine",
    "diagPacking",
  ],
};

// The list is the ipc module's, because the footer's Theme submenu sells exactly these eight.
const STYLES = INTERFACE_STYLES;

export function Settings({
  locale,
  page,
  onOpen,
  onBack,
}: {
  locale: Locale;
  /** Which page is open, held by the window's history rather than by this component: Back has to
   * return to the page the user was actually reading, not to whichever page the surface was first
   * reached through. */
  page: string;
  onOpen: (page: Page) => void;
  onBack: () => void;
}) {
  const doc = useSettings((s) => s.doc);
  const error = useSettings((s) => s.error);
  const load = useSettings((s) => s.load);
  const update = useSettings((s) => s.update);
  // An unknown deep-link page falls back to Appearance rather than rendering nothing.
  const active: Page = PAGE_IDS.includes(page as Page) ? (page as Page) : "appearance";
  // `?q=` seeds the field the way `?help=open` seeds the card below it: a result list is a state no
  // capture can reach by typing, and it is the state that has to look right on the real surface.
  const [query, setQuery] = useState(new URLSearchParams(window.location.search).get("q") ?? "");
  // The row a search hit names, once its page has opened. The nav can say "Context window", but the
  // answer to the query is the control itself, and a page that opens at its top sends the user back to
  // reading five sections to find it.
  const [jump, setJump] = useState<{ page: Page; title: string } | null>(null);
  const body = useRef<HTMLElement>(null);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!jump || jump.page !== active) return;
    // Read off the rendered title rather than from an id every row would have to carry: the row the
    // query matched is already on screen under this page, labelled in this language by the same catalog
    // key the nav item shows.
    const rows = body.current ? [...body.current.querySelectorAll<HTMLElement>(".card-row")] : [];
    const hit = rows.find((row) =>
      row.querySelector(".row-title")?.textContent?.trim().startsWith(jump.title),
    );
    if (!hit) return;
    hit.scrollIntoView({ block: "center" });
    hit.dataset.flash = "true";
    const id = window.setTimeout(() => delete hit.dataset.flash, 1400);
    return () => window.clearTimeout(id);
    // `doc` is a dependency because the page renders its rows only once the settings document has
    // arrived; a jump that ran against the loading placeholder would find nothing and say nothing.
  }, [jump, active, doc]);

  const t = (key: string, params?: Record<string, string | number>) =>
    translate(locale, `settings.${key}`, params);
  const set = (mutate: (doc: SettingsDoc) => void) => void update(mutate);

  const q = query.trim().toLowerCase();
  const finds = (key: string) => t(key).toLowerCase().includes(q);
  // A page matches by its own name or by any control it holds, and the controls that matched ride
  // along: typing "telemetry" should say *which* row on the Agent page it is on, not just that the
  // Agent page is somehow involved.
  const groups = GROUPS.map((g) => ({
    ...g,
    items: g.items
      .map((i) => ({
        ...i,
        hits: q
          ? CONTROLS[i.id].filter(
              (k) => finds(`row.${k}`) || (has(`settings.row.${k}.desc`) && finds(`row.${k}.desc`)),
            )
          : [],
      }))
      .filter((i) => !q || finds(`nav.${i.id}`) || finds(`about.${i.id}`) || i.hits.length > 0),
  })).filter((g) => g.items.length > 0);

  return (
    <div className="relative flex h-full min-h-0 bg-bg text-fg">
      <nav
        aria-label={t("title")}
        className="flex h-full w-[var(--inspector-width)] shrink-0 flex-col border-r border-line bg-panel"
      >
        <div
          className="pt-[var(--space-3)] pb-1 pr-[var(--space-3)]"
          style={{ paddingLeft: "max(var(--space-4), var(--traffic-light-inset))" }}
        >
          <button
            type="button"
            title={shortcutLabel("escape")}
            className="nav-item"
            onClick={onBack}
          >
            <Icon name="back" />
            <span>{t("back")}</span>
          </button>
        </div>

        <div className="px-[var(--space-3)] pb-[var(--space-2)]">
          <label className="field w-full">
            <Icon name="search" size={14} />
            <input
              className="field-input"
              type="search"
              value={query}
              placeholder={t("search")}
              aria-label={t("search")}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-3)] pb-[var(--space-3)]">
          {groups.length === 0 ? (
            <p className="px-3 py-2 text-quiet text-faint">{t("noResults", { query })}</p>
          ) : (
            groups.map((group) => (
              <div key={group.key}>
                <p className="section-label">{t(`group.${group.key}`)}</p>
                {group.items.map((item) => (
                  <div key={item.id}>
                    <button
                      type="button"
                      data-active={active === item.id || undefined}
                      className="nav-item"
                      onClick={() => onOpen(item.id)}
                    >
                      <Icon name={item.icon} />
                      <span className="truncate">{t(`nav.${item.id}`)}</span>
                    </button>
                    {item.hits.map((k) => {
                      const label = t(`row.${k}`);
                      return (
                        <button
                          key={k}
                          type="button"
                          className="nav-sub-item"
                          onClick={() => {
                            onOpen(item.id);
                            setJump({ page: item.id, title: label });
                          }}
                        >
                          {label}
                        </button>
                      );
                    })}
                  </div>
                ))}
              </div>
            ))
          )}
        </div>
      </nav>

      <main ref={body} className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[980px] px-[var(--space-6)] py-[var(--space-6)]">
          <h1 className="text-page font-semibold text-fg-strong">{t(`nav.${active}`)}</h1>
          <p className="mt-1 text-muted">{t(`about.${active}`)}</p>

          {error && (
            <p
              role="alert"
              className="mt-[var(--space-4)] rounded-[var(--radius-md)] border border-danger px-[var(--space-3)] py-[var(--space-2)] text-quiet text-danger"
            >
              {t("saveFailed", { error })}
            </p>
          )}

          <div className="mt-[var(--space-6)]">
            {!doc ? (
              <p className="text-muted">{t("loading")}</p>
            ) : (
              <Page doc={doc} page={active} set={set} t={t} locale={locale} />
            )}
          </div>
        </div>
      </main>

      <Help t={t} />
    </div>
  );
}

type T = (key: string, params?: Record<string, string | number>) => string;
type SetFn = (mutate: (doc: SettingsDoc) => void) => void;

function Page({
  doc,
  page,
  set,
  t,
  locale,
}: {
  doc: SettingsDoc;
  page: Page;
  set: SetFn;
  t: T;
  locale: Locale;
}) {
  switch (page) {
    case "appearance":
      return <Appearance doc={doc} set={set} t={t} locale={locale} />;
    case "voice":
      return <Voice doc={doc} set={set} t={t} locale={locale} />;
    case "agent":
      return <Agent doc={doc} set={set} t={t} />;
    case "model":
      return <Model doc={doc} set={set} t={t} />;
    case "reasoning":
      return <Reasoning doc={doc} set={set} t={t} />;
    case "shortcuts":
      return <Shortcuts t={t} />;
    case "storage":
      return <Storage t={t} />;
    case "diagnostics":
      return <Diagnostics locale={locale} t={t} />;
  }
}

/** Shot 02 and 03 both corner a help affordance here. It answers the one question this build can
 * actually settle — what is running, and what it is allowed to reach — rather than linking out to
 * documentation that does not exist yet. */
function Help({ t }: { t: T }) {
  const status = useModel((s) => s.status);
  // `?help=open` stands in for the click this window cannot be given, exactly as `?settings=` does:
  // the card it opens is the one surface on screen that glass paints, and a picture of it closed
  // would show nothing of the toggle that decides whether the page shows through it.
  const [open, setOpen] = useState(
    new URLSearchParams(window.location.search).get("help") === "open",
  );
  return (
    <div className="absolute bottom-[var(--space-6)] right-[var(--space-6)] flex flex-col items-end gap-[var(--space-2)]">
      {open && (
        <dl className="card surface w-[300px] p-[var(--space-4)] text-quiet shadow-raised">
          {status ? (
            <>
              <dt className="text-faint">{t("help.model")}</dt>
              <dd className="mb-[var(--space-2)] text-fg">{status.displayName}</dd>
              <dt className="text-faint">{t("help.engine")}</dt>
              <dd className="mb-[var(--space-2)] font-mono text-fg">
                {status.engineInstalled ? status.engineTag : t("help.engineMissing")}
              </dd>
              <dt className="text-faint">{t("help.packing")}</dt>
              <dd className="mb-[var(--space-2)] font-mono text-fg">
                {status.packing} · {status.sizeGb} GB
              </dd>
            </>
          ) : (
            <dd className="text-muted">{t("loading")}</dd>
          )}
          <dt className="text-faint">{t("help.egress")}</dt>
          <dd className="text-muted">
            {status?.local ? t("help.egress.local") : t("help.egress.remote")}
          </dd>
        </dl>
      )}
      <button
        type="button"
        aria-expanded={open}
        aria-label={t("help.title")}
        className="icon-btn size-9 rounded-full bg-fg text-bg"
        onClick={() => setOpen((v) => !v)}
      >
        <Icon name="help" />
      </button>
    </div>
  );
}

function Appearance({
  doc,
  set,
  t,
  locale,
}: {
  doc: SettingsDoc;
  set: SetFn;
  t: T;
  locale: Locale;
}) {
  const a = doc.appearance;
  const opt = (v: string) => t(`opt.${v}`);
  // Five of these rows write a field the window never reads: there is no terminal surface to theme,
  // no built-in browser to route a link through, no file list to icon, and the answer's code is
  // painted from the app's own colour tokens rather than from a bundled highlight stylesheet. They
  // stay because the reference shows them and the document they save is the one a later build
  // renders — but a row that changes nothing on screen says so, in the words the sidebar already
  // uses for the same case.
  const inert = translate(locale, "common.notInBuild");
  return (
    <>
      <Section label={t("section.interfaceStyle")}>
        <Row title={t("row.themeMode")} description={t("row.themeMode.desc")}>
          <Select
            label={t("row.themeMode")}
            icon={a.themeMode === "light" ? "sun" : a.themeMode === "dark" ? "moon" : "monitor"}
            value={a.themeMode}
            options={(["light", "dark", "system"] as const).map((v) => ({
              value: v,
              label: opt(v),
            }))}
            onChange={(v) => set((d) => void (d.appearance.themeMode = v))}
          />
        </Row>
        <div className="grid grid-cols-2 gap-[var(--space-3)] p-[var(--space-4)] md:grid-cols-4">
          {STYLES.map((s) => (
            <StyleCard
              key={s.id}
              style={s.id}
              name={s.label}
              selected={a.interfaceStyle === s.id}
              onSelect={() => set((d) => void (d.appearance.interfaceStyle = s.id))}
            />
          ))}
        </div>
        {/* The tiles already ship and the app already wears one of them; this adds only the
            choice, in the same group the reference puts its own icon control in. Pinning is host-side
            on purpose — the Dock tile is drawn by the host, which cannot see what the page decides —
            so the number travels through the settings document and the host repaints the tile the
            moment it lands. */}
        <div className="px-[var(--space-4)] pb-[var(--space-4)]">
          <p className="mb-[var(--space-2)] text-quiet text-muted">{t("row.appIcon")}</p>
          <div className="grid grid-cols-4 gap-[var(--space-3)] sm:grid-cols-6">
            {MASCOT_TILES.map((source, i) => (
              <IconCard
                key={source}
                source={source}
                label={t("row.appIcon.option", { index: i + 1 })}
                selected={a.appIcon === i + 1}
                onSelect={() => set((d) => void (d.appearance.appIcon = i + 1))}
              />
            ))}
          </div>
        </div>
        <Row title={t("row.appIconRoll")} description={t("row.appIconRoll.desc")}>
          <Toggle
            // Turning the roll off pins the tile the app is wearing at that moment rather than an
            // arbitrary one: the user is looking at the duck they just decided to keep.
            on={a.appIcon === null}
            label={t("row.appIconRoll")}
            onChange={(v) => set((d) => void (d.appearance.appIcon = v ? null : launchTile))}
          />
        </Row>
        <Row title={t("row.terminalTheme")} description={t("row.terminalTheme.desc")} inert={inert}>
          <Select
            label={t("row.terminalTheme")}
            value={a.terminalTheme}
            options={(["follow_theme", "light", "dark", "custom"] as const).map((v) => ({
              value: v,
              label: opt(v),
            }))}
            onChange={(v) => set((d) => void (d.appearance.terminalTheme = v))}
          />
        </Row>
        <Row
          title={t("row.builtInBrowser")}
          description={t("row.builtInBrowser.desc")}
          inert={inert}
        >
          <Toggle
            on={a.useBuiltInBrowserForTerminalLinks}
            label={t("row.builtInBrowser")}
            onChange={(v) => set((d) => void (d.appearance.useBuiltInBrowserForTerminalLinks = v))}
          />
        </Row>
        <Row title={t("row.language")} description={t("row.language.desc")}>
          <Select
            label={t("row.language")}
            value={a.language}
            options={(["en", "ru"] as const).map((v) => ({ value: v, label: opt(v) }))}
            onChange={(v) => set((d) => void (d.appearance.language = v))}
          />
        </Row>
        <Row title={t("row.fontStyle")} description={t("row.fontStyle.desc")}>
          <Select
            label={t("row.fontStyle")}
            value={a.fontStyle}
            options={(["sans", "serif", "mono"] as const).map((v) => ({ value: v, label: opt(v) }))}
            onChange={(v) => set((d) => void (d.appearance.fontStyle = v))}
          />
        </Row>
        <Row title={t("row.contentWidth")} description={t("row.contentWidth.desc")}>
          <Select
            label={t("row.contentWidth")}
            value={a.contentWidth}
            options={(["standard", "wide", "full"] as const).map((v) => ({
              value: v,
              label: opt(v),
            }))}
            onChange={(v) => set((d) => void (d.appearance.contentWidth = v))}
          />
        </Row>
        <Row title={t("row.fileIcons")} description={t("row.fileIcons.desc")} inert={inert}>
          <Select
            label={t("row.fileIcons")}
            value={a.fileIcons}
            options={(["qoder", "seti", "vscode", "none"] as const).map((v) => ({
              value: v,
              label: opt(v),
            }))}
            onChange={(v) => set((d) => void (d.appearance.fileIcons = v))}
          />
        </Row>
        <Row title={t("row.glass")} description={t("row.glass.desc")}>
          <Toggle
            on={a.glassEffects}
            label={t("row.glass")}
            onChange={(v) => set((d) => void (d.appearance.glassEffects = v))}
          />
        </Row>
      </Section>

      <Section label={t("section.code")} description={t("section.code.desc")}>
        <Row
          title={t("row.lightCodeTheme")}
          description={t("row.lightCodeTheme.desc")}
          inert={inert}
        >
          <TextField
            label={t("row.lightCodeTheme")}
            value={a.lightCodeTheme}
            onChange={(v) => set((d) => void (d.appearance.lightCodeTheme = v))}
          />
        </Row>
        <Row title={t("row.darkCodeTheme")} description={t("row.darkCodeTheme.desc")} inert={inert}>
          <TextField
            label={t("row.darkCodeTheme")}
            value={a.darkCodeTheme}
            onChange={(v) => set((d) => void (d.appearance.darkCodeTheme = v))}
          />
        </Row>
        <Row title={t("row.showLineNumbers")} description={t("row.showLineNumbers.desc")}>
          <Toggle
            on={a.showLineNumbers}
            label={t("row.showLineNumbers")}
            onChange={(v) => set((d) => void (d.appearance.showLineNumbers = v))}
          />
        </Row>
        <Row title={t("row.wrapLongLines")} description={t("row.wrapLongLines.desc")}>
          <Toggle
            on={a.wrapLongLines}
            label={t("row.wrapLongLines")}
            onChange={(v) => set((d) => void (d.appearance.wrapLongLines = v))}
          />
        </Row>
        <Row title={t("row.uiFontSize")} description={t("row.uiFontSize.desc")}>
          <NumberField
            label={t("row.uiFontSize")}
            value={a.uiFontSizePx}
            suffix="px"
            min={12}
            max={20}
            onChange={(v) => set((d) => void (d.appearance.uiFontSizePx = v))}
          />
        </Row>
        <Row title={t("row.codeFontSize")} description={t("row.codeFontSize.desc")}>
          <NumberField
            label={t("row.codeFontSize")}
            value={a.codeFontSizePx}
            suffix="px"
            min={10}
            max={18}
            onChange={(v) => set((d) => void (d.appearance.codeFontSizePx = v))}
          />
        </Row>
      </Section>
    </>
  );
}

function Voice({ doc, set, t, locale }: { doc: SettingsDoc; set: SetFn; t: T; locale: Locale }) {
  const v = doc.voice;
  const device = typeof v.inputDevice === "string" ? "automatic" : v.inputDevice.named;
  // No layer of this build opens a microphone or plays speech, so every row here stores a preference
  // and does nothing yet. The rows stay live — the document they save is the one a later build reads —
  // but each one says so, in the same words the Appearance page uses for the same case.
  const inert = translate(locale, "common.notInBuild");
  return (
    <>
      <Section label={t("section.general")}>
        <Row title={t("row.inputDevice")} description={t("row.inputDevice.desc")} inert={inert}>
          <Select
            label={t("row.inputDevice")}
            value={device}
            options={[{ value: "automatic", label: t("opt.automatic") }]}
            onChange={() => set((d) => void (d.voice.inputDevice = "automatic"))}
          />
        </Row>
      </Section>

      <Section label={t("section.voiceInput")}>
        <Row title={t("row.voiceInput")} description={t("row.voiceInput.desc")} inert={inert}>
          <Toggle
            on={v.voiceInput}
            label={t("row.voiceInput")}
            onChange={(next) => set((d) => void (d.voice.voiceInput = next))}
          />
        </Row>
        <Row
          title={t("row.recognitionPolish")}
          description={t("row.recognitionPolish.desc")}
          inert={inert}
        >
          <Toggle
            on={v.recognitionPolish}
            label={t("row.recognitionPolish")}
            onChange={(next) => set((d) => void (d.voice.recognitionPolish = next))}
          />
        </Row>
        <Row title={t("row.voiceprint")} description={t("row.voiceprint.desc")} inert={inert}>
          <Toggle
            on={v.voiceprintRecognition}
            label={t("row.voiceprint")}
            onChange={(next) => set((d) => void (d.voice.voiceprintRecognition = next))}
          />
        </Row>
        <Row
          title={t("row.sendAutomatically")}
          description={t("row.sendAutomatically.desc")}
          inert={inert}
        >
          <Toggle
            on={v.sendAutomatically}
            label={t("row.sendAutomatically")}
            onChange={(next) => set((d) => void (d.voice.sendAutomatically = next))}
          />
        </Row>
        <Corrections terms={v.commonWordCorrections} set={set} t={t} inert={inert} />
        <Row title={t("row.liveVoiceShortcut")} description={t("row.liveVoiceShortcut.desc")}>
          <ShortcutChips
            shortcut={v.liveVoiceShortcut}
            hint={translate(locale, "common.notInBuild")}
          />
        </Row>
      </Section>

      <p className="text-quiet text-faint">{translate(locale, "settings.voice.notInBuild")}</p>
    </>
  );
}

function Corrections({
  terms,
  set,
  t,
  inert,
}: {
  terms: string[];
  set: SetFn;
  t: T;
  inert: string;
}) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const term = draft.trim();
    if (!term) return;
    set((d) => {
      if (!d.voice.commonWordCorrections.includes(term)) d.voice.commonWordCorrections.push(term);
    });
    setDraft("");
  };
  return (
    <div className="card-row items-start">
      <div className="row-copy">
        <span className="row-title">
          {t("row.corrections")}
          <span className="row-inert">{inert}</span>
        </span>
        <span className="row-desc">{t("row.corrections.desc")}</span>
        <div className="mt-[var(--space-3)] flex items-center gap-[var(--space-2)]">
          <label className="field flex-1">
            <input
              className="field-input"
              type="text"
              value={draft}
              placeholder={t("corrections.placeholder")}
              aria-label={t("corrections.placeholder")}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && add()}
            />
          </label>
          <button type="button" className="field" disabled={!draft.trim()} onClick={add}>
            {t("corrections.add")}
          </button>
        </div>
        {terms.length === 0 ? (
          <p className="mt-[var(--space-2)] text-quiet text-faint">{t("corrections.empty")}</p>
        ) : (
          <ul className="mt-[var(--space-2)] flex flex-wrap gap-[var(--space-2)]">
            {terms.map((term) => (
              <li key={term}>
                <button
                  type="button"
                  className="pill"
                  title={t("corrections.remove", { term })}
                  onClick={() =>
                    set((d) => {
                      d.voice.commonWordCorrections = d.voice.commonWordCorrections.filter(
                        (x) => x !== term,
                      );
                    })
                  }
                >
                  {term}
                  <Icon name="close" size={12} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function Agent({ doc, set, t }: { doc: SettingsDoc; set: SetFn; t: T }) {
  const { approvals, context, privacy } = doc;
  return (
    <>
      <Section label={t("section.approvals")}>
        <Row title={t("row.approvalMode")} description={t("row.approvalMode.desc")}>
          <Select
            label={t("row.approvalMode")}
            value={approvals.mode}
            options={(["auto_approve", "ask_always", "plan_only", "custom"] as const).map((v) => ({
              value: v,
              label: t(`opt.${v}`),
            }))}
            onChange={(v) => set((d) => void (d.approvals.mode = v))}
          />
        </Row>
        <Row title={t("row.shellEnabled")} description={t("row.shellEnabled.desc")}>
          <Toggle
            on={approvals.shellEnabled}
            label={t("row.shellEnabled")}
            onChange={(next) => set((d) => void (d.approvals.shellEnabled = next))}
          />
        </Row>
        <Row title={t("row.shellTimeout")} description={t("row.shellTimeout.desc")}>
          <NumberField
            label={t("row.shellTimeout")}
            value={Math.round(approvals.shellTimeoutMs / 1000)}
            suffix="s"
            min={5}
            max={600}
            onChange={(v) => set((d) => void (d.approvals.shellTimeoutMs = v * 1000))}
          />
        </Row>
      </Section>

      <Section label={t("section.context")}>
        <Row title={t("row.compressAt")} description={t("row.compressAt.desc")}>
          <NumberField
            label={t("row.compressAt")}
            value={Math.round(context.compressAtRatio * 100)}
            suffix="%"
            min={10}
            max={95}
            onChange={(v) => set((d) => void (d.context.compressAtRatio = v / 100))}
          />
        </Row>
        <Row title={t("row.keepRecentTurns")} description={t("row.keepRecentTurns.desc")}>
          <NumberField
            label={t("row.keepRecentTurns")}
            value={context.keepRecentTurns}
            min={1}
            max={64}
            onChange={(v) => set((d) => void (d.context.keepRecentTurns = v))}
          />
        </Row>
      </Section>

      <Section label={t("section.privacy")}>
        <Row title={t("row.allowRemote")} description={t("row.allowRemote.desc")}>
          <Toggle
            on={privacy.allowRemoteProviders}
            label={t("row.allowRemote")}
            onChange={(v) => set((d) => void (d.privacy.allowRemoteProviders = v))}
          />
        </Row>
        {/* Reported, not switched: nothing in this build emits usage data, so a toggle here would
         * offer a choice the code does not honour. */}
        <Row title={t("row.telemetry")} description={t("row.telemetry.desc")}>
          <span className="pill">{t("row.telemetry.value")}</span>
        </Row>
      </Section>
    </>
  );
}

/**
 * The keys this window answers, and nothing else.
 *
 * There is no control on this page because there is no preference behind one: the four bindings are
 * the table `lib/shortcuts.ts` holds, which is the same table the window's listener reads and the
 * script that fails the build when a printed key has no handler. A row here is therefore a promise
 * the app keeps, and a fifth row would need a binding before it could exist.
 */
function Shortcuts({ t }: { t: T }) {
  return (
    <Section label={t("section.keys")} description={t("section.keys.desc")}>
      {(
        [
          ["shortcutNewTask", "newTask"],
          ["shortcutPalette", "palette"],
          ["shortcutSettings", "settings"],
          ["shortcutEscape", "escape"],
        ] as const
      ).map(([key, id]) => (
        <Row key={key} title={t(`row.${key}`)} description={t(`row.${key}.desc`)}>
          <ShortcutChips shortcut={SHORTCUTS[id]} />
        </Row>
      ))}
    </Section>
  );
}

function Model({ doc, set, t }: { doc: SettingsDoc; set: SetFn; t: T }) {
  const m = doc.localModel;
  const status = useModel((s) => s.status);
  const packing = typeof m.packing === "string" ? "auto" : m.packing.forced;
  const packOptions = [
    { value: "auto", label: t("opt.auto") },
    ...(status?.packs ?? []).map((p) => ({
      value: p.key,
      label: p.viable ? `${p.key} · ${p.sizeGb} GB` : `${p.key} — ${p.reason}`,
    })),
  ];
  return (
    <>
      <Section label={t("section.packing")}>
        <Row title={t("row.packing")} description={t("row.packing.desc")}>
          <Select
            label={t("row.packing")}
            value={packing}
            options={packOptions}
            onChange={(v) =>
              set((d) => void (d.localModel.packing = v === "auto" ? "auto" : { forced: v }))
            }
          />
        </Row>
        <Row title={t("row.modelDir")} description={t("row.modelDir.desc")}>
          <TextField
            mono
            label={t("row.modelDir")}
            value={m.modelDir ?? ""}
            placeholder={t("row.modelDir.placeholder")}
            onChange={(v) => set((d) => void (d.localModel.modelDir = v === "" ? null : v))}
          />
        </Row>
        <Row title={t("row.hfTokenEnv")} description={t("row.hfTokenEnv.desc")}>
          <TextField
            mono
            label={t("row.hfTokenEnv")}
            value={m.hfTokenEnv ?? ""}
            onChange={(v) => set((d) => void (d.localModel.hfTokenEnv = v === "" ? null : v))}
          />
        </Row>
      </Section>

      <Section label={t("section.engine")}>
        {(
          [
            ["offloadAllLayers", m.offloadAllLayers],
            ["flashAttention", m.flashAttention],
            ["visionLazyLoad", m.visionLazyLoad],
            ["dedicatedServerSlots", m.dedicatedServerSlots],
          ] as const
        ).map(([key, on]) => (
          <Row key={key} title={t(`row.${key}`)} description={t(`row.${key}.desc`)}>
            <Toggle
              on={on}
              label={t(`row.${key}`)}
              onChange={(v) =>
                set((d) => {
                  d.localModel[key] = v;
                })
              }
            />
          </Row>
        ))}
      </Section>
    </>
  );
}

function Reasoning({ doc, set, t }: { doc: SettingsDoc; set: SetFn; t: T }) {
  const s = doc.sampling;
  const active = s.nonThinkingMode ? s.instruct : s.thinking;
  return (
    <>
      <Section label={t("section.reasoning")}>
        <Row title={t("row.effort")} description={t("row.effort.desc")}>
          <Select
            label={t("row.effort")}
            value={s.effort}
            options={(["auto", "low", "medium", "xhigh", "none"] as const).map((v) => ({
              value: v,
              label: t(`opt.${v}`),
            }))}
            onChange={(v) => set((d) => void (d.sampling.effort = v))}
          />
        </Row>
        <Row title={t("row.nonThinking")} description={t("row.nonThinking.desc")}>
          <Toggle
            on={s.nonThinkingMode}
            label={t("row.nonThinking")}
            onChange={(v) => set((d) => void (d.sampling.nonThinkingMode = v))}
          />
        </Row>
        <Row title={t("row.maxTokens")} description={t("row.maxTokens.desc")}>
          <NumberField
            label={t("row.maxTokens")}
            value={s.budget.maxTokens}
            min={4096}
            max={131072}
            step={1024}
            onChange={(v) => set((d) => void (d.sampling.budget.maxTokens = v))}
          />
        </Row>
        <Row title={t("row.contextTokens")} description={t("row.contextTokens.desc")}>
          <NumberField
            label={t("row.contextTokens")}
            value={s.budget.contextTokens}
            min={4096}
            max={262144}
            step={1024}
            onChange={(v) => set((d) => void (d.sampling.budget.contextTokens = v))}
          />
        </Row>
      </Section>

      <Section
        label={t("section.sampling")}
        description={t("section.sampling.desc", {
          mode: t(s.nonThinkingMode ? "opt.instruct" : "opt.thinking"),
        })}
      >
        {(
          [
            ["temperature", 0.01],
            ["topP", 0.01],
            ["topK", 1],
            ["minP", 0.01],
            ["presencePenalty", 0.1],
            ["repetitionPenalty", 0.05],
          ] as const
        ).map(([key, step]) => (
          <Row key={key} title={t(`row.${key}`)} description={t(`row.${key}.desc`)}>
            <NumberField
              label={t(`row.${key}`)}
              value={active[key]}
              min={0}
              max={key === "topK" ? 200 : 40}
              step={step}
              onChange={(v) =>
                set((d) => {
                  const profile = d.sampling.nonThinkingMode
                    ? d.sampling.instruct
                    : d.sampling.thinking;
                  profile[key] = v;
                })
              }
            />
          </Row>
        ))}
      </Section>

      <Section label={t("section.systemPrompt")}>
        <Row title={t("row.systemPrompt")} description={t("row.systemPrompt.desc")}>
          <TextField
            label={t("row.systemPrompt")}
            value={s.systemPrompt}
            onChange={(v) => set((d) => void (d.sampling.systemPrompt = v))}
          />
        </Row>
      </Section>
    </>
  );
}

/**
 * What this app has put on the machine, and what the volume behind it has left.
 *
 * The one Settings page that reads nothing from the settings document, because none of these figures
 * are preferences: they are the filesystem, measured by the host as the page opens. And the one page
 * with no control on it. A "Clear weights" button would act on a directory the app does not own — the
 * Model page lets a path be typed into `modelDir`, and whatever sits there may be the user's own
 * files — so the page states what is there rather than offering to change it. The free-space figure is
 * the same probe a download is refused by, which makes it the one number here a user would act on.
 */
function Storage({ t }: { t: T }) {
  const [report, setReport] = useState<StorageReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void ipc
      .storageReport()
      .then((found) => {
        if (live) setReport(found);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, []);

  if (error) {
    return (
      <p
        role="alert"
        className="rounded-[var(--radius-md)] border border-danger px-[var(--space-3)] py-[var(--space-2)] text-quiet text-danger"
      >
        {t("storage.failed", { error })}
      </p>
    );
  }
  // The walk is over gigabytes of weights directory, so it arrives a moment after the rest of the
  // page. A placeholder that said nothing at all would read as an empty machine.
  if (!report) return <p className="text-muted">{t("storage.measuring")}</p>;

  const figure = (text: string) => (
    <span className="font-mono tabular-nums text-quiet text-fg">{text}</span>
  );
  // A directory that was never created is not a directory of zero bytes: the first is this app not
  // having needed that file yet, the second is it having lost what it downloaded.
  const held = (area: StorageArea) =>
    area.present ? formatBytes(area.bytes) : t("row.storage.none");

  return (
    <>
      <Section
        label={t("section.storageFiles")}
        description={t("section.storageFiles.desc", { size: formatBytes(report.totalBytes) })}
      >
        <Row title={t("row.storageWeights")} description={report.weights.dir}>
          {figure(held(report.weights))}
        </Row>
        <Row title={t("row.storageEngine")} description={report.engine.dir}>
          {figure(held(report.engine))}
        </Row>
        <Row
          title={t("row.storageSessions")}
          description={report.sessions.dir || t("row.storageSessions.noDir")}
        >
          {figure(held(report.sessions))}
        </Row>
      </Section>

      <Section label={t("section.storageFree")}>
        <Row title={t("row.storageFree")} description={t("row.storageFree.desc")}>
          {figure(formatBytes(report.freeBytes))}
        </Row>
      </Section>

      <Trash t={t} />
    </>
  );
}

/**
 * The conversations the sidebar stopped listing, and the two presses a bin has to offer: put one
 * back, or take its file off the machine.
 *
 * A delete moves the file into `sessions/trash/` rather than unlinking it, because the file is the
 * only copy of the exchange and the press that throws it away is a glyph that appears on hover. That
 * makes the bytes still held and still recoverable, which is what this page reports — and it is why
 * the section renders nothing until there is something in it: an empty bin is not a fact about the
 * machine, and the figures above already count the directory these files sit in.
 */
function Trash({ t }: { t: T }) {
  const [held, setHeld] = useState<TrashedSession[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Re-read after every press rather than edited here: the host is what moved the file, and a row
  // that vanished locally before the move landed would be a claim about the disk.
  const [read, setRead] = useState(0);
  // The purging press arms, like the sidebar's delete. Restoring needs no second thought; this one
  // is the only path in the app that loses a conversation.
  const [armed, setArmed] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void ipc
      .listTrashed()
      .then((found) => {
        if (!live) return;
        setHeld(found);
        setError(null);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [read]);

  if (error) {
    return (
      <p
        role="alert"
        className="rounded-[var(--radius-md)] border border-danger px-[var(--space-3)] py-[var(--space-2)] text-quiet text-danger"
      >
        {t("trash.failed", { error })}
      </p>
    );
  }
  if (!held || held.length === 0) return null;

  const press = (run: () => Promise<unknown>) => {
    void run().then(
      () => setRead((n) => n + 1),
      (e: unknown) => setError(String(e)),
    );
  };

  return (
    <Section
      label={t("section.trash")}
      description={t(held.length === 1 ? "section.trash.descOne" : "section.trash.descMany", {
        count: held.length,
      })}
    >
      {held.map((item) => (
        <Row
          key={item.name}
          title={item.title}
          description={`${item.workspace} · ${item.model} · ${t(
            item.turnCount === 1 ? "trash.turnsOne" : "trash.turnsMany",
            { count: item.turnCount },
          )} · ${formatBytes(item.bytes)}`}
        >
          <span className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              className="field"
              onClick={() => press(() => ipc.restoreTrashed(item.name))}
            >
              {t("trash.restore")}
            </button>
            <button
              type="button"
              className={`field ${armed === item.name ? "text-danger" : ""}`}
              onClick={() => {
                if (armed !== item.name) {
                  setArmed(item.name);
                  return;
                }
                setArmed(null);
                press(() => ipc.purgeTrashed(item.name));
              }}
            >
              {t(armed === item.name ? "trash.purgeAgain" : "trash.purge")}
            </button>
          </span>
        </Row>
      ))}
    </Section>
  );
}

/**
 * Which machine this is, and which build is running on it.
 *
 * The second page in the group that sets nothing, and the one that answers the question the status
 * card cannot: not "may I run" but "what are these numbers". Every figure comes from the same
 * assembly the card reads, so the two surfaces cannot disagree about one probe, and the notices the
 * card folds behind a disclosure are printed here in full — a page whose whole job is reporting has
 * nothing to fold. Values are rendered into the DOM rather than held in a parallel list because the
 * copy control below takes what is on the screen, in the language on the screen; a report that
 * drifted from the page that produced it would be the worst kind of wrong.
 */
function Diagnostics({ locale, t }: { locale: Locale; t: T }) {
  const [report, setReport] = useState<DiagnosticsReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState<"idle" | "done" | "failed">("idle");
  const page = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let live = true;
    void ipc
      .diagnosticsReport()
      .then((found) => {
        if (live) setReport(found);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, []);

  // The same beat the fence copy uses: the label states a fact about the pasteboard, so it holds
  // long enough to read and then stops claiming anything.
  useEffect(() => {
    if (copied === "idle") return;
    const id = window.setTimeout(() => setCopied("idle"), 1800);
    return () => window.clearTimeout(id);
  }, [copied]);

  if (error) {
    return (
      <p
        role="alert"
        className="rounded-[var(--radius-md)] border border-danger px-[var(--space-3)] py-[var(--space-2)] text-quiet text-danger"
      >
        {t("diagnostics.failed", { error })}
      </p>
    );
  }
  // This is the probe the status card pays at launch, so it is not instant on a machine with gigabytes
  // of weights to stat.
  if (!report) return <p className="text-muted">{t("diagnostics.reading")}</p>;

  const row = (key: string, text: string) => (
    <Row key={key} title={t(`row.${key}`)} description={t(`row.${key}.desc`)}>
      {/* `data-fact` is the hook the copy control below reads this figure back off the DOM by, not a
       * style: the figure's own classes are the Storage page's. */}
      <span
        data-fact="true"
        className="max-w-[60%] break-words text-right font-mono tabular-nums text-quiet text-fg"
      >
        {text}
      </span>
    </Row>
  );
  // A path is not a figure. It goes where the Storage page puts its paths — in the row's own
  // description, which is left-aligned and wraps on its own terms — rather than in the trailing
  // slot, where a 60-character directory breaks into an orphaned letter.
  const pathRow = (key: string, path: string) => (
    <Row key={key} title={t(`row.${key}`)} description={path} />
  );

  // The unchecked ones are marked here rather than hidden: this is the surface a user reads before
  // pasting the report into a bug thread, and a caveat about another machine is still a caveat about
  // the pin, which is what the thread is about.
  const notices = [
    ...(report.blocker
      ? [
          `${translate(locale, `blocker.${report.blocker.code}.title`)} — ${report.blocker.technical}`,
        ]
      : []),
    ...report.warnings,
    ...report.uncheckedWarnings.map((w) => `${w} (${t("diag.unverified")})`),
  ];

  async function copy() {
    const root = page.current;
    if (!root || !report) return;
    // Read back what rendered, in the order it rendered: the page and the pasteboard are then one
    // text by construction rather than two lists that happen to agree today.
    const facts = [...root.querySelectorAll<HTMLElement>(".card-row")].map((r) => {
      const label = r.querySelector(".row-title")?.textContent?.trim() ?? "";
      // A row's value is its figure, or — for the one row whose value is a path — the description the
      // path is rendered in.
      const figure =
        r.querySelector('[data-fact="true"]')?.textContent?.trim() ||
        r.querySelector(".row-desc")?.textContent?.trim() ||
        "";
      return `${label}: ${figure}`;
    });
    try {
      await ipc.copyText(
        [
          `${t("diag.heading", { version: report.appVersion })} · ${new Date().toLocaleString()}`,
          ...facts,
          ...notices,
        ].join("\n"),
      );
      setCopied("done");
    } catch (reason) {
      // The host's reason is about the pasteboard; the label has already said the copy did not happen.
      console.warn("copy failed", reason);
      setCopied("failed");
    }
  }

  return (
    <div ref={page}>
      <Section label={t("section.diagBuild")} description={t("section.diagBuild.desc")}>
        {row("diagVersion", report.appVersion)}
        {row("diagSystem", report.system)}
        {pathRow("diagRoot", report.root)}
      </Section>

      <Section label={t("section.diagMachine")} description={t("section.diagMachine.desc")}>
        {row("diagBackend", report.backend)}
        {row("diagChip", report.chip ?? t("row.diagChip.none"))}
        {row("diagMemory", formatMemory(report.totalRamBytes))}
        {row(
          "diagGpuMemory",
          report.gpuRamBytes === null
            ? t("row.diagGpuMemory.shared")
            : formatMemory(report.gpuRamBytes),
        )}
        {row("diagSimd", report.simd ?? t("row.diagSimd.none"))}
      </Section>

      <Section label={t("section.diagRun")} description={t("section.diagRun.desc")}>
        {row("diagModel", `${report.model} · ${report.repoId}`)}
        {row("diagEngine", report.engineInstalled ? report.engineTag : t("row.diagEngine.missing"))}
        {row("diagPacking", `${report.packing} · ${report.sizeGb.toFixed(2)} GB`)}
      </Section>

      <Section label={t("section.diagNotices")} description={t("section.diagNotices.desc")}>
        <div className="diag-notices">
          {notices.length === 0 ? (
            <p className="text-quiet text-muted">{t("diag.nothing")}</p>
          ) : (
            <ul className="flex flex-col gap-[var(--space-2)]">
              {notices.map((notice) => (
                <li key={notice} className="text-quiet text-warning">
                  {notice}
                </li>
              ))}
            </ul>
          )}
        </div>
      </Section>

      <button
        type="button"
        onClick={() => void copy()}
        className="mt-[var(--space-4)] inline-flex items-center gap-1.5 rounded-[var(--radius-md)] border border-line px-[var(--space-3)] py-1 text-quiet text-muted hover:bg-hover hover:text-fg"
      >
        <Icon name="copy" size={13} />
        {copied === "done"
          ? t("diag.copied")
          : copied === "failed"
            ? t("diag.copyFailed")
            : t("diag.copy")}
      </button>
    </div>
  );
}
