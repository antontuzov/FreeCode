#!/usr/bin/env python3
"""Cut the three themed brand tiles — the duck *as a logo*, one face and one ground per app style.

The set the Dock rolls from already carries the duck heads as photographed (1–12) and the brand mark
on white and black (13–14). Those are photographs on a black field; they are not logos. A logo here is
a composition: a ground the style family paints, a spotlight in that family's accent, and one head
rising out of the tile's bottom edge so the photograph's crop is the frame's edge rather than a neck
cut off in mid-air. The composition itself is `scripts/theme_logos.py`, shared with the Windows tiles
and the installer art; this script cuts the two Dock shapes from it.

* 15 forest — the dark green the app opens on, mint spotlight, the calm teal face. The window's mood.
* 16 bee — the warm cream ground the Bee family paints, honey spotlight, the laughing face. The
  flagship: this is the duck the brand already uses when it needs a face.
* 17 parchment — terracotta, the Parchment family's own colour, the wry purple face.

Run from the repository root:

    python3 scripts/render-duck-theme-logos.py

It writes tiles 15–17 in the two shapes they are consumed in — the webp the window globs and the PNG
`brand.rs` hands to `NSImage` — and a contact sheet of the three on a white and a black desktop.
"""

from pathlib import Path
import sys

from PIL import Image, ImageDraw

import theme_logos

WEBP_DIR = Path("apps/desktop/src/assets/mascots")
PNG_DIR = Path("apps/desktop/src-tauri/mascots")
PREVIEW = Path("/tmp/duck-theme-preview.png")
TILE = 256
SS = 4
DOCK_INSET = 0.86
CORNER = 0.225
# The three tiles' numbers in the set the settings document and `?mascot=` name.
TILES = {"forest": 15, "bee": 16, "parchment": 17}


def tile(spec: dict, head: Image.Image, side: int, dock: bool) -> Image.Image:
    art = theme_logos.compose(theme_logos.ground(side * SS, spec), head)
    if not dock:
        return art.resize((side, side), Image.LANCZOS)
    face = theme_logos.rounded(art, side, DOCK_INSET, CORNER)
    margin = int(side * (1 - DOCK_INSET) / 2)
    radius = int(side * DOCK_INSET * CORNER)
    return theme_logos.outlined(face, (margin, radius), spec["stroke"])


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    faces = {
        name: theme_logos.matte(
            Image.open(root / WEBP_DIR / f"mascot-{spec['duck']:02d}.webp")
        )
        for name, spec in theme_logos.THEMES.items()
    }
    for name, number in TILES.items():
        spec = theme_logos.THEMES[name]
        webp = root / WEBP_DIR / f"mascot-{number:02d}.webp"
        png = root / PNG_DIR / f"mascot-{number:02d}.png"
        tile(spec, faces[name], TILE, False).convert("RGB").save(webp, "WEBP", quality=82, method=6)
        tile(spec, faces[name], TILE, True).save(png, "PNG", optimize=True)
        print(
            f"{name} duck {spec['duck']} → {webp.relative_to(root)}"
            f" {webp.stat().st_size / 1024:.1f} KiB"
            f" | {png.relative_to(root)} {png.stat().st_size / 1024:.1f} KiB"
        )

    # The three tiles on both desktops: a light ground has to survive a light Dock.
    sheet = Image.new("RGB", (3 * (TILE + 40), 2 * (TILE + 40)), (246, 246, 246))
    for column, name in enumerate(TILES):
        for row, desktop in enumerate(("white", "black")):
            band = (255, 255, 255) if desktop == "white" else (12, 13, 14)
            x0, y0 = column * (TILE + 40), row * (TILE + 40)
            ImageDraw.Draw(sheet).rectangle([x0, y0, x0 + TILE, y0 + TILE], fill=band)
            face = tile(theme_logos.THEMES[name], faces[name], TILE, True)
            sheet.paste(face, (x0, y0), face)
            ImageDraw.Draw(sheet).text((x0 + 6, y0 + 4), f"{TILES[name]} {name}", fill=(140, 140, 140))
    sheet.save(root / PREVIEW)
    print(f"wrote {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())