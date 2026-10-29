#!/usr/bin/env python3
"""Render the DMG background — the first thing a new download opens on.

The installer window is the app's shop front, so it carries the app: the deep forest ground the window
opens in, the mint accent, and the app's own mark. The install window and the app it installs show one
face, which is the whole point of the art — and that face is the mark the volume icon and the app's
bundle carry, not one of the mascots the Dock rolls at launch.

* the app's mark (`icons/source.png`, the duck the bundle icon is cut from), top centre, on a soft
  shadow so a dark ground tile keeps its outline on the dark background;
* the "FreeCode" wordmark under it, in the interface's own cream;
* the arrow on the line the two icon slots sit on, and "Drag to Applications" under them.

The geometry is the one `tauri.conf.json` paints: a 660×400 window with the app at (180, 190) and
the Applications folder at (480, 190). Those two numbers are the icons' **centres**, not their
corners — Finder's `position`, which is what the bundler's AppleScript sets, points at the middle of
an icon with y growing downward from the window's top edge. So an icon of `SLOT` spans
`y 190±64`: the band above y=126 is free for the header, the band below y=254 is free for the note,
and the arrow belongs on y=190 between the two slots rather than under them.

Run from the repository root:

    python3 scripts/render-dmg-background.py
"""

from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

import theme_logos

MARK = Path("apps/desktop/src-tauri/icons/source.png")
OUT = Path("apps/desktop/src-tauri/icons/dmg-background.png")
PREVIEW = Path("/tmp/dmg-background-preview.png")
# tauri.conf.json's dmg windowSize, rendered at 2x and downsampled so the type stays crisp.
W, H = 660, 400
SS = 2
# The two slots the installer paints, from tauri.conf.json — these are the icons' centres, so the
# arrow and the bands are laid out against them rather than against the middle of the canvas.
APP_SLOT = (180, 190)
FOLDER_SLOT = (480, 190)
# Finder's icon size in the volume window, which the bundler sets to 128; the drawn header and note
# keep clear of the boxes `y 190±64` the two icons occupy.
SLOT = 128
# The mark tile: the app's own icon, the same one the bundle carries and the volume shows. It and the
# wordmark have to live in the 126px band above the icons, so the tile is small enough to leave
# room for the name under it.
TILE = 88
TILE_TOP = 16
CORNER = 0.225

# styles/tokens.css: the dark ground the window opens in and the accent it paints with, read from the
# logo module so the install window and the logo it shows are lit from the same palette. The two text
# colours are the installer's own — cream for the name, a muted mint for the line under the arrow.
FOREST = theme_logos.THEMES["forest"]
GROUND_TOP, GROUND_BOTTOM = FOREST["top"], FOREST["bottom"]
MINT = FOREST["glow"]
NAME = (243, 249, 245)
NOTE = (150, 186, 171)


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


def logo_tile(path: Path, side: int) -> tuple[Image.Image, int]:
    """The app's mark as the rounded square the installer draws, its corners cut and a soft shadow
    under it — a tile on a ground of the same family would otherwise sink into the background.

    Returns the plate and the transparent margin it carries, so the caller can place the *tile* where
    it means to rather than placing the shadow around it."""
    big = side * 2
    tile = Image.open(path).convert("RGBA").resize((big, big), Image.LANCZOS)
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, big - 1, big - 1], radius=int(big * CORNER), fill=255
    )
    tile.putalpha(mask)

    margin = side // 6
    shadow = Image.new("L", (big + margin * 4, big + margin * 4), 0)
    shadow.paste(mask, (margin * 2, margin * 2))
    shadow = shadow.filter(ImageFilter.GaussianBlur(side * 0.07)).point(lambda v: int(v * 0.5))
    plate = Image.new("RGBA", shadow.size, (0, 0, 0, 0))
    plate.putalpha(shadow)
    plate.alpha_composite(tile, (margin * 2, margin * 2))
    return plate.resize((side + margin * 2, side + margin * 2), Image.LANCZOS), margin


def arrow(draw: ImageDraw.ImageDraw, x0: int, x1: int, y: int, width: int, fill) -> None:
    """The one glyph an installer needs: a line with a head, on the slots' centre line."""
    draw.line([x0, y, x1 - width * 2, y], fill=fill, width=width)
    head = width * 2
    draw.polygon(
        [(x1, y), (x1 - head, y - head), (x1 - head // 2, y), (x1 - head, y + head)],
        fill=fill,
    )


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()

    w, h = W * SS, H * SS
    ramp = np.linspace(0.0, 1.0, h)[:, None, None]
    band = np.array(GROUND_TOP) + (np.array(GROUND_BOTTOM) - np.array(GROUND_TOP)) * ramp
    img = Image.fromarray(np.broadcast_to(band, (h, w, 3)).astype(np.uint8), "RGB")

    # A mint spotlight behind the logo, the same light the tile itself is lit by: the installer window
    # and the app window are lit from the same place.
    yy, xx = np.mgrid[0:h, 0:w]
    distance = np.hypot(xx - 0.5 * w, yy - 0.18 * h) / (0.42 * w)
    weight = (np.clip(1.0 - distance, 0.0, 1.0) ** 2 * 0.22)[..., None]
    lit = np.asarray(img, dtype=np.float64) * (1 - weight) + np.array(MINT) * weight
    img = Image.fromarray(np.clip(lit, 0, 255).astype(np.uint8), "RGB").convert("RGBA")
    draw = ImageDraw.Draw(img)

    # The face and the name, in the band above the icons. Both are placed by their top edge and sit
    # clear of the icons' boxes horizontally, so the title neither hides behind an icon nor floats off
    # the centre line.
    tile, margin = logo_tile(root / MARK, TILE * SS)
    img.alpha_composite(tile, ((w - tile.width) // 2, TILE_TOP * SS - margin))
    title = font(28 * SS, bold=True)
    name = "FreeCode"
    box = draw.textbbox((0, 0), name, font=title)
    draw.text(((w - (box[2] - box[0])) // 2, (TILE_TOP + TILE + 8) * SS), name, font=title, fill=NAME)

    # The arrow, on the icons' centre line, in the gap between the two slots.
    half = SLOT // 2
    centre_y = APP_SLOT[1] * SS
    x0 = (APP_SLOT[0] + half + 16) * SS
    x1 = (FOLDER_SLOT[0] - half - 16) * SS
    arrow(draw, x0, x1, centre_y, 5 * SS, MINT)

    # The instruction, under the icons — the first band below `y 190+64` that is free across the width.
    line = font(15 * SS, bold=False)
    words = "Drag to Applications"
    box = draw.textbbox((0, 0), words, font=line)
    draw.text(
        ((w - (box[2] - box[0])) // 2, (APP_SLOT[1] + half + 20) * SS),
        words,
        font=line,
        fill=NOTE,
    )

    img.resize((W, H), Image.LANCZOS).save(root / OUT, "PNG", optimize=True)
    img.save(root / PREVIEW)
    print(f"wrote {root / OUT} and {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())