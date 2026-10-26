#!/usr/bin/env python3
"""Render the Windows installer's two images — the first-download art for a Windows build.

macOS gets a window it can paint; NSIS draws two bitmaps instead, and both are the same shop front:
the forest ground, the mint accent, the rolled themed logo and the wordmark.

* `nsis-sidebar.bmp` (164×314) — the panel down the left of the welcome and finish pages: the logo
  large on the ground the app opens in, the name, and one line saying what the app is.
* `nsis-header.bmp` (150×57) — the small strip NSIS draws at the top right of the white installer
  pages. It is a light ground here on purpose: a dark block in the corner of a white page reads as a
  hole in the page rather than as the app.

Both are 24-bit BMPs because that is what NSIS decodes, and both roll one of the three themed logos
(15–17) per run, the same way the Dock and the DMG background do.

Run from the repository root:

    python3 scripts/render-windows-installer-art.py

They are wired into `bundle.windows.nsis` in `tauri.conf.json`; nothing copies them by hand.
"""

from pathlib import Path
import random
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

import theme_logos

MASCOTS = Path("apps/desktop/src/assets/mascots")
ICONS = Path("apps/desktop/src-tauri/icons")
SIDEBAR = ICONS / "nsis-sidebar.bmp"
HEADER = ICONS / "nsis-header.bmp"
PREVIEW = Path("/tmp/windows-installer-preview.png")
# NSIS's documented sizes, in pixels, drawn at 2× and downsampled so the type stays crisp.
SIDE_W, SIDE_H = 164, 314
HEAD_W, HEAD_H = 150, 57
SS = 2
LOGOS = (15, 16, 17)
CORNER = 0.225

# styles/tokens.css: the ground the window opens in and the accent it paints with, read from the logo
# module so the installer and the logo it shows come from one palette. The light pair is the ground
# the header is drawn on, where NSIS puts it on a white page.
FOREST = theme_logos.THEMES["forest"]
GROUND_TOP, GROUND_BOTTOM, MINT = FOREST["top"], FOREST["bottom"], FOREST["glow"]
NAME = (243, 249, 245)
NOTE = (150, 186, 171)
LIGHT_TOP = (250, 251, 249)
LIGHT_BOTTOM = (238, 244, 239)
INK = (26, 36, 32)


def font(size: int, bold: bool) -> ImageFont.FreeTypeFont:
    name = "Helvetica Bold" if bold else "Helvetica"
    for candidate in (
        f"/System/Library/Fonts/{name}.ttc",
        f"/System/Library/Fonts/Supplemental/{name}.ttc",
        "/System/Library/Fonts/Helvetica.ttc",
    ):
        path = Path(candidate)
        if path.is_file():
            return ImageFont.truetype(str(path), size)
    raise SystemExit("no Helvetica on this Mac — the wordmark needs a system face")


def ramp(size: tuple[int, int], top, bottom, glow=None, glow_alpha: float = 0.0) -> Image.Image:
    """A ground: a top-to-bottom ramp, lit by a round spotlight when one is asked for."""
    w, h = size
    column = np.linspace(0.0, 1.0, h)[:, None, None]
    band = np.array(top) + (np.array(bottom) - np.array(top)) * column
    field = np.broadcast_to(band, (h, w, 3)).astype(np.float64)
    if glow is not None and glow_alpha:
        yy, xx = np.mgrid[0:h, 0:w]
        distance = np.hypot(xx - 0.5 * w, yy - 0.22 * h) / (0.70 * w)
        weight = (np.clip(1.0 - distance, 0.0, 1.0) ** 2 * glow_alpha)[..., None]
        field = field * (1 - weight) + np.array(glow) * weight
    return Image.fromarray(np.clip(field, 0, 255).astype(np.uint8), "RGB").convert("RGBA")


def logo_tile(path: Path, side: int) -> Image.Image:
    """The rolled logo as a rounded square, drawn at 2× and downsampled, corners cut."""
    big = side * 2
    tile = Image.open(path).convert("RGBA").resize((big, big), Image.LANCZOS)
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, big - 1, big - 1], radius=int(big * CORNER), fill=255
    )
    tile.putalpha(mask)
    return tile.resize((side, side), Image.LANCZOS)


def centred(draw: ImageDraw.ImageDraw, text: str, fnt, y: int, w: int, fill) -> None:
    box = draw.textbbox((0, 0), text, font=fnt)
    draw.text(((w - (box[2] - box[0])) // 2, y), text, font=fnt, fill=fill)


def sidebar(root: Path, number: int) -> Image.Image:
    w, h = SIDE_W * SS, SIDE_H * SS
    img = ramp((w, h), GROUND_TOP, GROUND_BOTTOM, MINT, 0.20)
    draw = ImageDraw.Draw(img)

    tile = logo_tile(root / MASCOTS / f"mascot-{number:02d}.webp", 92 * SS)
    img.alpha_composite(tile, ((w - tile.width) // 2, 40 * SS))
    centred(draw, "FreeCode", font(26 * SS, bold=True), 152 * SS, w, NAME)

    # The accent rule between the name and the sentence, the same hairline the tiles carry.
    rule_w, rule_y = 46 * SS, 190 * SS
    draw.line([(w - rule_w) // 2, rule_y, (w + rule_w) // 2, rule_y], fill=MINT, width=2 * SS)

    line = font(13 * SS, bold=False)
    centred(draw, "Local-first AI", line, 212 * SS, w, NOTE)
    centred(draw, "coding agent", line, 230 * SS, w, NOTE)
    return img.resize((SIDE_W, SIDE_H), Image.LANCZOS)


def header(root: Path, number: int) -> Image.Image:
    w, h = HEAD_W * SS, HEAD_H * SS
    img = ramp((w, h), LIGHT_TOP, LIGHT_BOTTOM)
    draw = ImageDraw.Draw(img)

    tile = logo_tile(root / MASCOTS / f"mascot-{number:02d}.webp", 36 * SS)
    img.alpha_composite(tile, (10 * SS, (h - tile.height) // 2))
    draw.text((54 * SS, 12 * SS), "FreeCode", font=font(16 * SS, bold=True), fill=INK)
    draw.text((54 * SS, 32 * SS), "AI coding", font=font(10 * SS, bold=False), fill=(110, 130, 120))
    return img.resize((HEAD_W, HEAD_H), Image.LANCZOS)


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    roll = random.SystemRandom()
    side_number, head_number = roll.choice(LOGOS), roll.choice(LOGOS)
    print(f"rolled themed logos {side_number} (sidebar) and {head_number} (header)")

    side = sidebar(root, side_number).convert("RGB")
    head = header(root, head_number).convert("RGB")
    side.save(root / SIDEBAR, "BMP")
    head.save(root / HEADER, "BMP")

    # What the two look like side by side, so a change can be judged without a Windows build.
    sheet = Image.new("RGB", (SIDE_W + HEAD_W + 40, max(SIDE_H, HEAD_H) + 20), (255, 255, 255))
    sheet.paste(side, (10, 10))
    sheet.paste(head, (SIDE_W + 30, 10))
    sheet.save(root / PREVIEW)
    print(f"wrote {root / SIDEBAR} and {root / HEADER}; {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())