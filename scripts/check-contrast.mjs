#!/usr/bin/env node
// The contrast contract, checked statically off the token file.
//
// tokens.css is allowed to state a colour and nothing else is, which is what let an ink get as low
// as 2.3:1 without anyone noticing: no component owns the bad pair, the theme does. Every grey and
// every semantic colour in the window is read at `--size-micro` through `--size-quiet` — 11 to 13
// px — which is small text under WCAG 1.4.3 and therefore needs 4.5:1, not the 3:1 reserved for
// large headings. So the floor is asserted where the colours are actually declared.
//
// The grounds are the five surfaces a word can sit on in that mode. `--bg-hover` is deliberately
// absent: every rule that paints it also swaps the ink to `--text` or `--text-strong`, which is the
// point of a hover. Accent inks are checked against `--accent-soft` too, composited the way the
// browser does it, because a link chip paints that fill for itself and the fill is not the accent —
// in dark mode it is LIGHTER than the card behind it, which costs a pale ink contrast instead of
// gaining it.
import { readFileSync } from "node:fs";

const TOKENS = "apps/desktop/src/styles/tokens.css";
const AA = 4.5;

const srgbToLinear = (v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
const luminance = ([r, g, b]) => 0.2126 * srgbToLinear(r / 255) + 0.7152 * srgbToLinear(g / 255) + 0.0722 * srgbToLinear(b / 255);
const parse = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};
const name = (rgb) => `#${rgb.map((v) => v.toString(16).padStart(2, "0")).join("")}`;

// ---- read the token file ----
// Later blocks win, exactly as they do in the cascade: the accent families are declared for light at
// `:root` and lifted for dark by the `[data-theme-mode="dark"]` block at the bottom of the file.
const css = readFileSync(TOKENS, "utf8");
const declarations = (body) =>
  Object.fromEntries(
    [...body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map(([, token, value]) => [token, value.trim()]),
  );
const blocks = [...css.matchAll(/(:root|\[data-theme-mode="(light|dark)"\])\s*\{([\s\S]*?)\}/g)].map(
  ([, selector, mode, body]) => ({ selector, mode, decls: declarations(body) }),
);

const forMode = (mode) => {
  const merged = {};
  for (const block of blocks) {
    if (block.selector === ":root" || block.mode === mode) Object.assign(merged, block.decls);
  }
  return merged;
};

const resolve = (tokens, token, seen = new Set()) => {
  const value = tokens[token];
  if (value === undefined) return null;
  const ref = value.match(/^var\((--[\w-]+)\)$/);
  if (ref) {
    if (seen.has(token)) return null;
    seen.add(token);
    return resolve(tokens, ref[1], seen);
  }
  return /^#[0-9a-f]{6}$/i.test(value) ? value : null;
};

const GROUND_TOKENS = ["--bg", "--bg-panel", "--bg-elevated", "--bg-sunken", "--code-bg"];
const TEXT_INKS = [
  "--text",
  "--text-strong",
  "--text-muted",
  "--text-faint",
  "--success",
  "--warning",
  "--danger",
];
const ACCENTS = ["forest", "mint", "bee", "parchment"];

const failures = [];
for (const mode of ["light", "dark"]) {
  const tokens = forMode(mode);
  const grounds = GROUND_TOKENS.map((t) => [t, parse(resolve(tokens, t))]);
  // Declared in the `[data-accent]` block rather than a mode block, so it is read off the file.
  const softPercent = Number(/--accent-soft: color-mix\(in oklab, var\(--accent\) (\d+)%/.exec(css)[1]) / 100;

  const check = (inkToken, rgb, groundPairs) => {
    for (const [groundToken, ground] of groundPairs) {
      const ratio = contrast(rgb, ground);
      if (ratio < AA) {
        failures.push(
          `${mode}: ${inkToken} ${name(rgb)} on ${groundToken} ${name(ground)} is ${ratio.toFixed(2)}:1 (needs ${AA}:1)`,
        );
      }
    }
  };

  for (const ink of TEXT_INKS) check(ink, parse(resolve(tokens, ink)), grounds);

  // An accent's soft fill over each of the surfaces a chip can be drawn on: the transcript column,
  // a card, and a sunken gutter. Alpha compositing, which is what the browser does with a 16% fill.
  const chipGrounds = ["--bg", "--bg-elevated", "--bg-sunken"];
  for (const accent of ACCENTS) {
    const ink = resolve(tokens, `--style-${accent}-ink`);
    const base = resolve(tokens, `--style-${accent}`);
    const pairs = [...grounds];
    for (const groundToken of chipGrounds) {
      const ground = parse(resolve(tokens, groundToken));
      const tint = parse(base);
      pairs.push([
        `${groundToken}+${accent}-soft`,
        ground.map((v, i) => Math.round(tint[i] * softPercent + v * (1 - softPercent))),
      ]);
    }
    check(`--style-${accent}-ink`, parse(ink), pairs);
  }
}

if (failures.length) {
  console.error("contrast contract failed:\n  " + failures.join("\n  "));
  process.exit(1);
}
console.log(
  `OK — every ink clears ${AA}:1 on every surface it is read against, in both modes ` +
    `(${TEXT_INKS.length} greys + ${ACCENTS.length} accents x 2)`,
);
