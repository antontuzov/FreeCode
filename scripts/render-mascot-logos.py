#!/usr/bin/env python3
"""Cut the mascot sheet into the twelve logos the app wears, one per launch, at random.

The sheet is a 4x3 grid of painterly duck heads on black, one expression per tile. The app ships the
twelve crops, not the sheet: the grid lines are the only thing the sheet has that a single logo does
not, and a 4.4 MB file in the bundle to draw a 72px avatar is not a trade worth making.

The gutters are found rather than assumed, because the sheet is not divided evenly — the columns are
590, 592, 582 and 588 px wide, and a grid typed in by hand would put a black seam down one side of
every third logo. A gutter here is a line that is black end to end; the cut goes through its middle.

Each cell is then centre-cropped to a square, because the tile is drawn into a circle in the window
and a square is the largest circle-free shape that keeps every head the same size.

Each crop is written twice, because two different things read it:

* `…/src/assets/mascots/mascot-NN.webp`, loaded by the webview, where the browser's own decoder makes
  the file's cost the app's bundle size. Full-bleed square, because CSS draws the window's circle.
* `…/src-tauri/mascots/mascot-NN.png`, `include_bytes!` by `brand.rs` and handed to `NSImage` as the
  Dock tile. This one carries its own circle: the Dock clips nothing, so a square tile reads as a
  black-cornered stamp among the round icons. Alpha PNG, because transparency has no JPEG encoding.

The source sheet is the artwork, not build output, so it is not committed. Re-run with the path:

    python3 scripts/render-mascot-logos.py ~/Desktop/Joker/sheet.png
"""

from pathlib import Path
import sys

from PIL import Image, ImageDraw

OUT_DIR = Path("apps/desktop/src/assets/mascots")
# The host's copy of the same twelve, for the Dock tile. See the docstring for why it is a round PNG.
HOST_OUT_DIR = Path("apps/desktop/src-tauri/mascots")
# Displayed at 72px on a 2x screen. 256 keeps it sharp on a 3x panel and leaves the whole set at a
# few tens of kilobytes, which is what an avatar should cost.
TILE = 256
WEBP_QUALITY = 82
# The Dock canvas is bigger than the icon on it, and every round tile in a real Dock sits inset with
# transparent margin around it. Filling the square edge to edge makes ours look oversized next to
# them, so the disc takes this fraction of the canvas.
DOCK_INSET = 0.86
# The disc is cut at 4x and downscaled: an ellipse drawn at 256px lands on whole pixels and the
# Dock's own smoothing turns that into a scalloped edge.
SUPERSAMPLE = 4
# A line is a gutter when the brightest pixel along it is still black. 8/255 is below any feather
# the art itself draws, and above the JPEG-ish noise a screenshot of it would carry.
BLACK_MAX = 8
STRIDE = 7  # sample every 7th pixel across a line: this finds seams, it does not need every pixel
# Dark columns are not one line each: the feathers fade to black on both sides of a seam, so a seam
# reads as a handful of separate bands 20-40 px apart. They are the same gutter.
MERGE_GAP = 40


def gutters(length: int, line_at, other: int) -> list[int]:
    """Midpoints of the black separator bands running perpendicular to the cut."""
    dark = [i for i in range(length) if max(line_at(i, j) for j in range(0, other, STRIDE)) < BLACK_MAX]
    bands: list[list[int]] = []
    for i in dark:
        if bands and i - bands[-1][-1] <= MERGE_GAP:
            bands[-1].append(i)
        else:
            bands.append([i])
    # Edge bands are the sheet's own margin, not a seam between two logos.
    inner = [b for b in bands if b[0] > 0 and b[-1] < length - 1]
    return [(b[0] + b[-1]) // 2 for b in inner]


def dock_tile(cell: Image.Image) -> Image.Image:
    """The square crop as a round icon on transparent margin, which is what the Dock draws unmasked."""
    big = TILE * SUPERSAMPLE
    disc = int(big * DOCK_INSET)
    margin = (big - disc) // 2
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).ellipse((margin, margin, big - margin, big - margin), fill=255)
    art = cell.convert("RGBA")
    # Only the alpha is cut; the duck's own colours run right out to the square's edge under the
    # transparent ring, so there is no half-lit seam to composite against.
    art.putalpha(mask.resize((TILE, TILE), Image.LANCZOS))
    return art


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        print(f"usage: {sys.argv[0]} <sheet.png>")
        return 2
    src = Path(sys.argv[1]).expanduser()
    if not src.is_file():
        print(f"FAIL no sheet at {src}")
        return 1

    im = Image.open(src).convert("RGB")
    w, h = im.size
    px = im.load()
    v = gutters(w, lambda i, j: sum(px[i, j]) / 3, h)  # vertical seams -> column cuts
    hz = gutters(h, lambda i, j: sum(px[j, i]) / 3, w)  # horizontal seams -> row cuts
    if len(v) != 3 or len(hz) != 2:
        print(f"FAIL expected a 4x3 grid, found {len(v) + 1}x{len(hz) + 1} of gutters at {v} / {hz}")
        return 1

    cols = list(zip([0] + v, v + [w]))
    rows = list(zip([0] + hz, hz + [h]))
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    HOST_OUT_DIR.mkdir(parents=True, exist_ok=True)

    n = 0
    for y0, y1 in rows:
        for x0, x1 in cols:
            cell = im.crop((x0, y0, x1, y1))
            side = min(cell.size)
            left = (cell.width - side) // 2
            top = (cell.height - side) // 2
            cell = cell.crop((left, top, left + side, top + side)).resize((TILE, TILE), Image.LANCZOS)
            n += 1
            out = OUT_DIR / f"mascot-{n:02d}.webp"
            cell.save(out, "WEBP", quality=WEBP_QUALITY, method=6)
            host = HOST_OUT_DIR / f"mascot-{n:02d}.png"
            dock_tile(cell).save(host, "PNG", optimize=True)
            print(
                f"{out} {cell.width}x{cell.height} {out.stat().st_size / 1024:.1f} KiB"
                f" | {host} {host.stat().st_size / 1024:.1f} KiB"
            )

    if n != 12:
        print(f"FAIL wrote {n} tiles, not 12")
        return 1
    print(f"OK — 12 tiles in {OUT_DIR} and {HOST_OUT_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
