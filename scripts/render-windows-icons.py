#!/usr/bin/env python3
"""Render the Windows face of the app — the Square tile set, the Store logo and icon.ico.

`pnpm icon` cuts the whole platform set from `source.png`, and it cuts the Windows tiles with the
macOS shape: a rounded square on transparent margin. Windows masks nothing, and a tile that carries
its own rounding shows the desktop through the corners. This script bakes the Windows shape instead:

* the tile is a full-bleed square with no rounded corners;
* the face is the forest theme — the same composition the macOS icon, the Dock tiles and the
  installer art are cut from, so a Windows user, a macOS user and the installer are looking at the
  same brand. The bundle wears one face: a bundled icon cannot change after install, so the duck
  the Dock rolls at launch is the only part of the brand that moves;
* the ground, the spotlight and the head are `scripts/theme_logos.py`'s, drawn here at the size the
  bundle needs.

Run from the repository root:

    python3 scripts/render-windows-icons.py

It writes the nine Square tiles, `StoreLogo.png` and `icon.ico` into
`apps/desktop/src-tauri/icons/`, and a contact sheet of what the small sizes actually look like.
"""

from pathlib import Path
import sys

from PIL import Image

import theme_logos

MASCOTS = Path("apps/desktop/src/assets/mascots")
ICONS = Path("apps/desktop/src-tauri/icons")
PREVIEW = Path("/tmp/windows-icons-preview.png")
# The theme the bundle wears: the forest ground the window opens on, so the tile and the window are
# the same picture. See the docstring for why it is fixed rather than rolled.
THEME = "forest"
# The size the tile is drawn at once; every written size is a downsample of this, because drawing
# the feathered edges at 16px directly would alias them into noise.
RENDER = 1024
# The sizes a Tauri Windows bundle names. Everything except StoreLogo is a Square tile.
SQUARES = [30, 44, 71, 89, 107, 142, 150, 284, 310]
STORE = 50
ICO = [16, 24, 32, 48, 64, 128, 256]


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    spec = theme_logos.THEMES[THEME]
    head = theme_logos.matte(Image.open(root / MASCOTS / f"mascot-{spec['duck']:02d}.webp"))
    print(f"baked the {THEME} logo (duck {spec['duck']})")

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
