// The Knowledge Center: the pinned model catalog as a page, with this machine's share of it marked.
//
// It is the catalog rather than a store of the app's own because that file is the source of truth for
// every number this app quotes — pack sizes, bits per weight, context length, the known issues — and
// a page that re-typed them would be a second copy to drift from the pin. What the host adds is the
// one fact only it can get: which of those files are sitting in the weights directory.
//
// The status strip answers "can this machine run the model". This answers "what is there, and what
// does the card say about it", which is why the caveats below are labelled as the card's records
// rather than shown as warnings: nothing here evaluates their conditions against this machine.
import { useEffect, useState } from "react";

import { Row, Section } from "./controls";
import { ExternalLink } from "./ExternalLink";
import { Icon } from "./icons";
import { translate, type Locale } from "../i18n";
import { ipc, type CatalogModel, type KnowledgeCenter } from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";

/** The nav's non-model item, which is the one page that is not about a catalog entry. */
const MACHINE = "";

/** The catalog publishes bytes; a GB figure with four decimals is not a number anyone reads. */
function gb(bytes: number): string {
  return (bytes / 1_000_000_000).toFixed(2);
}

/** A pack is on this machine or it is not, and the engine is installed or it is not; the same pair of
 * states ends rows on both pages, so it is drawn once. The present one takes the accent's soft fill
 * the way the status card's badge does, and the absent one stays on the sunken ground — a file that
 * was never downloaded is a fact about the directory, not a warning about the machine. */
function State({ on, children }: { on: boolean; children: string }) {
  return (
    <span
      className={`shrink-0 rounded-[var(--radius-pill)] px-2 py-0.5 text-fine ${
        on ? "bg-accent-soft text-fg-strong" : "bg-sunken text-muted"
      }`}
    >
      {children}
    </span>
  );
}

type T = (key: string, params?: Record<string, string | number>) => string;

export function Knowledge({
  locale,
  initial,
  onBack,
}: {
  locale: Locale;
  /** The page to open, by model id; `""` is the machine page, and an id the catalog does not carry
   * falls back to it. The deep link `?knowledge=<id>` is the only caller that passes anything else,
   * because opening a page by clicking its row is a click the native window cannot make. */
  initial: string;
  onBack: () => void;
}) {
  const [center, setCenter] = useState<KnowledgeCenter | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The machine page opens by default: on a launch that has fetched nothing it is the only one with
  // anything this-machine to say, and a model page reads the same whatever has been downloaded.
  const [active, setActive] = useState<string>(initial);
  const t = (key: string, params?: Record<string, string | number>) =>
    translate(locale, `knowledge.${key}`, params);

  useEffect(() => {
    let live = true;
    void ipc
      .knowledgeCenter()
      .then((found) => {
        if (live) setCenter(found);
      })
      .catch((e: unknown) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, []);

  const model: CatalogModel | undefined = center?.models.find((m) => m.id === active);
  const heldOf = (m: CatalogModel) =>
    t("heldCount", { held: m.packs.filter((p) => p.onDisk).length, total: m.packs.length });

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

        <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-3)] pb-[var(--space-3)]">
          <button
            type="button"
            data-active={active === MACHINE || undefined}
            className="nav-item"
            onClick={() => setActive(MACHINE)}
          >
            <Icon name="monitor" />
            <span className="truncate">{t("machine")}</span>
          </button>

          <p className="section-label">{t("models")}</p>
          {center?.models.map((m) => (
            <button
              key={m.id}
              type="button"
              data-active={active === m.id || undefined}
              title={`${heldOf(m)} · ${m.repoId}`}
              className="nav-item"
              onClick={() => setActive(m.id)}
            >
              <Icon name="cube" />
              <span className="truncate">{m.name}</span>
            </button>
          ))}
          {!center && !error && (
            // The column holds its shape through the round trip rather than arriving empty and
            // filling, the way the status card does for the same reason.
            <div className="flex flex-col gap-1">
              {[0, 1].map((i) => (
                <span key={i} className="skeleton-line block h-7" />
              ))}
            </div>
          )}
        </div>
      </nav>

      <main className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-[980px] px-[var(--space-6)] py-[var(--space-6)]">
          {/* Which page is open is only knowable once the catalog answers, so the heading waits for
              that rather than claiming "This machine" and then changing under the reader. */}
          <h1 className="text-page font-semibold text-fg-strong">
            {model ? model.name : center ? t("machine") : t("title")}
          </h1>
          <p className="mt-1 text-muted">
            {!center
              ? t("loading")
              : model
                ? `${model.repoId} · ${model.license}`
                : t("machine.about")}
          </p>

          {error && (
            <p
              role="alert"
              className="mt-[var(--space-4)] rounded-[var(--radius-md)] border border-danger px-[var(--space-3)] py-[var(--space-2)] text-quiet text-danger"
            >
              {t("loadFailed", { error })}
            </p>
          )}

          {center &&
            (model ? (
              <ModelPage locale={locale} model={model} center={center} t={t} />
            ) : (
              <MachinePage center={center} t={t} heldOf={heldOf} />
            ))}
        </div>
      </main>
    </div>
  );
}

