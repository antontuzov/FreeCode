/**
 * Keeps a highlighter's guess from being printed as the fence's language.
 *
 * `rehype-highlight` with `detect: true` colours an unnamed fence by guessing at its content, and it
 * writes that guess back onto the node as a `language-*` class (its own source, line 134). The chip
 * above the fence reads that class — so a fenced block of markdown prose came out of a real run
 * wearing a `csharp` label, which is the highlighter's mistake printed as the model's statement.
 *
 * The fix is to know which fences were named before the highlighter gets the chance to rename them,
 * which is why this pass runs first. The marker is deliberately not spelled `lang-*` or `language-*`:
 * the highlighter's own reader treats either prefix as a name and would highlight nothing.
 */
import type { Element, ElementContent, Root } from "hast";

/** The class this pass adds, and the one the fence's chip checks before it prints a language. */
export const GUESSED_CLASS = "guessed-language";

export function rehypeFenceLanguage(): (tree: Root) => void {
  return (tree) => {
    const found: Element[] = [];
    collectFences(tree, found);
    for (const pre of found) {
      const code = pre.children.find(isCode);
      if (!code) continue;
      const classes = Array.isArray(code.properties.className) ? code.properties.className : [];
      if (classes.some(startsWithLanguage)) continue;
      code.properties.className = [...classes, GUESSED_CLASS];
    }
  };
}

function isCode(node: ElementContent): node is Element {
  return node.type === "element" && node.tagName === "code";
}

/** A `pre` owns its fence, so the walk collects one and does not descend into it. */
function collectFences(node: Root | Element, found: Element[]): void {
  for (const child of node.children) {
    if (child.type !== "element") continue;
    if (child.tagName === "pre") found.push(child);
    else collectFences(child, found);
  }
}

function startsWithLanguage(value: unknown): boolean {
  const name = String(value);
  return name.startsWith("language-") || name.startsWith("lang-");
}

/**
 * The fence's language, or nothing when there is nothing true to say about it.
 *
 * A named fence prints its name. An unnamed one arrives here already wearing `language-*`, because
 * `detect` wrote its guess onto the node — so the marker this pass lays down before the highlighter
 * runs is what tells the two apart, and a guess shows no chip rather than a confident wrong one.
 *
 * The classes come off the rendered `<code>` element, where React has already joined them into one
 * string.
 */
export function fenceLanguage(className: string | undefined): string | undefined {
  const classes = (className ?? "").split(/\s+/);
  if (classes.includes(GUESSED_CLASS)) return undefined;
  return /language-([\w-]+)/.exec(className ?? "")?.[1];
}
