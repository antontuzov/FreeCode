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
import type { Components } from "react-markdown";
import Markdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeKatex from "rehype-katex";
import remarkMath from "remark-math";

import type { Locale } from "../i18n";
import { rehypeCodeLines } from "../lib/codeLines";
import { ExternalLink } from "./ExternalLink";

/** The fence's language rides on the inner `<code>`; the chip has to reach past `<pre>` for it. */
type Fenced = { props?: { className?: string } };

const COMPONENTS: Components = {
  pre: ({ children }) => {
    const first = (Array.isArray(children) ? children[0] : children) as Fenced | undefined;
    const lang = /language-([\w-]+)/.exec(first?.props?.className ?? "")?.[1];
    return (
      <div className="answer-code">
        {/* Named even when the fence did not say: `detect` filled it in from the content, and the
            chip reports which of the two it was. */}
        {lang && <span className="answer-code-lang">{lang}</span>}
        <pre>{children}</pre>
      </div>
    );
  },
};

/**
 * A link in an answer is the same control as a refusal's action, so it gets the same treatment: the
 * address leaves through the system opener, and the webview never navigates. Without this, a model
 * that writes `[the fork](https://github.com/…)` would take the app's own window away with it.
 */
function linkComponents(locale: Locale): Components {
  return {
    ...COMPONENTS,
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
          [rehypeHighlight, { detect: true, ignoreMissing: true }],
          // After the highlighter, because it is the one that folds the snippet into a single span;
          // the line split reads that span's newlines and keeps each token's class with both halves.
          rehypeCodeLines,
          [rehypeKatex, { throwOnError: false, errorColor: "inherit", output: "htmlAndMathml" }],
        ]}
        components={linkComponents(locale)}
      >
        {text}
      </Markdown>
    </div>
  );
}
