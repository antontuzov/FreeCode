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
import type { Accent, SettingsDoc } from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";
import { useModel } from "../state/model";
import { useSettings } from "../state/settings";

const PAGE_IDS = ["appearance", "voice", "agent", "model", "reasoning"] as const;
type Page = (typeof PAGE_IDS)[number];

const GROUPS: { key: string; items: { id: Page; icon: IconName }[] }[] = [
  {
    key: "basics",
    items: [
      { id: "appearance", icon: "wand" },
      { id: "voice", icon: "voice" },
      { id: "agent", icon: "automations" },
    ],
  },
  {
    key: "model",
    items: [
      { id: "model", icon: "cube" },
      { id: "reasoning", icon: "thought" },
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
};

const STYLES: { id: Accent; label: string }[] = [
  { id: "forest", label: "Forest" },
  { id: "mint", label: "Mint" },
  { id: "bee", label: "Bee" },
  { id: "parchment", label: "Parchment" },
  { id: "rain", label: "Rain" },
];

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
        {/* The twelve tiles already ship and the app already wears one of them; this adds only the
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
            options={(["en", "zh"] as const).map((v) => ({ value: v, label: opt(v) }))}
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
  const status = useModel((s) => s.status);
  const device = typeof v.inputDevice === "string" ? "automatic" : v.inputDevice.named;
  return (
    <>
      <Section label={t("section.general")}>
        <Row title={t("row.inputDevice")} description={t("row.inputDevice.desc")}>
          <Select
            label={t("row.inputDevice")}
            value={device}
            options={[{ value: "automatic", label: t("opt.automatic") }]}
            onChange={() => set((d) => void (d.voice.inputDevice = "automatic"))}
          />
        </Row>
      </Section>

      <Section label={t("section.voiceInput")}>
        <Row title={t("row.voiceInput")} description={t("row.voiceInput.desc")}>
          <Toggle
            on={v.voiceInput}
            label={t("row.voiceInput")}
            onChange={(next) => set((d) => void (d.voice.voiceInput = next))}
          />
        </Row>
        <Row title={t("row.recognitionPolish")} description={t("row.recognitionPolish.desc")}>
          <Toggle
            on={v.recognitionPolish}
            label={t("row.recognitionPolish")}
            onChange={(next) => set((d) => void (d.voice.recognitionPolish = next))}
          />
        </Row>
        <Row title={t("row.voiceprint")} description={t("row.voiceprint.desc")}>
          <Toggle
            on={v.voiceprintRecognition}
            label={t("row.voiceprint")}
            onChange={(next) => set((d) => void (d.voice.voiceprintRecognition = next))}
          />
        </Row>
        <Row title={t("row.sendAutomatically")} description={t("row.sendAutomatically.desc")}>
          <Toggle
            on={v.sendAutomatically}
            label={t("row.sendAutomatically")}
            onChange={(next) => set((d) => void (d.voice.sendAutomatically = next))}
          />
        </Row>
        <Corrections terms={v.commonWordCorrections} set={set} t={t} />
        <Row title={t("row.liveVoiceShortcut")} description={t("row.liveVoiceShortcut.desc")}>
          <ShortcutChips
            shortcut={v.liveVoiceShortcut}
            hint={translate(locale, "common.notInBuild")}
          />
        </Row>
      </Section>

      {/* Recording is the engine's job, so the page says so instead of implying a microphone works
       * before the runtime is on disk. */}
      {status && !status.engineInstalled && (
        <p className="text-quiet text-faint">{translate(locale, "settings.voice.pending")}</p>
      )}
    </>
  );
}

function Corrections({ terms, set, t }: { terms: string[]; set: SetFn; t: T }) {
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
        <span className="row-title">{t("row.corrections")}</span>
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
