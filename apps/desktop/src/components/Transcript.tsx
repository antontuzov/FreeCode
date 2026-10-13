import { useEffect, useRef, useState } from "react";

import { translate, type Locale } from "../i18n";
import {
  isTurn,
  stepLabel,
  useTranscript,
  type Row,
  type Step,
  type Turn,
} from "../state/transcript";
import { Composer } from "./Composer";
import { Icon } from "./icons";

function StatusMark({ status }: { status?: Step["status"] }) {
  if (status === "running") return <span className="dot-running" aria-hidden />;
  if (status === "error" || status === "stopped")
    return <Icon name="warning" size={13} className="text-danger" />;
  return <Icon name="review" size={13} className="text-accent" />;
}

function StepRow({ step, locale }: { step: Step; locale: Locale }) {
  const [open, setOpen] = useState(true);
  const hasChildren = !!step.children?.length;

  if (step.kind === "contextCompressed") {
    return (
      <div className="my-[var(--space-4)] flex items-center gap-3 text-faint">
        <span className="h-px flex-1 bg-line" />
        <Icon name="context" size={14} />
        <span className="text-[calc(var(--font-ui)-1px)]">{stepLabel(locale, step)}</span>
        <span className="h-px flex-1 bg-line" />
      </div>
    );
  }

  return (
    <li>
      <div className="flex items-center gap-2 py-1.5">
        {hasChildren ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            className="icon-btn size-5"
            aria-label={stepLabel(locale, step)}
          >
            <Icon name="chevronRight" size={13} className={open ? "rotate-90" : ""} />
          </button>
        ) : (
          <StatusMark status={step.status} />
        )}
        <span className="text-[calc(var(--font-ui)-1px)] font-medium text-fg">
          {stepLabel(locale, step)}
        </span>
        {step.subject && (
          <code className="rounded-[var(--radius-sm)] bg-sunken px-1.5 py-0.5 font-mono text-[calc(var(--font-ui)-3px)] text-muted">
            {step.subject}
          </code>
        )}
        {step.diff && (
          <span className="font-mono text-[calc(var(--font-ui)-3px)]">
            <span className="text-success">+{step.diff.added}</span>{" "}
            <span className="text-danger">−{step.diff.removed}</span>
          </span>
        )}
        {step.detail && (
          <span
            className={
              step.kind === "blocked"
                ? // A refusal is read, not skimmed: it wraps, and the action that clears it follows
                  // the reason rather than replacing it.
                  "min-w-0 flex-1 whitespace-pre-wrap break-words text-[calc(var(--font-ui)-2px)] text-muted"
                : "min-w-0 flex-1 truncate text-[calc(var(--font-ui)-2px)] text-faint"
            }
          >
            · {step.detail}
            {step.cta && (
              <span className="mt-1 block text-fg">
                {step.cta.label}: <span className="font-mono">{step.cta.url}</span>
              </span>
            )}
          </span>
        )}
      </div>
      {hasChildren && open && (
        <ul className="ml-3 border-l border-line pl-3">
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
function TurnRow({ turn, locale }: { turn: Turn; locale: Locale }) {
  if (turn.role === "user") {
    return (
      <li className="flex justify-end py-1.5">
        <p className="max-w-[80%] whitespace-pre-wrap break-words rounded-[var(--radius-lg)] bg-accent-soft px-[var(--space-3)] py-1.5 text-[calc(var(--font-ui)-1px)] text-fg-strong">
          {turn.text}
        </p>
      </li>
    );
  }
  const waiting = turn.streaming && !turn.text && !turn.thought;
  return (
    <li className="py-1.5">
      {turn.thought && (
        // Open while it is the only thing there, folded once the answer arrives.
        <details className="mb-1.5" open={!turn.text}>
          <summary className="flex cursor-pointer items-center gap-1.5 text-[calc(var(--font-ui)-2px)] text-muted">
            <Icon name="thought" size={13} />
            {translate(locale, turn.text ? "transcript.step.thought" : "chat.thinking")}
          </summary>
          <p className="mt-1 whitespace-pre-wrap break-words border-l border-line pl-3 text-[calc(var(--font-ui)-2px)] text-muted">
            {turn.thought}
          </p>
        </details>
      )}
      {turn.text ? (
        <p className="whitespace-pre-wrap break-words text-[calc(var(--font-ui)-1px)] leading-relaxed text-fg">
          {turn.text}
        </p>
      ) : waiting ? (
        <p className="flex items-center gap-2 text-[calc(var(--font-ui)-2px)] text-muted">
          <span className="dot-running" aria-hidden />
          {translate(locale, "chat.starting")}
        </p>
      ) : null}
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

export function Transcript({ locale }: { locale: Locale }) {
  const rows = useTranscript((s) => s.rows);
  const running = useTranscript((s) => s.running);
  const scroll = useRef<HTMLDivElement>(null);

  // A streamed answer that renders below the fold is indistinguishable from one that never arrived,
  // so the column follows the text as it grows.
  useEffect(() => {
    const box = scroll.current;
    if (box) box.scrollTop = box.scrollHeight;
  }, [rows]);

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-bg">
      <header className="flex h-[var(--header-height)] shrink-0 items-center gap-2 border-b border-line px-[var(--space-4)]">
        <h1 className="min-w-0 flex-1 truncate text-sm font-medium text-fg">
          {translate(locale, "transcript.title")}
        </h1>
        <button
          type="button"
          className="icon-btn"
          aria-label={translate(locale, "transcript.more")}
        >
          <Icon name="skills" />
        </button>
      </header>

      <div
        ref={scroll}
        className="min-h-0 flex-1 overflow-y-auto px-[var(--space-6)] py-[var(--space-4)]"
      >
        {rows.length === 0 ? (
          <p className="mt-[var(--space-6)] text-center text-muted">
            {translate(locale, "transcript.empty")}
          </p>
        ) : (
          <ul className="mx-auto max-w-[720px]">
            {rows.map((row) => (
              <RowView key={row.id} row={row} locale={locale} />
            ))}
          </ul>
        )}
        {running && (
          <p className="transcript-row mt-[var(--space-3)]">
            <span className="dot-running" aria-hidden />
            {translate(locale, "transcript.keepGoing")}
          </p>
        )}
      </div>

      <Composer locale={locale} />
    </main>
  );
}
