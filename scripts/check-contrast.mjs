#!/usr/bin/env node
// The contrast contract, checked statically off the token file.
//
// tokens.css is allowed to state a colour and nothing else is, which is what let an ink get as low
// as 2.3:1 without anyone noticing: no component owns the bad pair, the theme does. Every grey and
// every semantic colour in the window is read at `--size-micro` through `--size-quiet` — 11 to 13
// px — which is small text under WCAG 1.4.3 and therefore needs 4.5:1, not the 3:1 reserved for
// large headings. So the floor is asserted where the colours are actually declared.
//
// Since every interface style repaints the whole window rather than only the accent, a token is
// no longer one colour per mode: `--bg-panel` is a different colour for every style. So the contract
// is evaluated once per mode and style — each mode × each accent — and the grounds it reads are the ones that
// accent actually paints. A tint tuned for Mint that drops `--text-faint` under the floor on a Bee
// sidebar is exactly the bug this file exists to catch.
//
// Two kinds of pair are asserted, and the difference between them is how a window marks a selection.
// A SURFACE carries any word the app puts on it, so every grey, every semantic colour and every
// style's ink are checked against every surface — the last because a code fence is highlighted from
// all of them at once, in whichever style is live. A FILL is `--accent-soft` laid on a surface, which
// is what a selected row and a link chip are drawn with, and it costs the ink standing on it some
// contrast: in dark mode the fill is LIGHTER than the card behind it, in light mode DARKER than the
// page. A fill is a promotion, so the rule that paints one swaps the ink too — `--accent-ink` for a
// chip, one of the three promoted greys for a row — and only those inks are asserted against fills.
// `--bg-hover` is absent from both lists for the same reason: every rule that paints it also swaps the
// ink to `--text` or `--text-strong`, which is the point of a hover.
//
// The last section is not contrast but fidelity: the mixing percentages below are chosen so that
// the default style's tinted grounds land on the pixels measured out of docs/reference-ui, so the
// gate also proves that the accent machinery has not drifted away from the reference.
import { readFileSync } from "node:fs";

const TOKENS = "apps/desktop/src/styles/tokens.css";
const AA = 4.5;
/** How far a tinted ground may sit from the measured reference pixel before the calibration claim
 * in tokens.css is false. Browsers quantize oklab mixing and this script does too; two units of an
 * eight-bit channel is that rounding, and nothing a reviewer could see. */
const CALIBRATION_SLACK = 2;

// ---- colour maths, the same path a browser takes for `color-mix(in oklab, …)` ----
const srgbToLinear = (v) =>
  v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
const linearToSrgb = (v) =>
  v <= 0.0031308 ? 12.92 * v : 1.055 * v ** (1 / 2.4) - 0.055;
