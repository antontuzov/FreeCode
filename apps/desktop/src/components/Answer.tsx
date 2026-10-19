/**
 * The answer as the model wrote it.
 *
 * A Bonsai answers in markdown — headings, fenced code, `$\frac{a}{b}$` for arithmetic — and
 * rendering that into a `whitespace-pre-wrap` paragraph put the source on screen: `### 1.
 * **Numbers**` with the stars still attached, and a `\frac` nobody could read. So the answer gets a
 * parser, and the parser gets KaTeX, because a model small enough to run on a laptop is still being
 * asked to do algebra.
 *
 * Raw HTML is not rendered. `react-markdown` drops it unless a caller opts into `rehype-raw`, and
 * this does not: the text here is produced by a weights file anyone could publish, and a chat
 * surface that executes its own transcript is a bug report waiting to happen. Malformed LaTeX is
 * shown as the source rather than thrown, for the same reason — a model that writes a stray `$`
 * has not failed the turn, and the answer should not look like it did.
 */
import "katex/dist/katex.min.css";
import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Components } from "react-markdown";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";

import { translate, type Locale } from "../i18n";
import { rehypeCodeLines } from "../lib/codeLines";
import { fenceLanguage, rehypeFenceLanguage } from "../lib/fenceLanguage";
import { ipc } from "../lib/ipc";
import { ExternalLink } from "./ExternalLink";
import { Icon } from "./icons";

/** The fence's language rides on the inner `<code>`; the chip has to reach past `<pre>` for it. */
type Fenced = { props?: { className?: string } };

/** `idle` is the button as drawn; the other two are the host's answer, held for a moment. */
type FenceCopy = "idle" | "copied" | "failed";

function copyLabel(locale: Locale, state: FenceCopy): string {
  if (state === "copied") return translate(locale, "answer.code.copied");
  if (state === "failed") return translate(locale, "answer.code.failed");
  return translate(locale, "answer.code.copy");
}

/**
 * The fence's source, as the page holds it.
 *
 * It is read off the rendered lines rather than the markdown text because `rehypeCodeLines` consumed
 * every newline the fence had: the only thing left holding the lines apart is the span each one got,
 * so the source comes back by joining them with the newline they no longer carry. A `pre` the parser
 * declined to number has no lines and so nothing to copy, which is the honest answer for it.
 */
function fenceText(root: Element): string {
  return [...root.querySelectorAll(".code-line")].map((line) => line.textContent ?? "").join("\n");
}

/**
 * A code block with the one control a chat surface owes it: a copy button that reports what the
 * pasteboard actually did.
 *
 * The write is the host's (`ipc.copyText`), so the label flips only when the pasteboard has taken the
 * text — a button that said "Copied" on a webview that silently dropped the write would be a worse
 * answer than one that says it failed. And a fence still streaming has no line of its own yet, which
 * is a click with nothing to hand over rather than an empty success.
 */
function CodeFence({ children, locale }: { children: ReactNode; locale: Locale }) {
  const [state, setState] = useState<FenceCopy>("idle");
  const box = useRef<HTMLDivElement>(null);
  const reset = useRef(0);
  useEffect(() => () => window.clearTimeout(reset.current), []);

  const first = (Array.isArray(children) ? children[0] : children) as Fenced | undefined;
  const lang = fenceLanguage(first?.props?.className);

  async function copy() {
    if (!box.current) return;
    const text = fenceText(box.current);
    if (!text) return;
    let next: FenceCopy = "failed";
    try {
      await ipc.copyText(text);
      next = "copied";
    } catch (reason) {
      // The host's reason is a string about the pasteboard, which belongs in the log rather than on
      // the button: the label has already said the copy did not happen.
      console.warn("copy failed", reason);
    }
    setState(next);
    window.clearTimeout(reset.current);
    reset.current = window.setTimeout(() => setState("idle"), 1800);
  }

  const label = copyLabel(locale, state);
  return (
    <div className="answer-code" ref={box}>
      <div className="answer-code-tools">
        {/* Only a fence the model named gets a chip; `detect`'s guess colours the block but says
            nothing the answer claimed, so it stays unlabelled. */}
        {lang && <span className="answer-code-lang">{lang}</span>}
        <button
          type="button"
          className={`answer-code-copy ${state}`}
          onClick={copy}
          aria-label={label}
          title={label}
        >
          <Icon name="copy" size={13} />
        </button>
      </div>
      <pre>{children}</pre>
    </div>
  );
}

function markdownComponents(locale: Locale): Components {
  return {
    pre: ({ children }) => <CodeFence locale={locale}>{children}</CodeFence>,
    /**
     * A link in an answer is the same control as a refusal's action, so it gets the same treatment:
     * the address leaves through the system opener, and the webview never navigates. Without this, a
     * model that writes `[the fork](https://github.com/…)` would take the app's own window away.
     */
    a: ({ href, children }) => (
      <ExternalLink url={href ?? ""} label={children} locale={locale} inline />
    ),
  };
}

export function Answer({
  text,
  streaming,
  locale,
}: {
  text: string;
  streaming?: boolean;
  locale: Locale;
}) {
  return (
    <div className={`answer ${streaming ? "caret-streaming" : ""}`}>
      <Markdown
        remarkPlugins={[remarkMath]}
        rehypePlugins={[
          // Before the highlighter, because `detect` writes its guess onto the node as a
          // `language-*` class, and after it runs a named fence and a guessed one look identical.
          rehypeFenceLanguage,
          [rehypeHighlight, { detect: true, ignoreMissing: true }],
          // After the highlighter, because it is the one that folds the snippet into a single span;
          // the line split reads that span's newlines and keeps each token's class with both halves.
          rehypeCodeLines,
          [rehypeKatex, { throwOnError: false, errorColor: "inherit", output: "htmlAndMathml" }],
        ]}
        components={markdownComponents(locale)}
      >
        {text}
      </Markdown>
    </div>
  );
}
