import { useState } from "react";

import { translate, type Locale } from "../i18n";
import { stepLabel, useTranscript, type Step } from "../state/transcript";
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
          <span className="min-w-0 flex-1 truncate text-[calc(var(--font-ui)-2px)] text-faint">
            · {step.detail}
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

export function Transcript({ locale }: { locale: Locale }) {
  const steps = useTranscript((s) => s.steps);
  const running = useTranscript((s) => s.running);

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

      <div className="min-h-0 flex-1 overflow-y-auto px-[var(--space-6)] py-[var(--space-4)]">
        {steps.length === 0 ? (
          <p className="mt-[var(--space-6)] text-center text-muted">
            {translate(locale, "transcript.empty")}
          </p>
        ) : (
          <ul className="mx-auto max-w-[720px]">
            {steps.map((step) => (
              <StepRow key={step.id} step={step} locale={locale} />
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
