import { useMemo } from "react";

import { translate, type Locale } from "../i18n";
import { useNav } from "../state/nav";
import { changedFiles, useTranscript, type Hunk } from "../state/transcript";
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
  const runDiff = useTranscript((s) => s.runDiff);
  const showInspector = useNav((s) => s.showInspector);
  // A file the run wrote twice is one section with two hunks in it, which is what a diff reader
  // expects; two sections of the same path would read as a bug in the count above them.
  const files = useMemo(() => changedFiles(rows).filter((file) => file.hunks.length > 0), [rows]);

  return (
    <aside
      aria-label={translate(locale, "review.title")}
      className="flex h-full w-[var(--review-width)] shrink-0 flex-col overflow-hidden border-l border-line bg-bg"
    >
      <header className="flex shrink-0 items-center gap-[var(--space-2)] border-b border-line px-[var(--space-3)] py-[var(--space-2)]">
        <Icon name="review" size={14} />
        <span className="text-quiet">{translate(locale, "review.title")}</span>
        <span className="flex-1" />
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

      {runDiff && (
        <div className="flex shrink-0 items-center gap-[var(--space-3)] border-b border-line px-[var(--space-4)] py-[var(--space-2)] text-fine text-muted">
          {/* The same words the inspector's own header uses for the same span of time, because the two
              columns report one run to either side of the same press. */}
          <span>{translate(locale, "inspector.run")}</span>
          <span className="font-mono tabular-nums">
            <span className="text-success">+{runDiff.added}</span>{" "}
            <span className="text-danger">−{runDiff.removed}</span>
          </span>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {files.length === 0 ? (
          <p className="px-[var(--space-4)] py-[var(--space-3)] text-fine text-faint">
            {translate(locale, "review.empty")}
          </p>
        ) : (
          files.map((file) => (
            <section key={file.path} className="border-b border-line/60">
              <div className="flex items-center gap-[var(--space-2)] px-[var(--space-3)] py-[var(--space-2)]">
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
              </div>
              {file.hunks.map((hunk, h) => (
                <div key={h} className="review-hunk">
                  {numberHunk(hunk).map((line, i) => (
                    <div key={i} className={`diff-line diff-${line.kind}`}>
                      <span className="diff-n">{line.n ?? ""}</span>
                      <span className="diff-text">{line.text}</span>
                    </div>
                  ))}
                </div>
              ))}
            </section>
          ))
        )}
      </div>
    </aside>
  );
}
