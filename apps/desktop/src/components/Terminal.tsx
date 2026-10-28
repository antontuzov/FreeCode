import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";

import { translate, type Locale } from "../i18n";
import { ipc, type ShellRefusal } from "../lib/ipc";
import { shortcutLabel } from "../lib/shortcuts";
import { useShell } from "../state/shell";
import { RUNNER_LABEL_KEY, SHELL_TAB, useTerminal, type TerminalRun } from "../state/terminal";
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

/** The shell's tab is the folder it was opened in, the way a terminal's title is its working
 *  directory — one segment, since a strip is one line. */
function folderName(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const cut = trimmed.lastIndexOf("/");
  return cut >= 0 ? trimmed.slice(cut + 1) || trimmed : trimmed;
}

/** Why a line or a start was refused, in the interface's language. The host answers with a code and
 *  the command it refused; where the sentence points is the catalog's business. */
export function refusalText(locale: Locale, refusal: ShellRefusal): string {
  switch (refusal.code) {
    case "off":
      return translate(locale, "terminal.refusal.off");
    case "plan":
      return translate(locale, "terminal.refusal.plan");
    case "blocked":
      return refusal.command
        ? translate(locale, "terminal.refusal.blocked", { command: refusal.command })
        : translate(locale, "terminal.refusal.blockedShell");
    case "closed":
      return translate(locale, "terminal.refusal.closed");
    case "start":
      return translate(locale, "terminal.refusal.start", { reason: refusal.reason });
  }
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
 * The first tab is the panel's own shell, when it has one: a real terminal in the project folder, so a
 * failing check can be re-run by hand without leaving the app. It is not a way around the dial — see
 * `state/shell.ts` and `shell.rs` — and its body is where the user types.
 *
 * The chrome is the reference's bottom panel: a strip of tabs, the clear and fold controls, and the
 * way out. A tab's output is a terminal body rather than a transcript row — it follows the tail while
 * the command prints, and stops following the moment the reader scrolls up, so neither the reader nor
 * the output can drag the other.
 */
export function Terminal({ locale }: { locale: Locale }) {
  const runs = useTerminal((s) => s.runs);
  const shellTab = useTerminal((s) => s.shell);
  const active = useTerminal((s) => s.active);
  const collapsed = useTerminal((s) => s.collapsed);
  const focus = useTerminal((s) => s.focus);
  const dismiss = useTerminal((s) => s.dismiss);
  const clear = useTerminal((s) => s.clear);
  const setCollapsed = useTerminal((s) => s.setCollapsed);
  const setOpen = useTerminal((s) => s.setOpen);
  const showShell = useTerminal((s) => s.showShell);
  const hideShell = useTerminal((s) => s.hideShell);
  const shellStatus = useShell((s) => s.status);
  const shellCwd = useShell((s) => s.info?.cwd ?? "");

  const run = runs.find((one) => one.id === active) ?? null;
  const showingShell = shellTab && active === SHELL_TAB;
  const output = run?.output ?? "";

  // The tab appears first and the process is asked for second, so a slow start shows the shell as
  // opening rather than the press doing nothing. A refusal lands in the same tab as a sentence.
  const openShell = () => {
    showShell();
    void useShell.getState().open();
  };
  const closeShell = () => {
    void useShell.getState().close();
    hideShell();
  };

  const clearLabel = translate(locale, "terminal.clear");
  const foldLabel = translate(locale, collapsed ? "terminal.expand" : "terminal.collapse");
  const closeLabel = translate(locale, "terminal.close");
  const shellLabel = translate(locale, "terminal.shell.open");

  return (
    <section
      aria-label={translate(locale, "terminal.title")}
      className={`relative flex min-h-0 shrink-0 flex-col overflow-hidden border-t border-line bg-bg ${
        collapsed ? "" : "h-[var(--terminal-height)]"
      }`}
    >
      <div className="flex h-[var(--header-height)] shrink-0 items-center border-b border-line">
        <div
          role="tablist"
          aria-label={translate(locale, "terminal.title")}
          className="flex min-w-0 flex-1 items-center gap-[var(--space-1)] overflow-x-auto px-[var(--space-2)]"
        >
          {runs.length === 0 && !shellTab && (
            <span className="shrink-0 px-[var(--space-2)] text-quiet text-faint">
              {translate(locale, "terminal.title")}
            </span>
          )}
          {shellTab && (
            <div
              data-active={showingShell}
              className={`flex h-8 min-w-0 max-w-[220px] shrink-0 items-center rounded-[var(--radius-md)] pl-[var(--space-2)] pr-0.5 ${
                showingShell ? "bg-elevated" : "hover:bg-hover"
              }`}
            >
              <button
                type="button"
                role="tab"
                aria-selected={showingShell}
                onClick={() => focus(SHELL_TAB)}
                title={shellCwd || translate(locale, "terminal.shell")}
                className="flex min-w-0 items-center gap-[var(--space-2)] py-1 text-quiet"
              >
                {shellStatus === "opening" ? (
                  <span aria-hidden className="dot-running shrink-0" />
                ) : (
                  <Icon name="terminal" size={12} className="shrink-0 text-faint" />
                )}
                <span className="truncate font-mono">
                  {shellCwd ? folderName(shellCwd) : translate(locale, "terminal.shell")}
                </span>
              </button>
              <button
                type="button"
                onClick={closeShell}
                aria-label={translate(locale, "terminal.closeShell")}
                title={translate(locale, "terminal.closeShell")}
                className="icon-btn shrink-0"
              >
                <Icon name="close" size={11} />
              </button>
            </div>
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
          {!shellTab && (
            <button
              type="button"
              onClick={openShell}
              aria-label={shellLabel}
              title={shellLabel}
              className="icon-btn shrink-0"
            >
              <Icon name="plus" size={15} />
            </button>
          )}
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
        (showingShell ? (
          <ShellBody locale={locale} />
        ) : run ? (
          <RunBody locale={locale} run={run} output={output} />
        ) : (
          <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-[var(--space-2)] px-[var(--space-6)] text-center">
            <Icon name="terminal" size={24} className="text-faint" strokeWidth={1.2} />
            <p className="max-w-[52ch] text-fine text-faint">
              {translate(locale, "terminal.empty")}
            </p>
            <button
              type="button"
              onClick={openShell}
              className="mt-[var(--space-1)] inline-flex items-center gap-1.5 rounded-[var(--radius-md)] border border-line px-[var(--space-3)] py-1 text-quiet text-muted hover:bg-hover hover:text-fg"
            >
              <Icon name="terminal" size={13} />
              {shellLabel}
            </button>
          </div>
        ))}
    </section>
  );
}

/**
 * One command's output, as the panel draws it. It follows the tail while the command prints and stops
 * the moment the reader scrolls up: new bytes must not pull the line they are reading out from under
 * them.
 */
function RunBody({ locale, run, output }: { locale: Locale; run: TerminalRun; output: string }) {
  const body = useRef<HTMLPreElement>(null);
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
  }, [run.id]);

  useEffect(() => {
    const el = body.current;
    if (!follow.current || !el) return;
    el.scrollTop = el.scrollHeight;
  }, [output]);

  return (
    <pre
      ref={body}
      // A command's output is watched, not read aloud: a screen reader announcing every chunk of a
      // build log would be one long interruption rather than a transcript.
      aria-live="off"
      onScroll={(event) => {
        const el = event.currentTarget;
        follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
      className="min-h-0 flex-1 overflow-y-auto whitespace-pre-wrap break-words px-[var(--space-4)] py-[var(--space-2)] font-mono text-fine text-muted"
    >
      {/* The prompt line a terminal session opens with is the command itself, so it is drawn only
          where the label is one. A check's label is a sentence about the run; the runner already put
          the command it chose at the top of its own output. */}
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
  );
}

/** Ten characters in the body's own font: the width one character takes, which is what turns the
 *  panel's pixels into the terminal's columns. */
const RULER = "0000000000";

/**
 * The panel's own shell, as a terminal body and a prompt row.
 *
 * The field is a line, not a keystroke stream: the host admits the whole line through the approval
 * engine before it reaches the terminal, and the shell runs with its own echo and line editor off, so
 * the prompt row *is* the shell's input. What that costs is stated rather than hidden — there is no
 * cursor editing or tab completion here — and what it buys is that a line the dial refuses never runs,
 * which is the half the dial exists for.
 */
function ShellBody({ locale }: { locale: Locale }) {
  const status = useShell((s) => s.status);
  const info = useShell((s) => s.info);
  const text = useShell((s) => s.text);
  const exitCode = useShell((s) => s.exitCode);
  const refusal = useShell((s) => s.refusal);
  const open = useShell((s) => s.open);
  const send = useShell((s) => s.send);
  const interrupt = useShell((s) => s.interrupt);

  const [line, setLine] = useState("");
  const history = useRef<string[]>([]);
  // One past the newest line is the empty field the user was typing in; walking up the list is what
  // an arrow key means in every terminal.
  const recalled = useRef(0);
  const body = useRef<HTMLPreElement>(null);
  const ruler = useRef<HTMLSpanElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const follow = useRef(true);

  useEffect(() => {
    input.current?.focus();
  }, []);

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
  }, [info?.id]);

  useEffect(() => {
    const el = body.current;
    if (!follow.current || !el) return;
    el.scrollTop = el.scrollHeight;
  }, [text, status]);

  // The terminal's width is the panel's, so a command that formats to the terminal draws where the
  // reader sees it end. Measured rather than assumed: the character's width comes from a ruler drawn
  // in the body's own font, and the rows from the body's own line height.
  useEffect(() => {
    if (status !== "live") return;
    const el = body.current;
    const sample = ruler.current;
    if (!el || !sample) return;
    const measure = () => {
      const style = getComputedStyle(el);
      const char = sample.getBoundingClientRect().width / RULER.length;
      const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.5;
      if (!char || !lineHeight) return;
      const cols =
        (el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight)) / char;
      const rows =
        (el.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom)) /
        lineHeight;
      void ipc
        .shellResize(Math.max(2, Math.floor(rows)), Math.max(20, Math.floor(cols)))
        .catch(() => {
          // A resize the host could not apply is not worth a sentence over the prompt.
        });
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [status]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const command = line.trim();
    if (!command) {
      setLine("");
      return;
    }
    const admitted = await send(command);
    if (!admitted) {
      // The line stays in the field: a refused command is something to fix, not to retype. The
      // refusal above says which rule said no.
      return;
    }
    const list = history.current;
    if (list[list.length - 1] !== command) list.push(command);
    recalled.current = list.length;
    setLine("");
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const list = history.current;
    if (event.key === "ArrowUp" && list.length > 0) {
      event.preventDefault();
      recalled.current = Math.max(0, recalled.current - 1);
      setLine(list[recalled.current] ?? "");
      return;
    }
    if (event.key === "ArrowDown" && list.length > 0) {
      event.preventDefault();
      recalled.current = Math.min(list.length, recalled.current + 1);
      setLine(list[recalled.current] ?? "");
      return;
    }
    // Ctrl-C is the terminal's own interrupt: it stops a command rather than starting one, so no line
    // is admitted and nothing is written to the policy.
    if (event.key === "c" && event.ctrlKey) {
      event.preventDefault();
      setLine("");
      recalled.current = history.current.length;
      interrupt();
    }
  };

  const placeholder = translate(locale, "terminal.shell.placeholder");

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <span
        ref={ruler}
        aria-hidden
        className="pointer-events-none invisible absolute font-mono text-fine"
      >
        {RULER}
      </span>
      <pre
        ref={body}
        aria-live="off"
        onScroll={(event) => {
          const el = event.currentTarget;
          follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
        className="min-h-0 flex-1 overflow-y-auto whitespace-pre-wrap break-words px-[var(--space-4)] py-[var(--space-2)] font-mono text-fine text-muted"
      >
        {text ? (
          text
        ) : (
          <span className="text-faint">
            {status === "opening"
              ? translate(locale, "terminal.shell.opening")
              : translate(locale, "terminal.shell.empty")}
          </span>
        )}
      </pre>

      {refusal && (
        <p
          role="alert"
          className="shrink-0 border-t border-line px-[var(--space-4)] py-1.5 text-fine text-danger"
        >
          {refusalText(locale, refusal)}
        </p>
      )}

      {status === "live" ? (
        <form
          onSubmit={(event) => void submit(event)}
          className="flex shrink-0 items-center gap-[var(--space-2)] border-t border-line px-[var(--space-4)] py-1"
        >
          <span aria-hidden className="select-none font-mono text-fine text-faint">
            $
          </span>
          <input
            ref={input}
            value={line}
            onChange={(event) => setLine(event.target.value)}
            onKeyDown={onKeyDown}
            aria-label={placeholder}
            placeholder={placeholder}
            spellCheck={false}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            className="min-w-0 flex-1 bg-transparent font-mono text-fine text-fg outline-none placeholder:text-faint"
          />
          <button
            type="button"
            onClick={() => {
              setLine("");
              interrupt();
            }}
            aria-label={translate(locale, "terminal.shell.interrupt")}
            title={translate(locale, "terminal.shell.interrupt")}
            className="icon-btn shrink-0"
          >
            <Icon name="stop" size={13} />
          </button>
        </form>
      ) : status === "opening" ? null : (
        <div className="flex shrink-0 items-center justify-between gap-[var(--space-3)] border-t border-line px-[var(--space-4)] py-1 text-fine text-faint">
          <span>
            {status === "ended"
              ? exitCode === null
                ? translate(locale, "terminal.shell.ended")
                : translate(locale, "terminal.shell.endedCode", { code: exitCode })
              : translate(locale, "terminal.shell.closed")}
          </span>
          <button
            type="button"
            onClick={() => void open()}
            className="rounded-[var(--radius-md)] border border-line px-[var(--space-2)] py-0.5 text-quiet text-muted hover:bg-hover hover:text-fg"
          >
            {translate(locale, "terminal.shell.restart")}
          </button>
        </div>
      )}
    </div>
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
