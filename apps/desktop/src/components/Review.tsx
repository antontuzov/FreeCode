import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { translate, type Locale } from "../i18n";
import { ipc } from "../lib/ipc";
import { useNav } from "../state/nav";
import {
  changedFiles,
  reviewRows,
  useTranscript,
  type Hunk,
  type ReviewScope,
} from "../state/transcript";
import { Icon } from "./icons";

/**
 * One line of a hunk, with the file line it stands on.
 *
 * `n` is null for the line that says how much of the change the result budget held back, because it
 * names no line of the file at all. Otherwise the number is the old file's for a context or removed
 * line and the new file's for an added one, which is what lets a gutter hold one column and still be
 * read: the added block counts on from where the removed one left off, exactly as the reference's does.
 */
type Line = { kind: "ctx" | "add" | "del" | "held"; n: number | null; text: string };

/**
 * Number a hunk's lines.
 *
 * The host's `at` names the line the *change* opens on, which is the caption the model's copy reads
 * ("at line 142"), not the first line printed: a hunk starts two context lines earlier. So the counter
 * is set back by however many context lines lead the block, which is also what makes a change at the
 * file's own start number from 1 rather than from −1.
 */
function numberHunk(hunk: Hunk): Line[] {
  const raw = hunk.lines.split("\n");
  let lead = 0;
  while (lead < raw.length && raw[lead]?.startsWith(" ")) lead += 1;
  let old = hunk.at - lead;
  let next = old;
  return raw.map((line): Line => {
    if (line.startsWith("+")) {
      const n = next;
      next += 1;
      // The marker goes: the band behind the line and the number beside it both already say it, and
      // the reference's diff column shows neither sign.
      return { kind: "add", n, text: line.slice(1) };
    }
    if (line.startsWith("-")) {
      const n = old;
      old += 1;
      return { kind: "del", n, text: line.slice(1) };
    }
    if (line.startsWith("…")) return { kind: "held", n: null, text: line };
    const n = old;
    old += 1;
    next += 1;
    return { kind: "ctx", n, text: line.slice(1) };
  });
}

/**
 * Each of a file's hunks, with the stretch of the file the block before it left off printing.
 *
 * The reference puts "99 unmodified lines" between two blocks so a jump in the gutter reads as an
 * omission rather than as a deletion the count above missed; without it, a hunk ending on 220 followed
 * by one starting on 258 looks like 38 lines vanished. Both ends of the stretch are the host's own
 * numbers — the line the block stopped on and the line the next one starts on — so the figure is
 * measured, never guessed. A block whose tail the result budget held back ends at a line nobody knows,
 * so it announces no stretch: a wrong number here is worse than the missing one, because it is the
 * kind a reader checks.
 */
function blocks(hunks: Hunk[]): { lines: Line[]; gap: number | null }[] {
  const numbered = hunks.map(numberHunk);
  return numbered.map((lines, i) => {
    const prev = numbered[i - 1];
    if (!prev) return { lines, gap: null };
    const from = hunkEnd(prev);
    const to = hunkStart(lines);
    if (from === undefined || to === undefined) return { lines, gap: null };
    return { lines, gap: to - from > 1 ? to - from - 1 : null };
  });
}

/** The first line a block prints. A held line carries no number, so it cannot open one. */
function hunkStart(lines: Line[]): number | undefined {
  return lines.find((line) => line.n !== null)?.n ?? undefined;
}

/** The last line a block prints, or nothing when it cannot be trusted to have printed all of them. */
function hunkEnd(lines: Line[]): number | undefined {
  if (lines.some((line) => line.kind === "held")) return undefined;
  return [...lines].reverse().find((line) => line.n !== null)?.n ?? undefined;
}

/** Whether one printed line says what the find field asked for. Compared lower-case on both sides. */
function says(text: string, needle: string): boolean {
  return needle !== "" && text.toLowerCase().includes(needle);
}

/**
 * A diff line with its matches painted.
 *
 * The text stays the host's own — the query never rewrites what the run wrote, only what stands out
 * in it — so the same line the model was handed is still the line on screen. Split rather than
 * replaced, because a `dangerouslySetInnerHTML` of a query the user typed is the bug, not the fix.
 */
