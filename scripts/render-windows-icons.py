#!/usr/bin/env python3
"""Render the Windows face of the app — the Square tile set, the Store logo and icon.ico.

Every icon in the bundle was derived from `source.png`, the drawn bonsai: the macOS Dock repaints
itself with a duck at launch (`brand.rs`), but Windows has no such repaint, so a Windows user's only
face of the app is the one baked at build time — a tree. This script bakes the duck instead:

* one of the three themed logos is rolled at random — the same compositions the Dock tiles and the
  installer art are cut from, so a Windows user, a macOS user and the installer are looking at the
  same brand. "Random logo" is a property of the build, said up front, because a bundled icon cannot
  change after install;
* the tile is the Windows shape: a full-bleed square with no rounded corners, where the macOS tiles
  round themselves and sit on transparent margin. Windows masks nothing, and a tile that carries its
  own rounding shows the desktop through the corners;
* the ground, the spotlight and the head are `scripts/theme_logos.py`'s, drawn here at the size the
  bundle needs.

Run from the repository root:

    python3 scripts/render-windows-icons.py

It writes the nine Square tiles, `StoreLogo.png` and `icon.ico` into
`apps/desktop/src-tauri/icons/`, and a contact sheet of what the small sizes actually look like.
"""

from pathlib import Path
import random
import sys

from PIL import Image

import theme_logos

MASCOTS = Path("apps/desktop/src/assets/mascots")
ICONS = Path("apps/desktop/src-tauri/icons")
PREVIEW = Path("/tmp/windows-icons-preview.png")
# The size the tile is drawn at once; every written size is a downsample of this, because drawing
# the feathered edges at 16px directly would alias them into noise.
RENDER = 1024
# The sizes a Tauri Windows bundle names. Everything except StoreLogo is a Square tile.
SQUARES = [30, 44, 71, 89, 107, 142, 150, 284, 310]
STORE = 50
ICO = [16, 24, 32, 48, 64, 128, 256]


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    name = random.SystemRandom().choice(list(theme_logos.THEMES))
    spec = theme_logos.THEMES[name]
    head = theme_logos.matte(Image.open(root / MASCOTS / f"mascot-{spec['duck']:02d}.webp"))
    print(f"rolled the {name} logo (duck {spec['duck']})")

    big = theme_logos.compose(theme_logos.ground(RENDER, spec), head)
    out = root / ICONS
    for side in SQUARES:
        big.resize((side, side), Image.LANCZOS).save(out / f"Square{side}x{side}Logo.png", "PNG")
    big.resize((STORE, STORE), Image.LANCZOS).save(out / "StoreLogo.png", "PNG")
    big.save(out / "icon.png", "PNG")
    big.save(
        out / "icon.ico",
        sizes=[(side, side) for side in ICO],
        append_images=[big.resize((side, side), Image.LANCZOS) for side in ICO[1:]],
    )

    # The sizes Windows actually judges the tile at: the taskbar is 24-32px and the Start menu 48.
    sheet = Image.new("RGB", (RENDER + 7 * 96, RENDER), (246, 246, 246))
    sheet.paste(big, (0, 0))
    x = RENDER + 24
    for side in (48, 32, 24, 16):
        small = big.resize((side, side), Image.LANCZOS)
        sheet.paste(small, (x, RENDER - side), small)
        x += 96
    sheet.save(root / PREVIEW)
    print(f"wrote the Square set, StoreLogo, icon.png and icon.ico into {out}")
    print(f"wrote {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
