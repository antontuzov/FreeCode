import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { translate, type Locale } from "../i18n";
import {
  ctaLabel,
  isTurn,
  stepLabel,
  stepText,
  useTranscript,
  type Row,
  type Step,
  type Turn,
} from "../state/transcript";
import { Answer } from "./Answer";
import { BrandMark } from "./BrandMark";
import { Composer } from "./Composer";
import { ExternalLink } from "./ExternalLink";
import { Icon } from "./icons";

function StatusMark({ step }: { step: Step }) {
  if (step.status === "running") return <span className="dot-running" aria-hidden />;
  // A notice is worth reading and not worth alarming over: same glyph, in the quiet colour the
  // answer around it is written in.
  if (step.notice) return <Icon name="warning" size={13} className="text-muted" />;
  if (step.status === "error" || step.status === "stopped")
    return <Icon name="warning" size={13} className="text-danger" />;
  return <Icon name="review" size={13} className="text-accent-ink" />;
}

/**
 * A refusal: what is wrong in one sentence, the one action that clears it, and the evidence folded
 * underneath. The row used to be a line of `· {title} · {detail}` with a bare URL at the end, which
 * reads like a panic log — so the headline carries the row, the reason sits under it at full width,
 * and paths and key names only appear when the user asks for the technical view.
 */
function RefusalRow({ step, locale }: { step: Step; locale: Locale }) {
  return (
    <li className="refusal-card row-enter">
      <div className="flex items-start gap-2">
        <span className="mt-0.5 shrink-0">
          <StatusMark step={step} />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-quiet font-medium text-fg">{stepLabel(locale, step)}</p>
          <p className="mt-1 whitespace-pre-wrap break-words text-fine text-muted">
            {stepText(locale, step)}
          </p>
          {step.cta && (
            <div className="mt-2">
              <ExternalLink
                url={step.cta.url}
                label={ctaLabel(locale, step.cta.code)}
                locale={locale}
              />
            </div>
          )}
          {step.technical && (
            <details className="mt-2">
              <summary className="cursor-pointer text-micro text-faint">
                {translate(locale, "transcript.refusal.technical")}
              </summary>
              <p className="disclosure-panel mt-1 break-words font-mono text-micro text-muted">
                {step.technical}
              </p>
            </details>
          )}
        </div>
      </div>
    </li>
  );
}

function StepRow({ step, locale }: { step: Step; locale: Locale }) {
  const [open, setOpen] = useState(true);
  const hasChildren = !!step.children?.length;
  // A notice's line is its catalog sentence with the run's own numbers in it; every other row's line
  // is the detail the host gave it.
  const text = stepText(locale, step);

  if (step.kind === "contextCompressed") {
    return (
      <div className="my-[var(--space-4)] flex items-center gap-3 text-faint">
        <span className="h-px flex-1 bg-line" />
        <Icon name="context" size={14} />
        <span className="text-quiet">{stepLabel(locale, step)}</span>
        <span className="h-px flex-1 bg-line" />
      </div>
    );
  }

  // A notice is a one-liner beside the answer it did not stop; a refusal is its own block.
  if (step.kind === "blocked" && !step.notice) {
    return <RefusalRow step={step} locale={locale} />;
  }

  return (
    <li className="row-enter">
      <div className="flex items-center gap-2 py-1.5">
        {hasChildren ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            className="icon-btn size-5"
            aria-label={stepLabel(locale, step)}
          >
            <Icon
              name="chevronRight"
              size={13}
              className={`disclosure-icon ${open ? "rotate-90" : ""}`}
            />
          </button>
        ) : (
          <StatusMark step={step} />
        )}
        <span className="text-quiet font-medium text-fg">{stepLabel(locale, step)}</span>
        {step.subject && (
          <code className="rounded-[var(--radius-sm)] bg-sunken px-1.5 py-0.5 font-mono text-micro text-muted">
            {step.subject}
          </code>
        )}
        {step.diff && (
          <span className="font-mono text-micro">
            <span className="text-success">+{step.diff.added}</span>{" "}
            <span className="text-danger">−{step.diff.removed}</span>
          </span>
        )}
        {text && (
          <span
            className={
              step.notice
                ? // A notice says what the run worked around, and that sentence is the row's whole
                  // payload — it wraps rather than trailing off under a clip.
                  "min-w-0 flex-1 whitespace-pre-wrap break-words text-fine text-muted"
                : "min-w-0 flex-1 truncate text-fine text-faint"
            }
          >
            · {text}
          </span>
        )}
      </div>
      {hasChildren && open && (
        <ul className="disclosure-panel ml-3 border-l border-line pl-3">
          {step.children!.map((child) => (
            <StepRow key={child.id} step={child} locale={locale} />
          ))}
        </ul>
      )}
    </li>
  );
}

