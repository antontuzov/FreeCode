#!/usr/bin/env python3
"""Render apps/desktop/src-tauri/icons/source.png — the one asset every platform icon is derived
from by `pnpm icon`.

The mark is the duck: the calm teal head on the forest ground, composed by `scripts/theme_logos.py`
— the same picture the Dock tiles 15–17, the Windows tiles and the installer art are cut from. The
app has worn a bonsai here before, and it was wrong twice over: the tree said nothing the app does,
and the tree was not the face the app already showed the moment it launched. One face, everywhere.

The ground and the accent are read out of `theme_logos.THEMES` rather than invented here, because an
icon in a palette the window does not use is what makes an app look like two apps.

The composition is cut to a rounded square with transparent corners, which is the shape macOS draws
an app icon in, and it is a *mask*, not a margin: the art runs to the edge of the rounded tile, the
way a macOS icon is supposed to.

Run from the repository root:

    python3 scripts/render-app-icon.py

It writes the 1024px source and a contact sheet of the sizes it has to survive at 32px and 16px,
because a mark that only reads at 1024 is not a mark.
"""

from pathlib import Path
import sys

from PIL import Image, ImageDraw

import theme_logos

OUT = Path("apps/desktop/src-tauri/icons/source.png")
PREVIEW = Path("/tmp/app-icon-preview.png")
MASCOTS = Path("apps/desktop/src/assets/mascots")
SIZE = 1024
# The corner the tile is cut at, as a fraction of its side — the shape macOS itself uses, and the
# shape the Dock tiles and the installer tile are cut at.
CORNER = 0.225
# The face the app opens on: the forest theme is the window's own ground, so the icon and the
# window are lit from the same place. The Dock repaints the tile with a mascot at launch; this is
# what Finder, the installer and the volume show, and those cannot change after the fact.
THEME = "forest"


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    spec = theme_logos.THEMES[THEME]
    head = theme_logos.matte(Image.open(root / MASCOTS / f"mascot-{spec['duck']:02d}.webp"))

    art = theme_logos.compose(theme_logos.ground(SIZE, spec), head)
    mask = Image.new("L", (SIZE, SIZE), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, SIZE - 1, SIZE - 1], radius=int(SIZE * CORNER), fill=255
    )
    art.putalpha(mask)
    art.save(root / OUT)

    # The Dock at 16px and a Retina menu bar at 32px are where an icon is actually judged, and both
    # are down here rather than up at 1024. The small sizes are shown twice, on a light and a dark
    # desktop, because a duck on a dark ground is judged against the desktop and not against itself.
    preview = Image.new("RGB", (SIZE + 400, SIZE), (246, 246, 246))
    preview.paste(art, (0, 0), art)
    x = SIZE + 40
    for side in (128, 64, 32, 16):
        small = art.resize((side, side), Image.LANCZOS)
        preview.paste(small, (x, SIZE - side), small)
        preview.paste(small, (x, SIZE - 220 - side), small)
        x += side + 24
    preview.save(root / PREVIEW)
    print(f"wrote {root / OUT} and {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
