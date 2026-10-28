import { useEffect, useRef } from "react";

import { translate, type Locale } from "../i18n";
import { shortcutLabel } from "../lib/shortcuts";
import { RUNNER_LABEL_KEY, useTerminal, type TerminalRun } from "../state/terminal";
import { Icon } from "./icons";

/** What a tab is called: the command as the host worded it, or the check it ran when the manifest
 *  chose the command and the row carries no subject. Only the first line is a label — a script can
 *  be several, and a strip is one. A check keeps the name of the thing it checked, so a filtered
 *  run says "Ran tests · auth" rather than a lone filter with nothing saying what it filtered. */
function labelOf(run: TerminalRun, locale: Locale): string {
  const first = run.label.split("\n")[0]?.trim() ?? "";
  const runner = RUNNER_LABEL_KEY[run.kind];
  if (!runner) return first || translate(locale, "transcript.step.terminal");
  const name = translate(locale, runner);
  return first ? `${name} · ${first}` : name;
}

/**
 * The terminal panel: the commands this task ran, one tab each, with the output of the tab on screen.
 *
 * The host streams a command's bytes into the window while it is still running (`TerminalOutput`,
 * drained off the same pipes `exec` always read), so the panel is a view of the run rather than a
 * second way to run anything: what a command may do, how long it may take and how it is killed are
 * still exactly the shell's own rules. The last N runs stay when a run ends, because a check's
 * failure is read after it failed.
 *
 * The chrome is the reference's bottom panel: a strip of tabs, the clear and fold controls, and the
 * way out. A tab's output is a terminal body rather than a transcript row — it follows the tail while
 * the command prints, and stops following the moment the reader scrolls up, so neither the reader nor
 * the output can drag the other.
 */
