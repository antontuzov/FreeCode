import { useState } from "react";

import { translate, type Locale } from "../i18n";
import type { RunReport } from "../lib/ipc";
import { useTranscript } from "../state/transcript";
import { Icon, type IconName } from "./icons";

type Group = { key: string; icon: IconName };

// The five groups the reference inspector shows, in order.
const GROUPS: Group[] = [
  { key: "subagents", icon: "subagents" },
  { key: "skills", icon: "skills" },
  { key: "artifact", icon: "artifact" },
  { key: "webReading", icon: "webReading" },
  { key: "source", icon: "source" },
];

type Item = { label: string; meta?: string; icon?: IconName };

/** `?fixture=1` only — see the note in state/transcript.ts about invented run output. */
const FIXTURE_ITEMS: Record<string, Item[]> = {
  subagents: [{ label: "Explore · llama.cpp", meta: "stopped" }, { label: "Audit engine claims" }],
  skills: [
    { label: "verify-smollm-studio-natively" },
    { label: "tauri-desktop-native-verification" },
  ],
  artifact: [{ label: "README.md", icon: "artifact" }],
  webReading: [{ label: "huggingface.co/PrismML-Eng", icon: "webReading" }],
  source: [{ label: "catalog/models.toml", icon: "artifact" }],
};

function SectionHeader({
  open,
  onToggle,
  label,
}: {
  open: boolean;
  onToggle: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="flex w-full items-center gap-2 px-[var(--space-4)] py-[var(--space-3)] text-muted hover:bg-hover hover:text-fg"
    >
      <Icon
        name="chevronDown"
        size={13}
        className={`disclosure-icon ${open ? "" : "-rotate-90"}`}
      />
      <span className="text-quiet">{label}</span>
    </button>
  );
}

function Section({ group, items, locale }: { group: Group; items: Item[]; locale: Locale }) {
  const [open, setOpen] = useState(true);
  const label = translate(locale, `inspector.${group.key}`);

  return (
    <section className="border-b border-line/60">
      <SectionHeader open={open} onToggle={() => setOpen((v) => !v)} label={label} />
      {open && (
        <ul className="disclosure-panel px-[var(--space-4)] pb-[var(--space-3)]">
          {items.length === 0 ? (
            <li className="py-1 text-fine text-faint">
              {translate(locale, "inspector.empty", { group: label })}
            </li>
          ) : (
            items.map((item) => (
              <li
                key={item.label}
                className="flex items-center gap-2 border-b border-dashed border-line/60 py-2 text-quiet text-fg last:border-0"
              >
                {item.icon ? <Icon name={item.icon} size={14} /> : null}
                <span className="min-w-0 flex-1 truncate">{item.label}</span>
                {item.meta && <span className="shrink-0 text-micro text-faint">{item.meta}</span>}
              </li>
            ))
          )}
        </ul>
      )}
    </section>
  );
}

/** How long the host waited, in the unit that keeps two digits meaningful. */
function elapsed(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/**
 * The run's own timings, at the head of a column named for the run.
 *
 * Every number here is one somebody measured: the rates and counts come from the `timings` object
 * the fork sends beside its usage frame, the wait from the host's wall clock — which is the longer
 * of the two, since it also covers the round trip and the deltas crossing IPC. A run whose stream
 * never answered the usage ask reports nothing, because a rate divided out of token counts nobody
 * timed is a number that gets believed.
 */
function RunSection({ locale, run }: { locale: Locale; run: RunReport }) {
  const [open, setOpen] = useState(true);
  const running = useTranscript((s) => s.running);
  const rows: [string, string][] = [
    [translate(locale, "inspector.run.generation"), `${run.generatedPerSecond.toFixed(1)} tok/s`],
    [translate(locale, "inspector.run.prompt"), `${run.promptPerSecond.toFixed(1)} tok/s`],
    [
      translate(locale, "inspector.run.tokens"),
      translate(locale, "inspector.run.tokensValue", {
        generated: run.generatedTokens,
        prompt: run.promptTokens,
      }),
    ],
    [
      translate(locale, "inspector.run.cache"),
      translate(locale, "inspector.run.cacheValue", {
        cached: run.cachedTokens,
        prompt: run.promptTokens,
      }),
    ],
    [translate(locale, "inspector.run.elapsed"), elapsed(run.elapsedMs)],
  ];

  return (
    <section className="border-b border-line">
      <SectionHeader
        open={open}
        onToggle={() => setOpen((v) => !v)}
        label={translate(locale, "inspector.run")}
      />
      {open && (
        <dl className="disclosure-panel flex flex-col gap-1 px-[var(--space-4)] pb-[var(--space-3)] text-fine text-muted">
          {rows.map(([label, value]) => (
            <div key={label} className="flex items-baseline justify-between gap-3">
              <dt className="shrink-0">{label}</dt>
              <dd className="truncate font-mono tabular-nums text-fg">
                {running ? (
                  // The next run's numbers are not this run's, so the row keeps its label and holds
                  // the place its value will land in rather than quoting the last answer.
                  <span className="skeleton-line ml-auto block h-3 w-16" />
                ) : (
                  value
                )}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </section>
  );
}

/** What the column says before any run has timed itself: one line, and the five groups below it. */
function UnmeasuredSection({ locale }: { locale: Locale }) {
  const [open, setOpen] = useState(true);
  return (
    <section className="border-b border-line">
      <SectionHeader
        open={open}
        onToggle={() => setOpen((v) => !v)}
        label={translate(locale, "inspector.run")}
      />
      {open && (
        <div className="disclosure-panel px-[var(--space-4)] pb-[var(--space-3)]">
          <p className="py-1 text-fine text-faint">{translate(locale, "inspector.run.none")}</p>
        </div>
      )}
    </section>
  );
}

/** The scripted report `?fixture=1` shows, so the layout still renders with the numbers it is
 * compared against. The rates are the ones this machine's Metal build measured on a real run. */
const FIXTURE_RUN: RunReport = {
  generatedTokens: 168,
  promptTokens: 15,
  cachedTokens: 14,
  generatedPerSecond: 19.9,
  promptPerSecond: 155.6,
  elapsedMs: 8509,
};

export function Inspector({ locale, fixture }: { locale: Locale; fixture: boolean }) {
  const items = fixture ? FIXTURE_ITEMS : {};
  const lastRun = useTranscript((s) => s.lastRun);
  const run = fixture ? FIXTURE_RUN : lastRun;

  return (
    <aside
      aria-label={translate(locale, "inspector.title")}
      className="flex h-full w-[var(--inspector-width)] shrink-0 flex-col overflow-y-auto border-l border-line bg-bg"
    >
      {run ? <RunSection locale={locale} run={run} /> : <UnmeasuredSection locale={locale} />}
      {GROUPS.map((group) => (
        <Section key={group.key} group={group} items={items[group.key] ?? []} locale={locale} />
      ))}
    </aside>
  );
}
