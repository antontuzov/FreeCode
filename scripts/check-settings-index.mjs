#!/usr/bin/env node
// Settings search is only as good as the index it reads, and that index is a list of catalog stems
// kept beside a page that renders them in JSX. Both directions are checked here: a row the page
// renders but the index omits is a row nobody can find by name, and a stem the index invents renders
// its own key in the result list. `translate()` prints the key when a catalog lacks it, so neither
// mistake fails a type-check — which is why this is a script and not a comment.
import { readFileSync } from "node:fs";

const SETTINGS = "apps/desktop/src/components/Settings.tsx";
const CATALOG = "apps/desktop/src/i18n/en.json";
const PAGE_IDS = ["appearance", "voice", "agent", "model", "reasoning", "shortcuts", "storage"];

const source = readFileSync(SETTINGS, "utf8");
const catalog = JSON.parse(readFileSync(CATALOG, "utf8"));
const failures = [];

const block = source.match(/const CONTROLS: Record<Page, string\[\]> = \{([\s\S]*?)\n\};/);
if (!block) {
  console.error("FAIL could not read CONTROLS out of Settings.tsx — the index has changed shape");
  process.exit(1);
}

// Page → the stems it lists. The block is flat, so each array is read under the key that opens it.
const index = {};
for (const page of PAGE_IDS) {
  const list = block[1].match(new RegExp(`${page}: \\[([\\s\\S]*?)\\]`));
  if (!list) {
    failures.push(`CONTROLS has no entry for the "${page}" page`);
    continue;
  }
  index[page] = [...list[1].matchAll(/"([\w.]+)"/g)].map((m) => m[1]);
}

const indexed = Object.values(index).flat();
for (const [page, keys] of Object.entries(index)) {
  const dupes = keys.filter((k, i) => keys.indexOf(k) !== i);
  if (dupes.length) failures.push(`${page} lists twice: ${dupes.join(", ")}`);
}
for (const key of indexed) {
  if (!(`settings.row.${key}` in catalog)) {
    failures.push(`CONTROLS lists "${key}", which no catalog defines as settings.row.${key}`);
  }
}
const shared = indexed.filter((k, i) => indexed.indexOf(k) !== i);
if (shared.length) failures.push(`a row on two pages, so search cannot say where it is: ${shared.join(", ")}`);

// Every row title the pages render must be findable. A title is either a literal
// `title={t("row.x")}` or a loop over keys that are each a quoted string in the array above it, so
// both spellings are read here — the loop is how the engine toggles and the sampling sliders render.
const literal = [...source.matchAll(/title=\{t\("row\.([\w.]+)"\)\}/g)].map((m) => m[1]);
const rendered = new Set(literal);
for (const [, keys] of source.matchAll(/\(\s*\[([\s\S]*?)\]\s*as const\s*\)\.map\(\(\[key/g)) {
  for (const [, key] of keys.matchAll(/^\s*\[\s*"([\w.]+)",/gm)) rendered.add(key);
}
for (const key of rendered) {
  if (!indexed.includes(key)) {
    failures.push(`Settings.tsx renders a row titled "${key}" that CONTROLS never lists — search cannot find it`);
  }
}

if (failures.length) {
  for (const f of failures) console.error(`FAIL ${f}`);
  process.exit(1);
}
console.log(
  `OK — ${indexed.length} settings rows indexed across ${PAGE_IDS.length} pages, every one in the catalog`,
);
