#!/usr/bin/env node
// GitHub Pages publishes this repo from the branch root, so /FreeCode/ is served
// from index.html at the root while site/ stays the canonical landing page.
// This regenerates that root copy; --check fails when the two have drifted.
import { readFileSync, writeFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const source = new URL("site/index.html", root);
const target = new URL("index.html", root);

const banner =
  "    <!-- Generated from site/index.html by scripts/site-root.mjs — edit that file, then regenerate. -->\n";

const generated = readFileSync(source, "utf8")
  .replace("<!doctype html>\n", `<!doctype html>\n${banner}`)
  .replaceAll('="assets/', '="site/assets/')
  .replaceAll('="video/', '="site/video/');

if (process.argv.includes("--check")) {
  if (readFileSync(target, "utf8") !== generated) {
    console.error("index.html is stale — run node scripts/site-root.mjs");
    process.exit(1);
  }
  console.log("index.html matches site/index.html");
} else {
  writeFileSync(target, generated);
  console.log("regenerated index.html from site/index.html");
}
