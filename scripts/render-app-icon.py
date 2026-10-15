#!/usr/bin/env python3
"""Render apps/desktop/src-tauri/icons/source.png — the one asset every platform icon is derived
from by `pnpm icon`.

The mark is a bonsai in a pot: the model this app runs is pruned down to ternary weights and kept on
the machine it was downloaded to, which is the same argument a potted bonsai makes. Two colours,
both read out of styles/tokens.css rather than invented here, because an icon in a palette the
window does not use is what makes an app look like two apps.

Run from the repository root:

    python3 scripts/render-app-icon.py

It writes the 1024px source and a contact sheet of the sizes it has to survive at 32px and 16px,
because a mark that only reads at 1024 is not a mark.
"""

from pathlib import Path
import sys

from PIL import Image, ImageDraw

OUT = Path("apps/desktop/src-tauri/icons/source.png")
PREVIEW = Path("/tmp/app-icon-preview.png")
SIZE = 1024
# Every shape is drawn at 4x and downsampled: the mark is ellipses and polygons, so anti-aliasing
# costs nothing but memory here, and a hard-edged 1024px source reads as jagged once the Dock scales
# it down.
SS = 4

# styles/tokens.css, dark mode: the ground the app opens on and the two accents it paints with.
BG_TOP = (17, 41, 33)  # --bg-panel
BG_BOTTOM = (10, 17, 14)  # --bg
MINT = (98, 201, 168)  # --style-mint, measured from reference shot 05
TERRACOTTA = (224, 128, 92)  # --style-parchment


def pad(draw: ImageDraw.ImageDraw, cx: float, cy: float, rx: float, ry: float, fill) -> None:
    """One canopy pad, in the 1024 design grid."""
    g = SIZE * SS
    box = [cx - rx, cy - ry, cx + rx, cy + ry]
    draw.ellipse([int(round(v * g / SIZE)) for v in box], fill=fill)


def stroke(draw: ImageDraw.ImageDraw, x0, y0, x1, y1, width: float, fill) -> None:
    """A trunk segment: a line with round ends, so the bends join without a notch."""
    g = SIZE * SS
    scale = lambda v: int(round(v * g / SIZE))  # noqa: E731
    draw.line([scale(x0), scale(y0), scale(x1), scale(y1)], fill=fill, width=scale(width))
    pad(draw, x0, y0, width / 2, width / 2, fill)
    pad(draw, x1, y1, width / 2, width / 2, fill)


def ground() -> Image.Image:
    """The macOS tile: a rounded square with a top-to-bottom lift off near-black green.

    Painted as a gradient behind its own mask, because a flat tile reads as a sticker and this one
    has to look like the window it opens.
    """
    band = Image.new("RGB", (1, SIZE))
    for y in range(SIZE):
        t = y / (SIZE - 1)
        band.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOTTOM)))
    tile = band.resize((SIZE, SIZE), Image.BILINEAR).convert("RGBA")

    mask = Image.new("L", (SIZE, SIZE), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, SIZE, SIZE], radius=int(SIZE * 0.225), fill=255)
    tile.putalpha(mask)
    return tile


def tree() -> Image.Image:
    img = Image.new("RGBA", (SIZE * SS, SIZE * SS), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    scale = lambda v: int(round(v * SS))  # noqa: E731

    # Two pads, not one: the large crown and a smaller side pad reached by a cut branch. A single
    # circle is a balloon, and the whole point of the silhouette is that someone pruned it into this
    # shape — which is the argument the app makes about the weights it loads.
    pad(draw, 476, 322, 182, 122, MINT)
    pad(draw, 672, 436, 104, 70, MINT)

    # Up out of the pot, one bend, and the stub that carries the side pad. Same colour as the canopy
    # because it is one plant, not two objects.
    stroke(draw, 506, 660, 462, 566, 26, MINT)
    stroke(draw, 462, 566, 484, 442, 20, MINT)
    stroke(draw, 478, 502, 612, 452, 15, MINT)

    # The pot: a rim over a tapered body, and the only warm thing on the tile so the eye lands under
    # the tree.
    draw.polygon(
        [
            (scale(382), scale(672)),
            (scale(642), scale(672)),
            (scale(600), scale(784)),
            (scale(424), scale(784)),
        ],
        fill=TERRACOTTA,
    )
    draw.rounded_rectangle(
        [scale(362), scale(650), scale(662), scale(686)], radius=scale(10), fill=TERRACOTTA
    )
    return img.resize((SIZE, SIZE), Image.LANCZOS)


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    tile = ground()
    tile.alpha_composite(tree())
    tile.save(root / OUT)

    # The Dock at 16px and a Retina menu bar at 32px are where an icon is actually judged, and both
    # are down here rather than up at 1024.
    preview = Image.new("RGB", (SIZE + 320, SIZE), (246, 246, 246))
    preview.paste(tile, (0, 0), tile)
    x = SIZE + 40
    for side in (128, 64, 32, 16):
        small = tile.resize((side, side), Image.LANCZOS)
        preview.paste(small, (x, SIZE - side), small)
        preview.paste(small, (x, SIZE - 220 - side), small)
        x += side + 24
    preview.save(root / PREVIEW)
    print(f"wrote {root / OUT} and {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