/**
 * The reference renders the two sides of a conversation differently: the prompt is a bubble pushed
 * to the right, the answer is plain text in the column. Reasoning is neither — it is the collapsed
 * block above the answer, and it is never sent back to the model.
 */
function TurnRow({
  turn,
  locale,
  anchor,
  flash,
}: {
  turn: Turn;
  locale: Locale;
  anchor?: (el: HTMLLIElement | null) => void;
  flash?: boolean;
}) {
  if (turn.role === "user") {
    return (
      <li
        ref={anchor}
        data-flash={flash ? "true" : undefined}
        className="turn-anchor row-enter flex justify-end py-1.5"
      >
        <p className="max-w-[80%] whitespace-pre-wrap break-words rounded-[var(--radius-lg)] bg-accent-soft px-[var(--space-3)] py-1.5 text-quiet text-fg-strong">
          {turn.text}
        </p>
      </li>
    );
  }
  const waiting = turn.streaming && !turn.text && !turn.thought;
  return (
    <li className="row-enter py-1.5">
      {turn.thought && (
        // Open while it is the only thing there, folded once the answer arrives.
        <details className="mb-1.5" open={!turn.text}>
          <summary className="flex cursor-pointer items-center gap-1.5 text-fine text-muted">
            <Icon name="thought" size={13} />
            {translate(locale, turn.text ? "transcript.step.thought" : "chat.thinking")}
          </summary>
          <p className="disclosure-panel mt-1 whitespace-pre-wrap break-words border-l border-line pl-3 text-fine text-muted">
            {turn.thought}
          </p>
        </details>
      )}
      {turn.text ? (
        <Answer text={turn.text} streaming={turn.streaming} locale={locale} />
      ) : waiting ? (
        <p className="flex items-center gap-2 text-fine text-muted">
          <span className="dot-running" aria-hidden />
          {translate(locale, "chat.starting")}
        </p>
      ) : null}
      {turn.stopped && (
        <p className="mt-1 text-micro text-faint">{translate(locale, "transcript.stopped")}</p>
      )}
    </li>
  );
}

function RowView({ row, locale }: { row: Row; locale: Locale }) {
  return isTurn(row) ? (
    <TurnRow turn={row} locale={locale} />
  ) : (
    <StepRow step={row} locale={locale} />
  );
}

/** One prompt and the answer it got. `n` is the turn's position, spoken and titled but not drawn:
 *  the marker itself is a dash, and a column of numbers would be the loudest thing on the screen. */
type Checkpoint = { id: string; n: number; text: string };

/**
 * A checkpoint is a user turn: the question the pair is named after. Steps are not counted — they
 * hang off whichever run was in flight, so numbering them would make the same conversation number
 * differently between two renders of the same rows.
 */
function checkpointsOf(rows: Row[]): Checkpoint[] {
  const out: Checkpoint[] = [];
  for (const row of rows) {
    if (isTurn(row) && row.role === "user")
      out.push({ id: row.id, n: out.length + 1, text: row.text });
  }
  return out;
}

/** The prompt in one line, for the marker's tooltip and accessible name. */
function previewOf(text: string): string {
  const oneLine = text.replace(/\s+/g, " ").trim();
  return oneLine.length > 60 ? `${oneLine.slice(0, 60)}…` : oneLine;
}

const NEAR_BOTTOM_PX = 48;

/**
 * How long the open run has been running, in whole seconds, or null when nothing is open to time.
 *
 * The engine spends ten seconds or more booting the server and paging the weights in before the
 * first token arrives, and a spinner that never changes is what a frozen window looks like from
 * where the user is sitting. So the running line counts the wait it is asking to be waited out —
 * which is also what the reference shot does with its in-flight row.
 */
function useRunSeconds(startedAt: number | null): number | null {
  const [, setBeat] = useState(0);
  useEffect(() => {
    if (startedAt === null) return;
    const id = window.setInterval(() => setBeat((n) => n + 1), 1000);
    return () => window.clearInterval(id);
  }, [startedAt]);
  if (startedAt === null) return null;
  return Math.max(0, Math.round((Date.now() - startedAt) / 1000));
}