function ModelPage({
  locale,
  model,
  center,
  t,
}: {
  locale: Locale;
  model: CatalogModel;
  center: KnowledgeCenter;
  t: T;
}) {
  return (
    <>
      <Section label={t("packs")} description={t("packs.desc")}>
        {model.packs.map((pack) => (
          <Row
            key={pack.file}
            // Most packs are named for their quant, so the two halves of the title are the same word;
            // a second copy of it is a stutter rather than an extra fact.
            title={pack.key === pack.quantType ? pack.key : `${pack.key} · ${pack.quantType}`}
            description={`${t("bits", { bits: pack.bitsPerWeight })} · ${t("size", {
              gb: pack.sizeGb.toFixed(2),
            })} · ${pack.file}${pack.notes ? ` — ${pack.notes}` : ""}`}
          >
            <State on={pack.onDisk}>{pack.onDisk ? t("pack.held") : t("pack.absent")}</State>
          </Row>
        ))}
      </Section>

      {/* The half of "this model takes images" that the pack rows cannot say: a projector is not a
          quantisation of the language model, so it belongs on its own rows. Recorded here because a
          reader choosing a model for image input needs the file named, not the architecture string. */}
      <Section label={t("vision")} description={t("vision.desc")}>
        {model.vision.length === 0 && (
          <Row title={t("vision.none")}>
            <span />
          </Row>
        )}
        {model.vision.map((proj) => (
          <Row
            key={proj.file}
            title={proj.key === proj.quantType ? proj.key : `${proj.key} · ${proj.quantType}`}
            description={`${t("size", { gb: proj.sizeGb.toFixed(2) })} · ${proj.file}`}
          >
            {/* Two claims on one row, and the row's own copy block does not grow, so they travel as
                one group rather than being spread to either side of the leftover width. */}
            <span className="flex shrink-0 items-center gap-[var(--space-2)]">
              {proj.recommended && <span className="pill">{t("vision.recommended")}</span>}
              <State on={proj.onDisk}>{proj.onDisk ? t("pack.held") : t("pack.absent")}</State>
            </span>
          </Row>
        ))}
      </Section>

      {/* Held projectors are deliberately outside the byte totals above: a 0.4 GB mmproj with no
          language pack under it is not a model this machine can run, and the row that counts packs
          would stop meaning what it says. */}
      {model.vision.length > 0 && (
        <p className="mb-[var(--space-6)] text-fine text-faint">{t("vision.note")}</p>
      )}

      <Section label={t("issues")} description={t("issues.desc")}>
        {model.caveats.length === 0 && (
          <Row title={t("issues.none")}>
            <span />
          </Row>
        )}
        {model.caveats.map((caveat) => (
          <Row
            key={`${caveat.severity}-${caveat.condition}-${caveat.symptom}`}
            title={caveat.symptom}
            description={`${t("when", { condition: caveat.condition })} — ${t("workaround", {
              workaround: caveat.workaround,
            })}`}
          >
            {/* Severity is the card's own word, not a state of this machine, so it gets the plain
                chip rather than the one that means present or absent. */}
            <span className="pill shrink-0">{t(`severity.${caveat.severity}`)}</span>
          </Row>
        ))}
      </Section>

      <div className="flex flex-wrap items-center gap-[var(--space-4)]">
        <ExternalLink url={model.repoUrl} label={t("card")} locale={locale} />
        {/* The card's own date rather than today's: these numbers were read from the file on the day
            the pin was written, and that is the only date worth quoting. */}
        <span className="text-fine text-faint">
          {t("updated", { date: center.catalogUpdated })}
        </span>
      </div>
    </>
  );
}

function MachinePage({
  center,
  t,
  heldOf,
}: {
  center: KnowledgeCenter;
  t: T;
  heldOf: (m: CatalogModel) => string;
}) {
  const held = center.models.flatMap((m) => m.packs.filter((p) => p.onDisk));
  const heldBytes = held.reduce((sum, p) => sum + p.sizeBytes, 0);
  const packCount = center.models.reduce((n, m) => n + m.packs.length, 0);
  return (
    <>
      <Section label={t("engine")} description={t("engine.desc")}>
        <Row title={t("engine.pinned")} description={t("engine.pinned.desc")}>
          <span className="font-mono tabular-nums text-quiet text-fg">{center.engineTag}</span>
        </Row>
        <Row title={t("engine.state")} description={t("engine.state.desc")}>
          <State on={center.engineInstalled}>
            {center.engineInstalled ? t("engine.present") : t("engine.missing")}
          </State>
        </Row>
        <Row title={t("weightsDir")} description={center.weightsDir}>
          <span className="font-mono tabular-nums text-quiet text-fg">
            {t("heldCount", { held: held.length, total: packCount })}
          </span>
        </Row>
      </Section>

      <Section label={t("held")} description={t("held.desc", { gb: gb(heldBytes) })}>
        {center.models.map((m) => (
          <Row key={m.id} title={m.name} description={heldOf(m)}>
            <span className="font-mono tabular-nums text-quiet text-fg">{gb(m.heldBytes)} GB</span>
          </Row>
        ))}
      </Section>

      <p className="text-fine text-faint">{t("sizeNote")}</p>
    </>
  );
}
