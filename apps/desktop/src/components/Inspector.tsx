import { useEffect, useMemo, useState } from "react";

import { translate, type Locale } from "../i18n";
import type { RunReport } from "../lib/ipc";
import {
  changedFiles,
  readSubjects,
  reviewRows,
  stepLabel,
  usedCapabilities,
  useTranscript,
  type DiffStat,
  type StepKind,
} from "../state/transcript";
import { Icon, type IconName } from "./icons";

type Group = {
  key: string;
  /** The glyph a row in this group gets when it has no mark of its own. The reference puts a chip
   * behind each row, so a group with items always draws one; a group that had no glyph of its own
   * would draw a bare word. */
  icon: IconName;
  /** A subagent is a who rather than a what, so its chip closes into a disc the way the footer's
   * avatar does. Everything else is a file, a tool or a page, and those are square. */
  round?: boolean;
  /** Whether the header counts its rows in a chip, which only the capability group does: the reference
   * draws the chip there and nowhere else, and the number it holds is the length of the list under it,
   * so the folded heading and the open rows are one answer. The other groups name paths and addresses,
   * where reading the rows *is* counting them. */
  counted?: boolean;
};

// The groups the reference inspector shows, in order: first what the run changed, then the five it
// draws under it.
const GROUPS: Group[] = [
  { key: "files", icon: "editFile" },
  { key: "subagents", icon: "user", round: true },
  { key: "skills", icon: "skills", counted: true },
  { key: "artifact", icon: "artifact" },
  { key: "webReading", icon: "webReading" },
  // The document glyph rather than a stack of them, because every row in this group is one file the
  // run opened: the pair of path lists has to read as *read* against *changed* at a glance, which is
  // what the two glyphs' shapes are for.
  { key: "source", icon: "readFile" },
];

type Item = { label: string; meta?: string; diff?: DiffStat; icon?: IconName; path?: boolean };

/**
 * The sites this build can name, and the sign each one gets on a row that says it was read. Only the
 * hosts the app itself points at have a mark — the hub its catalog downloads from, the repository its
 * status card links to, and the encyclopedia its one web tool searches. A page from anywhere else keeps
 * the group's glyph, because a made-up look-alike of a site the app cannot reach is worse than a
 * generic one.
 */
const HOST_MARKS: [string, IconName][] = [
  ["huggingface.co", "huggingFace"],
  ["github.com", "github"],
  ["wikipedia.org", "wikipedia"],
];

/** The mark for a host, which may be bare (`huggingface.co`) or carry a subdomain (`en.wikipedia.org`). */
function hostMark(host: string): IconName | undefined {
  const bare = host.toLowerCase();
  return HOST_MARKS.find(([name]) => bare === name || bare.endsWith(`.${name}`))?.[1];
}

/**
 * The capability rows wear the glyph of the call they name, which is the shape the panel already uses
 * to tell a read from an edit from a page: six identical chips under six different labels would say the
 * group holds one kind of thing, when the point of listing them is that the run had several. A kind
 * with no mark of its own — `tool`, the call the host names without giving it a row shape — keeps the
 * group's prompt glyph, which is the mark that means "a capability" on its own.
 */
const CAPABILITY_ICONS: Partial<Record<StepKind, IconName>> = {
  readFile: "readFile",
  editFile: "editFile",
  searchContent: "searchContent",
  terminal: "terminal",
  webSearch: "search",
  webReading: "webReading",
};

/**
 * `?collapsed=<keys>` starts named sections folded — the five groups by key, and the run's own numbers
 * as `run`. Dev-only, like every other deep link here: the fold is a control the reader has on screen,
 * and this is the same press arriving from the URL, which is what lets a capture reach the sections a
 * full column pushes below the window.
 */
const SEEDED_FOLDED = new Set(
  (new URLSearchParams(window.location.search).get("collapsed") ?? "").split(","),
);

/**
 * `?fixture=1` only, for the two groups no loop in this build writes rows for yet — see the note in
 * state/transcript.ts about invented run output. The four a run *can* produce — `files`, `source`,
 * `webReading` and `skills` — are read off the transcript in every mode, so what the panel lists and
 * what the rows above it report stay one answer rather than two.
 */
const FIXTURE_ITEMS: Record<string, Item[]> = {
  subagents: [{ label: "Explore · llama.cpp", meta: "stopped" }, { label: "Audit engine claims" }],
  artifact: [{ label: "README.md" }],
};