export function Transcript({ locale, onPickModel }: { locale: Locale; onPickModel: () => void }) {
  const rows = useTranscript((s) => s.rows);
  const running = useTranscript((s) => s.running);
  const runStartedAt = useTranscript((s) => s.runStartedAt);
  const waited = useRunSeconds(running ? runStartedAt : null);
  const scroll = useRef<HTMLDivElement>(null);
  const anchors = useRef(new Map<string, HTMLLIElement>());
  const flashTimer = useRef<number | undefined>(undefined);
  // False between a checkpoint jump and the next prompt, or while the user has scrolled up to read:
  // a column that follows the stream while the user is somewhere else is a column they cannot read.
  const follow = useRef(true);
  const [active, setActive] = useState("");
  const [flash, setFlash] = useState("");

  const checkpoints = useMemo(() => checkpointsOf(rows), [rows]);

  // The dash marks the pair at the top of the column, which is the pair the user is looking at.
  const spy = useCallback(() => {
    const box = scroll.current;
    if (!box) return;
    const edge = box.getBoundingClientRect().top + 24;
    let current = "";
    for (const item of checkpoints) {
      const el = anchors.current.get(item.id);
      if (!el) break;
      if (el.getBoundingClientRect().top > edge) break;
      current = item.id;
    }
    setActive(current);
  }, [checkpoints]);

  useEffect(() => {
    const box = scroll.current;
    if (!box) return;
    const last = rows[rows.length - 1];
    // Sending a prompt is the one thing that always means "watch this answer".
    if (last && isTurn(last) && last.role === "user") follow.current = true;
    if (follow.current) box.scrollTop = box.scrollHeight;
    spy();
  }, [rows, spy]);

  useEffect(() => () => window.clearTimeout(flashTimer.current), []);

  const jump = (id: string) => {
    const box = scroll.current;
    const el = anchors.current.get(id);
    if (!box || !el) return;
    follow.current = false;
    box.scrollTop += el.getBoundingClientRect().top - box.getBoundingClientRect().top - 12;
    setActive(id);
    setFlash(id);
    window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlash(""), 1400);
  };

  return (
    <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-bg">
      <header className="flex h-[var(--header-height)] shrink-0 items-center gap-2 border-b border-line px-[var(--space-4)]">
        <h1 className="min-w-0 flex-1 truncate text-ui font-medium text-fg">
          {translate(locale, "transcript.title")}
        </h1>
        {/* The skills and extensions this button would manage are not in this build. */}
        <button
          type="button"
          className="icon-btn"
          aria-label={translate(locale, "transcript.more")}
          aria-disabled
          title={translate(locale, "common.notInBuild")}
        >
          <Icon name="skills" />
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        <div
          ref={scroll}
          onScroll={() => {
            const box = scroll.current;
            if (box && box.scrollHeight - box.scrollTop - box.clientHeight < NEAR_BOTTOM_PX) {
              follow.current = true;
            }
            spy();
          }}
          className="min-h-0 min-w-0 flex-1 overflow-y-auto px-[var(--space-6)] py-[var(--space-4)]"
        >
          {rows.length === 0 ? (
            // The one screen in the app with nothing on it, so it is the one carrying the name. The
            // mark is drawn in the live accent rather than shipped as an image, which is why it can
            // be here in all eight theme combinations without a second palette.
            <div className="flex flex-col items-center gap-2 pt-[var(--space-6)] text-accent-ink">
              <BrandMark />
              <p className="text-ui font-medium text-fg-strong">{translate(locale, "app.name")}</p>
              <p className="max-w-[360px] text-center text-quiet text-muted">
                {translate(locale, "transcript.empty")}
              </p>
            </div>
          ) : (
            <ul className="mx-auto w-full max-w-[var(--content-width)]">
              {rows.map((row) =>
                isTurn(row) && row.role === "user" ? (
                  <TurnRow
                    key={row.id}
                    turn={row}
                    locale={locale}
                    flash={row.id === flash}
                    anchor={(el) => {
                      if (el) anchors.current.set(row.id, el);
                      else anchors.current.delete(row.id);
                    }}
                  />
                ) : (
                  <RowView key={row.id} row={row} locale={locale} />
                ),
              )}
            </ul>
          )}
          {running && (
            <p className="transcript-row mt-[var(--space-3)] tabular-nums">
              <span className="dot-running" aria-hidden />
              {waited === null
                ? translate(locale, "transcript.keepGoing")
                : translate(locale, "transcript.keepGoing.elapsed", { seconds: waited })}
            </p>
          )}
        </div>

        {checkpoints.length > 1 && (
          <nav
            aria-label={translate(locale, "transcript.checkpoints")}
            className="min-h-0 w-10 shrink-0 overflow-y-auto py-[var(--space-4)]"
          >
            <ol className="flex flex-col">
              {checkpoints.map((c) => (
                <li key={c.id}>
                  <button
                    type="button"
                    onClick={() => jump(c.id)}
                    data-active={c.id === active ? "true" : undefined}
                    title={previewOf(c.text)}
                    aria-label={translate(locale, "transcript.checkpoint", {
                      n: c.n,
                      text: previewOf(c.text),
                    })}
                    className="checkpoint"
                  />
                </li>
              ))}
            </ol>
          </nav>
        )}
      </div>

      <Composer locale={locale} onPickModel={onPickModel} />
    </main>
  );
}