export function Terminal({ locale }: { locale: Locale }) {
  const runs = useTerminal((s) => s.runs);
  const active = useTerminal((s) => s.active);
  const collapsed = useTerminal((s) => s.collapsed);
  const focus = useTerminal((s) => s.focus);
  const dismiss = useTerminal((s) => s.dismiss);
  const clear = useTerminal((s) => s.clear);
  const setCollapsed = useTerminal((s) => s.setCollapsed);
  const setOpen = useTerminal((s) => s.setOpen);

  const run = runs.find((one) => one.id === active) ?? null;
  const output = run?.output ?? "";
  const body = useRef<HTMLPreElement>(null);
  // Whether the body is pinned to the last line. A reader who scrolled up is reading something that
  // already printed; new bytes must not pull the line they are on out from under them.
  const follow = useRef(true);

  // A new tab starts at its own end. Switching tabs is the reader asking for that command's whole
  // output, and the useful end of it is the last line. The scroll is repeated on the next frame
  // because the webview can commit the tab before it has laid the text out, and a jump taken against
  // the pre-layout height leaves the reader at the top of a log they opened to see the end of.
  useEffect(() => {
    follow.current = true;
    const el = body.current;
    if (!el) return;
    const toEnd = () => {
      el.scrollTop = el.scrollHeight;
    };
    toEnd();
    const frame = requestAnimationFrame(toEnd);
    return () => cancelAnimationFrame(frame);
  }, [run?.id, collapsed]);

  useEffect(() => {
    const el = body.current;
    if (!follow.current || !el) return;
    el.scrollTop = el.scrollHeight;
  }, [output]);

  const clearLabel = translate(locale, "terminal.clear");
  const foldLabel = translate(locale, collapsed ? "terminal.expand" : "terminal.collapse");
  const closeLabel = translate(locale, "terminal.close");

  return (
    <section
      aria-label={translate(locale, "terminal.title")}
      className={`flex min-h-0 shrink-0 flex-col overflow-hidden border-t border-line bg-bg ${
        collapsed ? "" : "h-[var(--terminal-height)]"
      }`}
    >
      <div className="flex h-[var(--header-height)] shrink-0 items-center border-b border-line">
        <div
          role="tablist"
          aria-label={translate(locale, "terminal.title")}
          className="flex min-w-0 flex-1 items-center gap-[var(--space-1)] overflow-x-auto px-[var(--space-2)]"
        >
          {runs.length === 0 && (
            <span className="shrink-0 px-[var(--space-2)] text-quiet text-faint">
              {translate(locale, "terminal.title")}
            </span>
          )}
          {runs.map((one) => {
            const label = labelOf(one, locale);
            const shown = one.id === active;
            return (
              <div
                key={one.id}
                data-active={shown}
                className={`flex h-8 min-w-0 max-w-[220px] shrink-0 items-center rounded-[var(--radius-md)] pl-[var(--space-2)] pr-0.5 ${
                  shown ? "bg-elevated" : "hover:bg-hover"
                }`}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={shown}
                  onClick={() => focus(one.id)}
                  title={label}
                  className="flex min-w-0 items-center gap-[var(--space-2)] py-1 text-quiet"
                >
                  <RunMark run={one} />
                  <span className="truncate font-mono">{label}</span>
                </button>
                <button
                  type="button"
                  onClick={() => dismiss(one.id)}
                  aria-label={translate(locale, "terminal.closeTab")}
                  title={translate(locale, "terminal.closeTab")}
                  className="icon-btn shrink-0"
                >
                  <Icon name="close" size={11} />
                </button>
              </div>
            );
          })}
        </div>
        <div className="flex shrink-0 items-center gap-0.5 pr-[var(--space-2)]">
          <button
            type="button"
            onClick={clear}
            disabled={runs.length === 0}
            aria-label={clearLabel}
            title={clearLabel}
            className="icon-btn shrink-0"
          >
            <Icon name="trash" size={14} />
          </button>
          <button
            type="button"
            onClick={() => setCollapsed(!collapsed)}
            aria-expanded={!collapsed}
            aria-label={foldLabel}
            title={foldLabel}
            className="icon-btn shrink-0"
          >
            <Icon
              name="chevronDown"
              size={14}
              className={`disclosure-icon ${collapsed ? "rotate-180" : ""}`}
            />
          </button>
          <button
            type="button"
            onClick={() => setOpen(false)}
            aria-label={closeLabel}
            title={`${closeLabel} · ${shortcutLabel("terminal")}`}
            className="icon-btn shrink-0"
          >
            <Icon name="close" size={14} />
          </button>
        </div>
      </div>

      {!collapsed &&
        (run ? (
          <pre
            ref={body}
            // A command's output is watched, not read aloud: a screen reader announcing every chunk
            // of a build log would be one long interruption rather than a transcript.
            aria-live="off"
            onScroll={(event) => {
              const el = event.currentTarget;
              follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
            }}
            className="min-h-0 flex-1 overflow-y-auto whitespace-pre-wrap break-words px-[var(--space-4)] py-[var(--space-2)] font-mono text-fine text-muted"
          >
            {/* The prompt line a terminal session opens with is the command itself, so it is drawn
                only where the label is one. A check's label is a sentence about the run; the runner
                already put the command it chose at the top of its own output. */}
            {run.kind === "terminal" && run.label.trim() && (
              <>
                <span className="select-none text-faint">{"$ "}</span>
                <span className="text-fg-strong">{run.label.split("\n")[0]}</span>
                {"\n"}
              </>
            )}
            {output ? (
              output
            ) : (
              <span className="text-faint">
                {run.status === "running"
                  ? translate(locale, "terminal.waiting")
                  : translate(locale, "terminal.emptyOutput")}
              </span>
            )}
          </pre>
        ) : (
          <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-[var(--space-2)] px-[var(--space-6)] text-center">
            <Icon name="terminal" size={24} className="text-faint" strokeWidth={1.2} />
            <p className="max-w-[52ch] text-fine text-faint">
              {translate(locale, "terminal.empty")}
            </p>
          </div>
        ))}
    </section>
  );
}

/** The tab's state as a mark: a live command pulses, a failed one is the only one that spends the
 *  danger colour, and a command that ended well needs no badge — its tab is the record. */
function RunMark({ run }: { run: TerminalRun }) {
  if (run.status === "error") {
    return <span aria-hidden className="size-2 shrink-0 rounded-full bg-danger" />;
  }
  if (run.status === "running") {
    return <span aria-hidden className="dot-running shrink-0" />;
  }
  return <Icon name="terminal" size={12} className="shrink-0 text-faint" />;
}