function SectionHeader({
  open,
  onToggle,
  label,
  count,
}: {
  open: boolean;
  onToggle: () => void;
  label: string;
  /** The reference's header chip, drawn at the row's right edge: a count that holds while the group
   * is folded, in the same accent pill the composer's mode switch wears. */
  count?: string;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      className="flex w-full items-center gap-2 px-[var(--space-4)] py-[var(--space-3)] text-muted hover:bg-hover hover:text-fg"
    >
      {/* The chevron follows the name, as it does in the reference, which leaves the name itself at
       * the column's left edge — where the chips under it start too. A leading chevron would hang
       * every heading one glyph to the left of its own rows. */}
      <span className="text-quiet">{label}</span>
      <Icon
        name="chevronDown"
        size={13}
        className={`disclosure-icon shrink-0 ${open ? "" : "-rotate-90"}`}
      />
      {count && <span className="pill ml-auto shrink-0 px-2 py-0 text-micro">{count}</span>}
    </button>
  );
}

/** A path split where it is worth keeping: the file name is what both of the column's path lists are
 * scanned for, so the directories above it are the part allowed to run out of room. */
function FileLabel({ path }: { path: string }) {
  const dir = path.slice(0, path.lastIndexOf("/") + 1);
  return (
    <span className="flex min-w-0 flex-1 items-baseline">
      <span className="truncate text-faint">{dir}</span>
      <span className="shrink-0">{path.slice(dir.length)}</span>
    </span>
  );
}

/**
 * The address with its scheme cut off. This column truncates from the right, so `https://` on the front
 * is what costs the reader the part that names the page — and the page an agent read is chosen by its
 * host and its path, which is what remains.
 */