// Ottosson's pair, as css-color-4 defines it. The inverses are computed from the forwards rather
// than transcribed, because a mistyped constant here would silently misplace every derived token.
const TO_LMS = [
  [0.4122214708, 0.5363325363, 0.0514459929],
  [0.2119034982, 0.6806995451, 0.1073969566],
  [0.0883024619, 0.2817188376, 0.6299787005],
];
const LMS_TO_OKLAB = [
  [0.2104542553, 0.793617785, -0.0040720468],
  [1.9779984951, -2.428592205, 0.4505937099],
  [0.0259040371, 0.7827717662, -0.8086757625],
];
const invert = (m) => {
  const [a, b, c, d, e, f, g, h, i] = m.flat();
  const det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
  return [
    [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
    [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
    [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
  ];
};
const apply = (m, v) =>
  m.map((row) => row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);
const FROM_LMS = invert(TO_LMS);
const OKLAB_TO_LMS = invert(LMS_TO_OKLAB);
const toOklab = (rgb) =>
  apply(
    LMS_TO_OKLAB,
    apply(
      TO_LMS,
      rgb.map((v) => srgbToLinear(v / 255)),
    ).map(Math.cbrt),
  );
const fromOklab = (lab) =>
  apply(
    FROM_LMS,
    apply(OKLAB_TO_LMS, lab).map((v) => v ** 3),
  ).map((v) => linearToSrgb(v) * 255);

const parse = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
const name = (rgb) =>
  `#${rgb.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
const luminance = (rgb) => {
  const [r, g, b] = rgb.map((v) => srgbToLinear(v / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

/** A colour with the alpha it arrives with: `--accent-soft` is a 16% fill, not a colour, and it only
 * becomes a ground once something has been painted behind it. */
const TRANSPARENT = { rgb: [0, 0, 0], alpha: 0 };
const composite = (layer, ground) => ({
  rgb: ground.rgb.map(
    (c, i) => layer.rgb[i] * layer.alpha + c * (1 - layer.alpha),
  ),
  alpha: 1,
});

// ---- read the token file ----
// Later blocks win, exactly as they do in the cascade. The accent families are declared for light at
// `:root` and lifted for dark by a `[data-theme-mode="dark"]` block further down, and each style's
// own block re-points `--accent`, so a token's value depends on both axes at once.
const css = readFileSync(TOKENS, "utf8");
const declarations = (body) =>
  Object.fromEntries(
    [...body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map(([, token, value]) => [
      token,
      value.trim(),
    ]),
  );
const blocks = [
  ...css.matchAll(
    /(:root|\[data-(?:theme-mode|accent)(?:="(\w+)")?\])\s*\{([\s\S]*?)\}/g,
  ),
].map(([, selector, value, body]) => ({
  selector,
  value,
  decls: declarations(body),
}));

/** The declarations that answer for one style in one mode, in cascade order. */
const forCombination = (mode, accent) => {
  const merged = {};
  for (const block of blocks) {
    const matches =
      block.selector === ":root" ||
      (block.selector.startsWith("[data-theme-mode") && block.value === mode) ||
      (block.selector.startsWith("[data-accent") &&
        (block.value === undefined || block.value === accent));
    if (matches) Object.assign(merged, block.decls);
  }
  return merged;
};

/** Split a value's arguments on the commas that are not inside a nested call. */
const splitArgs = (source) => {
  const parts = [];
  let depth = 0;
  let current = "";
  for (const character of source) {
    if (character === "(") depth += 1;
    if (character === ")") depth -= 1;
    if (character === "," && depth === 0) {
      parts.push(current.trim());
      current = "";
      continue;
    }
    current += character;
  }
  parts.push(current.trim());
  return parts;
};

/** Whitespace inside a declaration's value means nothing to CSS and everything to this parser, and
 * Prettier wraps a long `color-mix` across lines without asking. Read the value flat. */
const flatten = (value) =>
  value
    .replace(/\s+/g, " ")
    .replace(/\(\s+/g, "(")
    .replace(/\s+\)/g, ")")
    .trim();

/** Any CSS value this file uses, down to a colour and an alpha. `null` means "not a colour" — a
 * length, a font stack, a quoted theme name — and a token that should have been a colour failing to
 * resolve is reported as a failure, not quietly skipped. */
const resolve = (tokens, value, seen = new Set()) => {
  const source = flatten(value);
  if (/^#[0-9a-f]{6}$/i.test(source)) return { rgb: parse(source), alpha: 1 };
  if (source === "transparent") return TRANSPARENT;
  const reference = /^var\((--[\w-]+)\)$/.exec(source);
  if (reference) {
    const token = reference[1];
    // A token's own name is not yet in `seen` when this is called for it, so a self-reference is
    // caught on the next hop instead of recursing forever.
    if (seen.has(token) || tokens[token] === undefined) return null;
    return resolve(tokens, tokens[token], new Set([...seen, token]));
  }
  const mix = /^color-mix\(in\s+oklab\s*,([\s\S]*)\)$/i.exec(source);
  if (mix) {
    // The colourspace argument is already off the front, so what remains is the two colours.
    const args = splitArgs(mix[1]);
    if (args.length !== 2) return null;
    const percent = args.map((arg) => /\s(-?[\d.]+)%$/.exec(arg)?.[1]);
    const color = args.map((arg, i) =>
      percent[i] === undefined ? arg : arg.replace(/\s-?[\d.]+%$/, ""),
    );
    const first = resolve(tokens, color[0], seen);
    const second = resolve(tokens, color[1], seen);
    if (first === null || second === null) return null;
    // An unqualified mix is 50/50; a qualified one leaves the other side to take the rest.
    const p =
      percent[0] !== undefined
        ? Number(percent[0]) / 100
        : percent[1] !== undefined
          ? 1 - Number(percent[1]) / 100
          : 0.5;
    // Premultiplied, which is what makes a mix with `transparent` keep the other colour's hue
    // instead of sliding toward black.
    const alpha = p * first.alpha + (1 - p) * second.alpha;
    if (alpha === 0) return TRANSPARENT;
    const [a, b] = [toOklab(first.rgb), toOklab(second.rgb)];
    const lab = [0, 1, 2].map(
      (i) => (p * first.alpha * a[i] + (1 - p) * second.alpha * b[i]) / alpha,
    );
    return { rgb: fromOklab(lab), alpha };
  }
  return null;
};

const GROUND_TOKENS = [
  "--bg",
  "--bg-panel",
  "--bg-elevated",
  "--bg-sunken",
  "--code-bg",
];
const TEXT_INKS = [
  "--text",
  "--text-strong",
  "--text-muted",
  "--text-faint",
  "--success",
  "--warning",
  "--danger",
];
const ACCENTS = ["forest", "mint", "bee", "parchment", "rain"];
// The surfaces a style's translucent fill is laid on: the page, the sidebar, a card. Not
// `--bg-sunken`, which is a code gutter and the user's own bubble — both of which paint their own
// label rather than a chip, so no fill is ever stacked on them.
const FILL_BASES = ["--bg", "--bg-panel", "--bg-elevated"];
// The greys and every style ink, because a code fence is syntax-highlighted from all of them
// at once whatever the selected style: `--style-bee-ink` is a type name in every mode.
const SURFACE_INKS = [
  ...TEXT_INKS,
  ...ACCENTS.map((family) => `--style-${family}-ink`),
];
// What a selected region is actually read in. A fill is a promotion, and the ink it carries is
// promoted with it: a chip paints `--accent-ink` for itself, and the row a fill marks uses the body
// text, the heading text or the rung above faint — `global.css` swaps the ink in every rule that
// paints `--accent-soft`, which is the same contract `--bg-hover` has always held. `--text-faint`
// therefore does not appear: in dark mode it sits one rung under the floor a fill costs it, and a
// rule that put a faint word on a style's own tint would be a rule that de-emphasises by erasing.
// Neither do the three semantic colours: a caveat, an alert and a failure are painted on a surface,
// and the moment one is placed inside a selected row it has to be lifted with it.
const FILL_INKS = ["--text", "--text-strong", "--text-muted", "--accent-ink"];

/** Pixels read out of docs/reference-ui by scripts/probe-reference-palette.swift for the DEFAULT
 * style, Mint, for every token that is now tinted rather than stated. The mixing percentages in
 * tokens.css are not free parameters: they were solved from these, so the gate can say out loud that
 * a style repaint has not moved the shipped default away from the reference. Tokens that are not
 * tinted (`--bg` and `--bg-elevated` in light, every grey) are absent, and so is `--text-strong`,
 * which trades an exact match for the dark green the reference shows in order to follow the style. */
const REFERENCE = {
  light: {
    "--bg-panel": "#e0f1ea",
    "--bg-sunken": "#eef5f2",
    "--bg-hover": "#d5e9e0",
    "--border": "#d5e0d9",
    "--border-strong": "#c0cfc6",
    "--ink-quiet": "#98a098",
  },
  dark: {
    "--bg": "#0a110e",
    "--bg-panel": "#112921",
    "--bg-elevated": "#16332a",
    "--bg-sunken": "#080d0b",
    "--bg-hover": "#1a3a30",
    "--border": "#1e3a30",
    "--border-strong": "#2b4c40",
    "--ink-quiet": "#6d8278",
    "--checkpoint": "#2c3632",
    "--checkpoint-active": "#3c4c45",
  },
};

const failures = [];
for (const mode of ["light", "dark"]) {
  for (const accent of ACCENTS) {
    const tokens = forCombination(mode, accent);
    const resolveToken = (label, token) => {
      const layer = resolve(tokens, `var(${token})`);
      if (layer === null)
        failures.push(
          `${mode}/${accent}: ${label} ${token} does not resolve to a color`,
        );
      return layer;
    };

    const grounds = GROUND_TOKENS.map((token) => [
      token,
      resolveToken(token, token),
    ]).filter(([, layer]) => layer);
    // A selected region's ground: the style's own translucent fill laid on an opaque surface.
    const soft = resolveToken(`--style-${accent}`, "--accent-soft");
    const fills = [];
    for (const groundToken of FILL_BASES) {
      const ground = resolveToken(groundToken, groundToken);
      const painted = ground && soft && composite(soft, ground);
      if (painted) fills.push([`${groundToken}+accent-soft`, painted]);
    }

    const check = (label, layer, pairs) => {
      if (!layer || layer.alpha < 1) return;
      for (const [groundToken, ground] of pairs) {
        if (!ground || ground.alpha < 1) continue;
        const ratio = contrast(layer.rgb, ground.rgb);
        if (ratio < AA) {
          failures.push(
            `${mode}/${accent}: ${label} ${name(layer.rgb)} on ${groundToken} ${name(ground.rgb)} is ${ratio.toFixed(2)}:1 (needs ${AA}:1)`,
          );
        }
      }
    };

    for (const ink of SURFACE_INKS) check(ink, resolveToken(ink, ink), grounds);
    for (const ink of FILL_INKS) check(ink, resolveToken(ink, ink), fills);

    // The calibration claim, once: the default style's grounds are the measured reference pixels.
    if (accent === "mint") {
      for (const [token, measured] of Object.entries(REFERENCE[mode])) {
        const layer = resolveToken(token, token);
        if (!layer) continue;
        const painted = name(layer.rgb);
        const drift = parse(painted).map((v, i) =>
          Math.abs(v - parse(measured)[i]),
        );
        if (drift.some((d) => d > CALIBRATION_SLACK)) {
          failures.push(
            `${mode}: ${token} paints ${painted} but ${measured} is what docs/reference-ui measures — the mix percentage is no longer Mint-faithful`,
          );
        }
      }
    }
  }
}

if (failures.length) {
  console.error("contrast contract failed:\n  " + failures.join("\n  "));
  process.exit(1);
}
console.log(
  `OK — ${TEXT_INKS.length} greys + ${ACCENTS.length} style inks clear ${AA}:1 on every surface each of ` +
    `${ACCENTS.length} styles paints, in both modes, and Mint's grounds still match the reference`,
);
