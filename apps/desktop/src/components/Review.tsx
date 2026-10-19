import { Fragment, useEffect, useMemo, useRef, useState } from "react";

import { translate, type Locale } from "../i18n";
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
  const toggleFile = (path: string) => setExcept((prev) => ({ ...prev, [path]: !isFolded(path) }));

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
          files.map((file) => {
            const folded = isFolded(file.path);
            return (
              <section
                key={file.path}
                data-file={file.path}
                className="review-file border-b border-line/60"
              >
                {/* The header is the row the folded panel is made of, so it is the control that folds
                    and opens rather than a caption above one. */}
                <button
                  type="button"
                  onClick={() => toggleFile(file.path)}
                  aria-expanded={!folded}
                  className="review-file-head flex w-full items-center gap-[var(--space-2)] px-[var(--space-3)] py-[var(--space-2)] text-left"
                >
                  <Icon
                    name="chevronRight"
                    size={12}
                    className={`disclosure-icon shrink-0 ${folded ? "" : "rotate-90"}`}
                  />
                  <span className="insp-chip">
                    <Icon name="editFile" size={13} />
                  </span>
                  {/* The name is what a changed-file row is scanned for, so the directories above it are
                      the part allowed to run out of room. */}
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
                {!folded &&
                  blocks(file.hunks).map((block, h) => (
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
                            <span className="diff-text">{line.text}</span>
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
