#!/usr/bin/env node
// Emits the app icon as a 1024×1024 RGBA PNG, then `pnpm tauri icon` fans it out into the
// per-platform sets. It is checked in rather than shipped as a binary blob so the mark can be
// changed in review: the geometry is the source of truth, and a rebuild is deterministic.
//
// The mark is the ternary alphabet the model is quantized in — three bars for −1, 0, +1, with a
// caret for the agent that runs on them.
import { deflateSync, crc32 } from "node:zlib";
import { writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SIZE = 1024;
const OUT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "apps", "desktop", "src-tauri", "icons", "source.png");

const CORNER_RADIUS = 180;
// Top of the tile to its bottom: a deep forest that falls into ink, so the mint mark is the
// brightest thing on the icon at any thumbnail size.
const TOP = [16, 56, 46];
const BOTTOM = [7, 20, 22];
const MINT = [123, 227, 192];

/** Solid axis-aligned rectangle, antialiased against whatever is already in the buffer. */
function paint(buf, x0, y0, w, h, [r, g, b], alpha = 1) {
  for (let y = Math.floor(y0); y < Math.ceil(y0 + h); y += 1) {
    for (let x = Math.floor(x0); x < Math.ceil(x0 + w); x += 1) {
      if (x < 0 || y < 0 || x >= SIZE || y >= SIZE) continue;
      const coverageX = Math.min(1, Math.max(0, x + 1 - x0)) * Math.min(1, Math.max(0, x0 + w - x));
      const coverageY = Math.min(1, Math.max(0, y + 1 - y0)) * Math.min(1, Math.max(0, y0 + h - y));
      const a = alpha * coverageX * coverageY;
      const i = (y * SIZE + x) * 4;
      buf[i] = Math.round(buf[i] * (1 - a) + r * a);
      buf[i + 1] = Math.round(buf[i + 1] * (1 - a) + g * a);
      buf[i + 2] = Math.round(buf[i + 2] * (1 - a) + b * a);
      buf[i + 3] = Math.min(255, Math.round(buf[i + 3] + a * 255));
    }
  }
}

function insideRoundedTile(x, y) {
  const r = CORNER_RADIUS;
  const cx = Math.min(Math.max(x, r), SIZE - r);
  const cy = Math.min(Math.max(y, r), SIZE - r);
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r || (x >= r && x <= SIZE - r) || (y >= r && y <= SIZE - r);
}

const raw = Buffer.alloc(SIZE * SIZE * 4);
for (let y = 0; y < SIZE; y += 1) {
  const t = y / (SIZE - 1);
  const base = TOP.map((c, i) => Math.round(c * (1 - t) + BOTTOM[i] * t));
  for (let x = 0; x < SIZE; x += 1) {
    if (!insideRoundedTile(x, y)) continue;
    const i = (y * SIZE + x) * 4;
    [raw[i], raw[i + 1], raw[i + 2], raw[i + 3]] = [...base, 255];
  }
}

// Three bars, longest at the top, centred as a block.
const BAR_H = 78;
const BAR_GAP = 46;
const BAR_W = [340, 250, 160];
const CARET_W = 26;
const blockH = BAR_W.length * BAR_H + (BAR_W.length - 1) * BAR_GAP;
const blockW = BAR_W[0] + 74 + CARET_W;
const left = (SIZE - blockW) / 2;
const top = (SIZE - blockH) / 2;
BAR_W.forEach((w, i) => {
  paint(raw, left, top + i * (BAR_H + BAR_GAP), w, BAR_H, MINT);
});
paint(raw, left + BAR_W[0] + 74, top, CARET_W, blockH, MINT, 0.85);

// --- PNG container: IHDR + one filtered scanline per row in IDAT + IEND. -----------------------
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body) >>> 0);
  return Buffer.concat([len, body, crc]);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; // bit depth
ihdr[9] = 6; // RGBA
const scanlines = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y += 1) {
  const row = y * (SIZE * 4 + 1);
  scanlines[row] = 0; // filter: none
  raw.copy(scanlines, row + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", deflateSync(scanlines, { level: 9 })),
  chunk("IEND", Buffer.alloc(0)),
]);

writeFileSync(OUT, png);
console.log(`wrote ${OUT} (${png.length} bytes)`);
