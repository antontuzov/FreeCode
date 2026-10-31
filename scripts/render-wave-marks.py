#!/usr/bin/env python3
"""Cut the supplied wave artwork into the two round tiles the app wears beside the ducks: the same
mark on a white ground and on a black one.

The supplied file is one image holding both grounds side by side — white on the left, black on the
right — so the halves are separated rather than keyed out: each half keeps the ground it was painted
on, and the mark is fitted into a circle instead of being cropped to the half's edges. That is what
"like the duck logos" means here: a round tile whose art sits inside the ring rather than running off
it.

Written twice, for the same two readers `scripts/render-mascot-logos.py` serves:

* `…/src/assets/mascots/mascot-NN.webp`, full-bleed square — CSS draws the window's circle.
* `…/src-tauri/mascots/mascot-NN.png`, a round PNG on transparent margin for the Dock, which clips
  nothing.

The artwork is not build output, so it is not committed. Run from the repository root:

    python3 scripts/render-wave-marks.py ~/Desktop/wave.png
"""

from pathlib import Path
import sys

from PIL import Image, ImageChops, ImageDraw

OUT_DIR = Path("apps/desktop/src/assets/mascots")
HOST_OUT_DIR = Path("apps/desktop/src-tauri/mascots")
TILE = 256
SUPERSAMPLE = 4
DOCK_INSET = 0.86
WEBP_QUALITY = 82
# The mark takes this fraction of the canvas. A disc's inscribed square is 0.707 of its diameter, so
# 0.70 keeps the art inside the ring with the same air the duck heads have around them.
MARK_INSET = 0.70
# How far a pixel may sit from the ground and still be ground. The fields are flat and the art is
# vivid, so this is a wide gap and does not need tuning.
GROUND_TOLERANCE = 28


def ground_of(cell: Image.Image) -> tuple[tuple[int, int, int], Image.Image]:
    """The flat colour the half was painted on, measured as the half's most common colour.

    The borders are not a safe place to measure: the wave runs off both side edges, so a sample along
    them is part art. The ground is what most of the half is, by a wide margin.
    """
    w, h = cell.size
    px = cell.load()
    counts: dict[tuple[int, int, int], int] = {}
    for y in range(0, h, 7):
        for x in range(0, w, 7):
            colour = px[x, y]
            counts[colour] = counts.get(colour, 0) + 1
    ground = max(counts, key=counts.get)
    flat = Image.new("RGB", cell.size, ground)
    return ground, flat


def mark_box(cell: Image.Image, ground: tuple[int, int, int]) -> tuple[int, int, int, int]:
    """The art's bounding box, as everything that differs from the ground by more than the tolerance."""
    flat = Image.new("RGB", cell.size, ground)
    diff = ImageChops.difference(cell, flat).convert("L").point(
        lambda v: 255 if v > GROUND_TOLERANCE else 0
    )
    box = diff.getbbox()
    if box is None:
        raise SystemExit("FAIL one half is all ground: nothing was supplied to cut")
    return box


def round_tile(square: Image.Image) -> Image.Image:
    """The square as the Dock draws it: a disc inset on transparent margin, cut at 4x and downscaled."""
    big = TILE * SUPERSAMPLE
    disc = int(big * DOCK_INSET)
    margin = (big - disc) // 2
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).ellipse((margin, margin, big - margin, big - margin), fill=255)
    art = square.convert("RGBA")
    art.putalpha(mask.resize((TILE, TILE), Image.LANCZOS))
    return art


def tile(cell: Image.Image, number: int, name: str) -> None:
    ground, _ = ground_of(cell)
    x0, y0, x1, y1 = mark_box(cell, ground)
    art = cell.crop((x0, y0, x1, y1))

    canvas = TILE * SUPERSAMPLE
    target = int(canvas * MARK_INSET)
    scale = min(target / art.width, target / art.height)
    art = art.resize(
        (max(1, round(art.width * scale)), max(1, round(art.height * scale))), Image.LANCZOS
    )
    square = Image.new("RGB", (canvas, canvas), ground)
    square.paste(art, ((canvas - art.width) // 2, (canvas - art.height) // 2))
    square = square.resize((TILE, TILE), Image.LANCZOS)

    webp = OUT_DIR / f"mascot-{number:02d}.webp"
    png = HOST_OUT_DIR / f"mascot-{number:02d}.png"
    square.save(webp, "WEBP", quality=WEBP_QUALITY, method=6)
    round_tile(square).save(png, "PNG", optimize=True)
    print(
        f"{name}: ground {ground} -> {webp} {webp.stat().st_size / 1024:.1f} KiB"
        f" | {png} {png.stat().st_size / 1024:.1f} KiB"
    )


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        print(f"usage: {sys.argv[0]} <wave.png>")
        return 2
    src = Path(sys.argv[1]).expanduser()
    if not src.is_file():
        print(f"FAIL no artwork at {src}")
        return 1

    im = Image.open(src).convert("RGB")
    if im.width % 2:
        print(f"FAIL expected two halves, width is {im.width}")
        return 1
    mid = im.width // 2
    left = im.crop((0, 0, mid, im.height))
    right = im.crop((mid, 0, im.width, im.height))

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    HOST_OUT_DIR.mkdir(parents=True, exist_ok=True)
    tile(left, 13, "white")
    tile(right, 14, "black")
    print(f"OK — tiles 13 and 14 in {OUT_DIR} and {HOST_OUT_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
