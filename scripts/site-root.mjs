#!/usr/bin/env node
// GitHub Pages publishes this repo from the branch root, so /FreeCode/ is served
// from index.html at the root while site/ stays the canonical landing page.
// The Chinese copy works the same way: site/zh/index.html is the source, zh/index.html
// is what the root publishes at /FreeCode/zh/.
// This regenerates those copies; --check fails when any of them have drifted.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);

const copies = [
  {
    source: "site/index.html",
    target: "index.html",
    rewrites: [
      ['="assets/', '="site/assets/'],
      ['="video/', '="site/video/'],
    ],
  },
  {
    source: "site/zh/index.html",
    target: "zh/index.html",
    rewrites: [
      ['="../assets/', '="../site/assets/'],
      ['="../video/', '="../site/video/'],
    ],
  },
];

function build(copy) {
  const banner = `    <!-- Generated from ${copy.source} by scripts/site-root.mjs — edit that file, then regenerate. -->\n`;
  let html = readFileSync(new URL(copy.source, root), "utf8").replace(
    "<!doctype html>\n",
    `<!doctype html>\n${banner}`,
  );
  for (const [from, to] of copy.rewrites) html = html.replaceAll(from, to);
  return html;
}

const check = process.argv.includes("--check");
const stale = [];

for (const copy of copies) {
  const generated = build(copy);
  const target = new URL(copy.target, root);
  if (check) {
    let current = null;
    try {
      current = readFileSync(target, "utf8");
    } catch {
      current = null;
    }
    if (current !== generated) stale.push(copy.target);
  } else {
    mkdirSync(dirname(fileURLToPath(target)), { recursive: true });
    writeFileSync(target, generated);
    console.log(`regenerated ${copy.target} from ${copy.source}`);
  }
}

if (check) {
  if (stale.length) {
    console.error(`${stale.join(", ")} ${stale.length > 1 ? "are" : "is"} stale — run node scripts/site-root.mjs`);
    process.exit(1);
  }
  console.log(`root copies match ${copies.map((c) => c.source).join(", ")}`);
}