function marked(text: string, needle: string): ReactNode {
  if (needle === "") return text;
  const lower = text.toLowerCase();
  const out: ReactNode[] = [];
  let from = 0;
  for (let at = lower.indexOf(needle); at >= 0; at = lower.indexOf(needle, from)) {
    out.push(text.slice(from, at));
    out.push(
      <mark key={at} className="diff-mark">
        {text.slice(at, at + needle.length)}
      </mark>,
    );
    from = at + needle.length;
  }
  out.push(text.slice(from));
  return out;
}

/**
 * The run's change, in the column the reference gives it (shot 06), opened from the pill that counted
 * it.
 *
 * It replaces the inspector rather than covering the answer, because a diff needs a code line's width
 * and the transcript column cannot spare one without shrinking the answer that caused the change. The
 * hunks are the host's own text — the same lines the model was handed — so the review cannot describe
 * a different change than the answer asked for.
 */
export function Review({ locale }: { locale: Locale }) {
  const rows = useTranscript((s) => s.rows);
  const showInspector = useNav((s) => s.showInspector);
  // `?scope=all` seeds the second scope the way `?q=` seeds the settings search: the switch is a press
  // the native window cannot be given, and a control nobody has seen rendered is a guess.
  const [scope, setScope] = useState<ReviewScope>(
    new URLSearchParams(window.location.search).get("scope") === "all" ? "all" : "lastTurn",
  );
  // A file the run wrote twice is one section with two hunks in it, which is what a diff reader
  // expects; two sections of the same path would read as a bug in the count above them.
  const files = useMemo(
    () => changedFiles(reviewRows(rows, scope)).filter((file) => file.hunks.length > 0),
    [rows, scope],
  );
  // The headline counts the sections under it rather than the run's ledger, which is the difference
  // between a panel that answers its own question and one whose two halves describe different spans:
  // the ledger is one run's, and the last turn's rows and the whole task's are not the same set.
  const totals = files.reduce(
    (acc, file) => ({ added: acc.added + file.added, removed: acc.removed + file.removed }),
    { added: 0, removed: 0 },
  );
  const reviewFile = useNav((s) => s.reviewFile);
  const body = useRef<HTMLDivElement>(null);
  // The sections whose hunks are put away, leaving the one line that names the file. Shot 06's header
  // carries this as a control, and it is the panel's own answer to a long run: six files of hunks read
  // as one long scroll, while six lines read as the set the change touched.
  //
  // A default and the exceptions to it, rather than a list of folded paths, because the list this panel
  // folds is not on screen when the deep link is read: `?outline=1` arrives before the seeded rows are
  // pushed, so a set built from the first render's files would be built from nothing.
  const [foldAll, setFoldAll] = useState(
    () => new URLSearchParams(window.location.search).get("outline") === "1",
  );
  const [except, setExcept] = useState<Record<string, boolean>>({});
  const isFolded = (path: string) => except[path] ?? foldAll;
  const allFolded = files.length > 0 && files.every((file) => isFolded(file.path));
  const toggleAll = () => {
    setFoldAll(!foldAll);
    setExcept({});
  };

  // A press of a row's open control that the desktop refused, kept per path so the sentence lands
  // under the file it is about and a later press that works clears it. The panel has just told the
  // user this file changed; asking for a press and answering it with silence is the failure here.
  const [refused, setRefused] = useState<Record<string, string>>({});
  const open = async (path: string) => {
    setRefused((held) => {
      const rest = { ...held };
      delete rest[path];
      return rest;
    });
    try {
      await ipc.openFile(path);
    } catch (why) {
      setRefused((held) => ({ ...held, [path]: String(why) }));
    }
  };

  // The reference's other header control: a query over the change itself. A run that touched six
  // files puts a wall of hunks in this column, and what a reader does with a wall is look something
  // in it for a line.
  //
  // Seeded from the URL for the reason `?outline=1` is — the field opens on a click the native
  // window cannot be given, and a control nobody has seen rendered is a guess.
  const [find, setFind] = useState(
    () => new URLSearchParams(window.location.search).get("find") ?? "",
  );
  const [findOpen, setFindOpen] = useState(() => find !== "");
  const needle = find.trim().toLowerCase();
  const findField = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (findOpen) findField.current?.focus();
  }, [findOpen]);

  // One pass per rendered file: the blocks it prints, and how many of their lines the query says.
  // Counted here rather than in the row because a file's own fold and the total beside the field are
  // the same number, and two places working it out are two places that can disagree.
  const sections = useMemo(
    () =>
      files.map((file) => {
        const printed = blocks(file.hunks);
        const hits = printed.reduce(
          (n, block) => n + block.lines.filter((line) => says(line.text, needle)).length,
          0,
        );
        return { file, printed, hits };
      }),
    [files, needle],
  );
  const matches = sections.reduce((n, section) => n + section.hits, 0);

  // A query folds the files it found nothing in, and a fold the reader cannot open is a dead control,
  // so the press still answers: an explicit `except` outranks both the outline switch and the query.
  const foldedBy = (path: string, hits: number) =>
    except[path] ?? (foldAll || (needle !== "" && hits === 0));
  const toggleFile = (path: string, hits: number) =>
    setExcept((prev) => ({ ...prev, [path]: !foldedBy(path, hits) }));

  // The file an edit row named, once the column is holding the change. Matching on the path rather
  // than on an index because the transcript and this list are two orderings of one set: the row was
  // written when the tool ran, and a file the run wrote twice folds both of its hunks under one path.
  useEffect(() => {
    if (!reviewFile || !body.current) return;
    // A jump is a request to read the change, so it opens what it lands on. The unfold grows the
    // section downward from the line it folded to, which leaves the top edge — where the scroll is
    // aiming — exactly where it was put.
    setExcept((prev) => (prev[reviewFile] === false ? prev : { ...prev, [reviewFile]: false }));
    const hit = [...body.current.querySelectorAll<HTMLElement>("[data-file]")].find(
      (section) => section.dataset.file === reviewFile,
    );
    if (!hit) return;
    hit.scrollIntoView({ block: "start" });
    hit.dataset.flash = "true";
    const id = window.setTimeout(() => delete hit.dataset.flash, 1400);
    return () => window.clearTimeout(id);
  }, [reviewFile, files]);

  return (
    <aside
      aria-label={translate(locale, "review.title")}
      className="flex min-h-0 flex-1 flex-col overflow-hidden border-l border-line bg-bg"
    >
      <header className="flex shrink-0 items-center gap-[var(--space-2)] border-b border-line px-[var(--space-3)] py-[var(--space-2)]">
        <Icon name="review" size={14} />
        <span className="text-quiet">{translate(locale, "review.title")}</span>
        <span className="flex-1" />
        {files.length > 0 && (
          <button
            type="button"
            onClick={() => setFindOpen(!findOpen)}
            aria-expanded={findOpen}
            className="icon-btn"
            aria-label={translate(locale, "review.find")}
            title={translate(locale, "review.find")}
          >
            <Icon name="search" size={13} />
          </button>
        )}
        {files.length > 0 && (
          <button
            type="button"
            onClick={toggleAll}
            aria-pressed={allFolded}
            className="icon-btn"
            aria-label={translate(locale, "review.outline")}
            title={translate(locale, "review.outline")}
          >
            <Icon name="outline" size={13} />
          </button>
        )}
        <button
          type="button"
          onClick={showInspector}
          className="icon-btn"
          aria-label={translate(locale, "review.close")}
          title={translate(locale, "review.close")}
        >
          <Icon name="close" size={13} />
        </button>
      </header>

      {findOpen && files.length > 0 && (
        <div className="review-find-row">
          <label className="field review-find">
            <Icon name="search" size={14} />
            <input
              ref={findField}
              className="field-input"
              type="search"
              value={find}
              placeholder={translate(locale, "review.find")}
              aria-label={translate(locale, "review.find")}
              onChange={(e) => setFind(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Escape") return;
                // One press, one thing. The window's own Esc handler closes a review panel, so
                // without this the press that empties the field also throws the change out of the
                // column, and the second press the user has to make is one they did not ask for.
                e.stopPropagation();
                setFind("");
                setFindOpen(false);
              }}
            />
          </label>
          {/* The count is the field's own answer, so it sits beside it rather than in the header's
              totals, which count lines added and removed and would read as a third number. */}
          {needle === "" ? null : matches === 0 ? (
            <span className="text-fine text-faint">
              {translate(locale, "review.noMatch", { query: find.trim() })}
            </span>
          ) : (
            <span className="text-fine text-muted tabular-nums">
              {translate(locale, matches === 1 ? "review.matchOne" : "review.matchMany", {
                count: matches,
              })}
            </span>
          )}
        </div>
      )}

      {files.length > 0 && (
        <div className="flex shrink-0 items-center gap-[var(--space-3)] border-b border-line px-[var(--space-4)] py-[var(--space-2)] text-fine text-muted">
          {/* The reference's first control in this column: which span the numbers beside it count. */}
          <label className="review-scope">
            <select
              aria-label={translate(locale, "review.scope")}
              value={scope}
              onChange={(e) => setScope(e.target.value as ReviewScope)}
            >
              <option value="lastTurn">{translate(locale, "review.scope.lastTurn")}</option>
              <option value="all">{translate(locale, "review.scope.all")}</option>
            </select>
            <Icon name="chevronDown" size={12} />
          </label>
          <span className="font-mono tabular-nums">
            <span className="text-success">+{totals.added}</span>{" "}
            <span className="text-danger">−{totals.removed}</span>
          </span>
        </div>
      )}

      <div ref={body} className="min-h-0 flex-1 overflow-y-auto">
        {files.length === 0 ? (
          <p className="px-[var(--space-4)] py-[var(--space-3)] text-fine text-faint">
            {translate(locale, "review.empty")}
          </p>
        ) : (
          sections.map(({ file, printed, hits }) => {
            const folded = foldedBy(file.path, hits);
            const why = refused[file.path];
            return (
              <section
                key={file.path}
                data-file={file.path}
                className="review-file border-b border-line/60"
              >
                {/* The header is the row the folded panel is made of, so it is the control that folds
                    and opens rather than a caption above one. The open control sits beside it rather
                    than inside it, because a button may not hold another. */}
                <div className="review-file-head flex w-full items-center">
                  <button
                    type="button"
                    onClick={() => toggleFile(file.path, hits)}
                    aria-expanded={!folded}
                    className="flex min-w-0 flex-1 items-center gap-[var(--space-2)] px-[var(--space-3)] py-[var(--space-2)] text-left"
                  >
                    <Icon
                      name="chevronRight"
                      size={12}
                      className={`disclosure-icon shrink-0 ${folded ? "" : "rotate-90"}`}
                    />
                    <span className="insp-chip">
                      {/* A file the run made is marked as one, because the create is the difference
                          between "the agent changed my file" and "the agent wrote this" — and the two
                          counts alone do not say it, since an edit that only added lines reads +N −0
                          too. */}
                      <Icon name={file.created ? "createFile" : "editFile"} size={13} />
                    </span>
                    {/* The name is what a changed-file row is scanned for, so the directories above it
                        are the part allowed to run out of room. */}
                    <span className="flex min-w-0 flex-1 items-baseline text-quiet">
                      <span className="truncate text-faint">
                        {file.path.slice(0, file.path.lastIndexOf("/") + 1)}
                      </span>
                      <span className="shrink-0">
                        {file.path.slice(file.path.lastIndexOf("/") + 1)}
                      </span>
                    </span>
                    <span className="shrink-0 font-mono text-micro tabular-nums">
                      <span className="text-success">+{file.added}</span>{" "}
                      <span className="text-danger">−{file.removed}</span>
                    </span>
                  </button>
                  <button
                    type="button"
                    onClick={() => void open(file.path)}
                    className="icon-btn mr-[var(--space-2)] shrink-0"
                    aria-label={translate(locale, "review.open")}
                    title={translate(locale, "review.open")}
                  >
                    <Icon name="external" size={13} />
                  </button>
                </div>
                {why && (
                  <p className="review-open-failed">
                    {translate(locale, "review.openFailed", { why })}
                  </p>
                )}
                {!folded &&
                  printed.map((block, h) => (
                    <Fragment key={h}>
                      {block.gap !== null && (
                        <p className="diff-gap">
                          {translate(
                            locale,
                            block.gap === 1 ? "review.unmodifiedOne" : "review.unmodifiedMany",
                            { count: block.gap },
                          )}
                        </p>
                      )}
                      <div className="review-hunk">
                        {block.lines.map((line, i) => (
                          <div key={i} className={`diff-line diff-${line.kind}`}>
                            <span className="diff-n">{line.n ?? ""}</span>
                            <span className="diff-text">{marked(line.text, needle)}</span>
                          </div>
                        ))}
                      </div>
                    </Fragment>
                  ))}
              </section>
            );
          })
        )}
      </div>
    </aside>
  );
}
