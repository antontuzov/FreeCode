#!/usr/bin/env node
// The locale contract, checked statically: both catalogs hold the same keys with the same
// {{placeholders}}, and no component asks for a key that neither catalog defines. `translate()`
// falls back to printing the key itself, which is exactly the failure a screenshot would catch and
// a type-checker would not.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const I18N = "apps/desktop/src/i18n";
const SRC = "apps/desktop/src";

const catalog = (name) => JSON.parse(readFileSync(join(I18N, `${name}.json`), "utf8"));
const en = catalog("en");
const zh = catalog("zh");

const placeholders = (s) => [...s.matchAll(/{{(\w+)}}/g)].map((m) => m[1]).sort().join(",");

const failures = [];
const missing = Object.keys(en).filter((k) => !(k in zh));
const extra = Object.keys(zh).filter((k) => !(k in en));
if (missing.length) failures.push(`missing from zh.json: ${missing.join(", ")}`);
if (extra.length) failures.push(`not present in en.json: ${extra.join(", ")}`);
const drift = Object.keys(en).filter((k) => k in zh && placeholders(en[k]) !== placeholders(zh[k]));
if (drift.length) failures.push(`placeholder drift in: ${drift.join(", ")}`);

const files = [];
const walk = (dir) => {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) walk(path);
    else if (/\.(ts|tsx)$/.test(entry)) files.push(path);
  }
};
walk(SRC);

// `translate(locale, "a.b")` and a page's own shorthand, e.g. `const t = (key) => translate(locale,
// `settings.${key}`)`. The shorthand's prefix is read off its own definition, so a typo in either
// still fails.
const KEY_USE = /(?:translate\(\s*[\w.]+\s*,\s*|[^.\w]t\(\s*)"([\w.]+)"/g;
const PREFIX = /`([\w.]+)\$\{key\}`/g;
for (const file of files) {
  const source = readFileSync(file, "utf8");
  const prefixes = ["", ...[...source.matchAll(PREFIX)].map((m) => m[1])];
  for (const [, key] of source.matchAll(KEY_USE)) {
    const defined = prefixes.some((p) => p + key in en);
    if (!defined) failures.push(`${file} asks for "${key}", which en.json does not define`);
  }
}

if (failures.length) {
  for (const f of failures) console.error(`FAIL ${f}`);
  process.exit(1);
}
console.log(`OK — ${Object.keys(en).length} keys in both catalogs, every literal lookup defined`);
