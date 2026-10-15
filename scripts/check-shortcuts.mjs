// The contract behind the keyboard hints: a key the app prints is a key the app answers.
//
// Two shortcuts shipped as decoration before either existed — a literal `⌥V` interpolated into the
// composer's mic label, and the Voice page's chip rendered with no handler behind it. Neither broke
// a build or a type, because the glyphs were strings and the missing listener was an absence. This
// script turns both into failures: one table owns the bindings, the window's handler set has to
// match it, and no copy or component string may state a key the table does not define.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const SRC = "apps/desktop/src";
const rel = (file) => file.slice(SRC.length + 1);

/** The two files allowed to show a modifier, because both derive it from the table: the label
 * builder and the chip renderer. */
const GLYPH_OWNERS = new Set(["lib/shortcuts.ts", "components/controls.tsx"]);

const fail = (msg) => {
  console.error(`FAIL ${msg}`);
  process.exit(1);
};

/** Comments removed, so a sentence explaining a binding is not read as a claim of one. */
const codeOnly = (text) =>
  text
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("//"))
    .join("\n");

function* sources(dir) {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) yield* sources(path);
    else if (/\.(?:ts|tsx|json)$/.test(entry)) yield path;
  }
}

function matchingBrace(text, open) {
  let depth = 0;
  for (let i = open; i < text.length; i += 1) {
    if (text[i] === "{") depth += 1;
    else if (text[i] === "}") {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** An object literal's own keys: the ones at the shallowest indent inside its braces, so a nested
 * field or a line of a handler body cannot be mistaken for one. */
function objectKeys(text, startMarker) {
  const start = text.indexOf(startMarker);
  if (start === -1) return null;
  const open = text.indexOf("{", start);
  const close = matchingBrace(text, open);
  if (close === -1) return null;
  const body = text.slice(open + 1, close).replace(/^\n/, "");
  const found = [...body.matchAll(/^([ \t]+)(\S[^:\n]*):/gm)]
    .map(([, indent, key]) => [indent.length, key])
    .filter(([, key]) => /^\w+$/.test(key));
  const shallowest = Math.min(...found.map(([indent]) => indent));
  return found.filter(([indent]) => indent === shallowest).map(([, key]) => key);
}

const shortcuts = codeOnly(readFileSync(`${SRC}/lib/shortcuts.ts`, "utf8"));
const table = objectKeys(shortcuts, "export const SHORTCUTS");
if (!table?.length) fail("could not read the SHORTCUTS table — the check itself broke");

const union = shortcuts.match(/export type ShortcutId =([^;]+);/)?.[1] ?? "";
const declared = [...union.matchAll(/"(\w+)"/g)].map((m) => m[1]);
const unlisted = declared.filter((id) => !table.includes(id));
if (unlisted.length) fail(`${unlisted.join(", ")} is a ShortcutId with no entry in SHORTCUTS`);

// A binding no handler answers is a hint that does nothing when followed — the bug this shipped with.
const handlers = objectKeys(codeOnly(readFileSync(`${SRC}/App.tsx`, "utf8")), "useShortcuts(");
if (!handlers?.length) fail("App.tsx has no useShortcuts({ … }) block to read the handlers from");
const unanswered = table.filter((id) => !handlers.includes(id));
if (unanswered.length) {
  fail(`SHORTCUTS defines ${unanswered.join(", ")} but no handler in App.tsx answers it`);
}

// Every key a control names out loud has to be one of those bindings.
const referenced = [];
for (const file of sources(SRC)) {
  const text = codeOnly(readFileSync(file, "utf8"));
  for (const m of text.matchAll(/shortcutLabel\(\s*"(\w+)"|shortcut:\s*"(\w+)"/g)) {
    referenced.push([file, m[1] ?? m[2]]);
  }
}
if (!referenced.length) fail("no control names a shortcut, so the wiring went missing");
for (const [file, id] of referenced) {
  if (!table.includes(id)) fail(`${rel(file)} advertises "${id}", which is not a binding in SHORTCUTS`);
}

// And no copy states a key of its own: `shortcutLabel` builds the string from the table, so a glyph
// in a component or a catalog is either a second source of truth or a promise with nothing behind it.
const GLYPH = /[⌘⌥⇧⌃]|Ctrl\+|\{\{shortcut\}\}/;
for (const file of sources(SRC)) {
  if (GLYPH_OWNERS.has(rel(file))) continue;
  const lines = readFileSync(file, "utf8").split("\n");
  const body = file.endsWith(".json") ? lines : codeOnly(lines.join("\n")).split("\n");
  const line = body.find((l) => GLYPH.test(l));
  if (line) fail(`${rel(file)} puts a key in copy rather than in SHORTCUTS: ${line.trim()}`);
}

console.log(
  `OK — ${table.length} bindings (${table.join(", ")}) each answered by a handler, and all ` +
    `${referenced.length} hints that name a key read it from the table`,
);
