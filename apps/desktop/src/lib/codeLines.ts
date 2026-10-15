/**
 * One block per source line inside a fenced code answer, so CSS can put a number in the gutter.
 *
 * Line numbers are a setting the user flips at any time, and re-rendering the transcript to apply it
 * would mean re-parsing every answer on screen. So the parser's only job is to make the lines
 * addressable; whether they carry a number, and how wide that gutter is, is decided by two attribute
 * rules off the document element. It also keeps the pass safe on a stream: a half-finished fence gains
 * a line for each newline the model has emitted so far.
 *
 * Highlighting runs first, and its `<span class="hljs">` wrapper holds the whole snippet in one text
 * node with the newlines inside it. Splitting that node, rather than counting lines of rendered text,
 * is what keeps a token painted across a break intact: each half keeps the class that coloured it.
 */
import type { Element, ElementContent, Root } from "hast";

export function rehypeCodeLines(): (tree: Root) => void {
  return (tree) => {
    const found: Element[] = [];
    collectFences(tree, found);
    for (const pre of found) {
      const code = pre.children.find(isCode);
      // A `pre` with no `code` child is hand-written markup, not a fence, and has nothing to number.
      if (code) code.children = toLines(code.children).map(line);
    }
  };
}

function isCode(node: ElementContent): node is Element {
  return node.type === "element" && node.tagName === "code";
}

/** A `pre` owns its lines, so the walk collects one and never descends into it — going deeper would
 * re-split the lines this pass already made. */
function collectFences(node: Root | Element, found: Element[]): void {
  for (const child of node.children) {
    if (child.type !== "element") continue;
    if (child.tagName === "pre") found.push(child);
    else collectFences(child, found);
  }
}

function line(children: ElementContent[]): Element {
  return { type: "element", tagName: "span", properties: { className: ["code-line"] }, children };
}

/** Children grouped by the line of source each character sat on. */
function toLines(children: ElementContent[]): ElementContent[][] {
  const lines: ElementContent[][] = [];
  let open: ElementContent[] = [];
  // The group being filled is held rather than indexed: it is the one the last newline opened, and it
  // is where every character up to the next one belongs.
  lines.push(open);
  for (const child of children) {
    const groups = splitLines(child);
    for (const [index, group] of groups.entries()) {
      if (index > 0) {
        open = [];
        lines.push(open);
      }
      open.push(...group);
    }
  }
  // A fence's final newline ends the last line rather than opening an empty one, which the gutter
  // would otherwise number as a line the model never wrote.
  const last = lines[lines.length - 1];
  if (lines.length > 1 && last !== undefined && isBlank(last)) lines.pop();
  return lines;
}

function splitLines(node: ElementContent): ElementContent[][] {
  if (node.type === "text") {
    // One group per line the newline describes, each holding its own copy of the text node.
    return (node.value ?? "").split("\n").map((value) => [{ ...node, value }]);
  }
  if (node.type === "element") {
    // A token whose span crosses a break is kept on both halves, class and all, by cloning the
    // wrapper around each of the lines its children fall on.
    return toLines(node.children).map((children) => [{ ...node, children }]);
  }
  // A comment or a doctype holds no character of the source; it rides along with the line around it.
  return [[node]];
}

function isBlank(group: ElementContent[]): boolean {
  return group.every((node) => {
    if (node.type === "text") return (node.value ?? "").length === 0;
    // The highlighter's span is still around each line it painted, so an empty line reads as an
    // element with nothing under it rather than as nothing at all.
    if (node.type === "element") return isBlank(node.children);
    return true;
  });
}