function page(url: string): string {
  return url.replace(/^https?:\/\//, "");
}

function Section({ group, items, locale }: { group: Group; items: Item[]; locale: Locale }) {
  const label = translate(locale, `inspector.${group.key}`);
  const folded = SEEDED_FOLDED.has(group.key);
  // A group starts open when it has rows and closed when it does not. Most of this build's run reports
  // no skills, pages or subagents — the loops that would produce them are not wired, so under a real
  // answer those groups are empty at once — and a column of open headings each followed by their own
  // apology is a panel that advertises what is missing. The sentence is one press away either way, so
  // closing says nothing that opening hides.
  const [open, setOpen] = useState(items.length > 0 && !folded);
  const has = items.length > 0;
  // A run that starts editing mid-answer fills this group from nothing, and a count the user has to
  // press open is a count they will not know to look for. A group the URL folded stays folded: the
  // rows arriving is the thing the capture came to see *past*, not a reason to undo the press.
  useEffect(() => {
    if (has && !folded) setOpen(true);
  }, [has, folded]);

  return (
    <section className="border-b border-line/60">
      <SectionHeader
        open={open}
        onToggle={() => setOpen((v) => !v)}
        label={label}
        count={
          group.counted && has
            ? translate(locale, "inspector.skills.count", { count: items.length })
            : undefined
        }
      />
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
                className="flex items-center gap-[var(--space-3)] border-b border-dashed border-line/60 py-[var(--space-2)] text-quiet text-fg last:border-0"
              >
                <span className={`insp-chip ${group.round ? "round" : ""}`}>
                  <Icon name={item.icon ?? group.icon} size={14} />
                </span>
                {item.diff || item.path ? (
                  <FileLabel path={item.label} />
                ) : (
                  <span className="min-w-0 flex-1 truncate">{item.label}</span>
                )}
                {item.diff && (
                  <span className="shrink-0 font-mono text-micro tabular-nums">
                    <span className="text-success">+{item.diff.added}</span>{" "}
                    <span className="text-danger">−{item.diff.removed}</span>
                  </span>
                )}
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

/** What the engine never reported, rendered the way the context meter renders it: a gap, not a
 * number. `null` is a stream that ended before its closing frame, and a zero would claim the run
 * generated nothing at all. */
const UNMEASURED = "—";
const count = (value: number | null) => (value === null ? UNMEASURED : String(value));
const rate = (value: number | null) => (value === null ? UNMEASURED : `${value.toFixed(1)} tok/s`);

/**
 * The run's own timings, at the head of a column named for the run.
 *
 * Every number here is one somebody measured: the rates and counts come from the `timings` object
 * the fork sends beside its usage frame, the wait from the host's wall clock — which is the longer
 * of the two, since it also covers the round trip and the deltas crossing IPC. A stream the user
 * stopped ends before that object exists, and the rows it leaves keep their labels and hold an em
 * dash: the run is still the one they waited through, and it is not nothing.
 */
function RunSection({ locale, run }: { locale: Locale; run: RunReport }) {
  const [open, setOpen] = useState(!SEEDED_FOLDED.has("run"));
  const running = useTranscript((s) => s.running);
  const rows: [string, string][] = [
    [translate(locale, "inspector.run.generation"), rate(run.generatedPerSecond)],
    [translate(locale, "inspector.run.prompt"), rate(run.promptPerSecond)],
    [
      translate(locale, "inspector.run.tokens"),
      translate(locale, "inspector.run.tokensValue", {
        generated: count(run.generatedTokens),
        prompt: count(run.promptTokens),
      }),
    ],
    [
      translate(locale, "inspector.run.cache"),
      translate(locale, "inspector.run.cacheValue", {
        cached: count(run.cachedTokens),
        prompt: count(run.promptTokens),
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

/** What the column says while this window has had no run at all: one line, and the five groups below
 * it. A run the user stopped is not this state — it has a wait to report, and rows that say the
 * engine never got to time the rest. */
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
  const rows = useTranscript((s) => s.rows);
  const lastRun = useTranscript((s) => s.lastRun);
  const run = fixture ? FIXTURE_RUN : lastRun;
  // The changed files come off the transcript's own rows, in both modes: an edit row and a ledger row
  // are the host's answer to one `Edit`, so listing them from the rows keeps this group and the pill
  // above the composer reading the same run. The rows are read from the last prompt down for the same
  // reason — this column reports one run, and a conversation's earlier turns are not part of it.
  const files = useMemo<Item[]>(
    () =>
      changedFiles(reviewRows(rows, "lastTurn")).map(({ path, added, removed }) => ({
        label: path,
        diff: { added, removed },
      })),
    [rows],
  );
  // The two read groups come off the same span of rows as the files, for the same reason: `fetch_url`
  // writes the address it opened into the row it leaves, and `read_file` the path it brought into the
  // next round, so a group that listed anything else would tell the reader about a run the transcript
  // above does not describe. What a *source* is here is the run's own reading rather than the files a
  // user attached: this build has no attachment path — the composer's add control says so on hover —
  // so the honest occupant of the reference's Source group is what the run opened to answer.
  const pages = useMemo<Item[]>(
    () =>
      readSubjects(reviewRows(rows, "lastTurn"), "webReading").map((url) => {
        const label = page(url);
        return { label, icon: hostMark(label.split("/")[0] ?? "") };
      }),
    [rows],
  );
  const sources = useMemo<Item[]>(
    () =>
      readSubjects(reviewRows(rows, "lastTurn"), "readFile").map((path) => ({
        label: path,
        path: true,
      })),
    [rows],
  );
  // The capabilities are the same derivation over the same span, named by the row's own label: what
  // "Skills & MCP" holds here is what this build's agent can be handed, which is the tool registry's
  // vocabulary the transcript already speaks — see `Extensions.tsx`.
  const capabilities = useMemo<Item[]>(
    () =>
      usedCapabilities(reviewRows(rows, "lastTurn")).map((step) => ({
        label: stepLabel(locale, step),
        icon: CAPABILITY_ICONS[step.kind],
      })),
    [rows, locale],
  );
  // The scripted groups fill in under the row-derived ones, which a fixture window therefore still
  // sees — and a real run does not, because it has no scripted rows to fill them from.
  const groups = useMemo<Record<string, Item[]>>(
    () => ({
      ...(fixture ? FIXTURE_ITEMS : {}),
      files,
      skills: capabilities,
      source: sources,
      webReading: pages,
    }),
    [fixture, files, capabilities, sources, pages],
  );

  return (
    <aside
      aria-label={translate(locale, "inspector.title")}
      className="flex min-h-0 flex-1 flex-col overflow-y-auto border-l border-line bg-bg"
    >
      {run ? <RunSection locale={locale} run={run} /> : <UnmeasuredSection locale={locale} />}
      {GROUPS.map((group) => (
        <Section key={group.key} group={group} items={groups[group.key] ?? []} locale={locale} />
      ))}
    </aside>
  );
}
