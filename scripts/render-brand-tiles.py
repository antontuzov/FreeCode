#!/usr/bin/env python3
"""Cut the brand logo into the two Dock tiles the app can wear: its mark on a white ground and on a
black one.

`apps/desktop/src-tauri/brand/freecode-logo.png` is the artwork as supplied — the mark already on its
own dark field, edge to edge. That is one tile, and the Dock it goes on is painted either way round,
so the same mark needs both grounds. A user asked for exactly that: "add icon for dock but make bg
white and black".

The mark cannot simply be recoloured, because it is *emissive* art: it was painted as light added to
a near-black field, and its glow fades continuously into that field. Pasting the pixels onto white
would paste the field with them, and a white tile with a dark haze where the glow was is not the
brand. So the mark is matted out first:

* the field's colour is measured (the median of the pixels that are not the mark), never typed, so a
  redrawn logo re-derives its own ground rather than being keyed against a stale number;
* alpha is a smoothstep on how far a pixel's luminance sits above that field, which is the same
  quantity the artwork was built from;
* the colour is then un-premultiplied against the field, which turns "emission over dark" back into
  "the colour itself, with the weight it would carry on its own".

The result is composited on both grounds at the logo's own proportions, so the mark keeps the size it
has on the supplied tile rather than being re-cropped.

Run from the repository root:

    python3 scripts/render-brand-tiles.py

It writes tiles 13 (white) and 14 (black) in the two shapes they are consumed in — the webp the
window globs and the PNG `brand.rs` hands to `NSImage` — and a contact sheet of both on a white and a
black desktop.
"""

from pathlib import Path
import sys

import numpy as np
from PIL import Image, ImageDraw

SRC = Path("apps/desktop/src-tauri/brand/freecode-logo.png")
WEBP_DIR = Path("apps/desktop/src/assets/mascots")
PNG_DIR = Path("apps/desktop/src-tauri/mascots")
PREVIEW = Path("/tmp/brand-tiles-preview.png")
# The same geometry the twelve ducks are cut at, so the set stays one set.
TILE = 256
SS = 4
DOCK_INSET = 0.86
CORNER = 0.225
# The two tiles' numbers in the set the settings document and `?mascot=` name.
TILES = {"white": 13, "black": 14}

LUMA = np.array([0.299, 0.587, 0.114])
# A pixel belongs to the field rather than the mark below this luminance. The supplied artwork's
# field runs 9..33 end to end and its mark starts at ~120, so the gap is wide and the number is not
# delicate.
FIELD_MAX = 40


def smoothstep(x: np.ndarray, low: float, high: float) -> np.ndarray:
    t = np.clip((x - low) / (high - low), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def matte(logo: Image.Image) -> tuple[Image.Image, tuple[int, int, int]]:
    """The mark as an RGBA layer that can be laid on any ground, and the field it came off."""
    a = np.asarray(logo.convert("RGB"), dtype=np.float64)
    lum = a @ LUMA
    field = np.median(a[lum < FIELD_MAX], axis=0)
    excess = lum - float(field @ LUMA)
    alpha = smoothstep(excess, 6.0, 80.0)
    weight = np.maximum(alpha[..., None], 1e-3)
    colour = (a - field * (1.0 - alpha[..., None])) / weight
    rgba = np.dstack([np.clip(colour, 0, 255), alpha * 255.0]).astype(np.uint8)
    return Image.fromarray(rgba, "RGBA"), tuple(int(round(v)) for v in field)


def ground(side: int, top: tuple[int, int, int], bottom: tuple[int, int, int], edge) -> Image.Image:
    """A tile's background: a top-to-bottom lift, and a hairline that keeps a light ground from
    losing its outline on a light desktop. `None` edge means the ground is dark enough to need none.
    """
    big = side * SS
    ramp = np.linspace(0.0, 1.0, big)[:, None, None]
    band = np.array(top) + (np.array(bottom) - np.array(top)) * ramp
    tile = np.broadcast_to(band, (big, big, 3)).astype(np.uint8)
    img = Image.fromarray(tile, "RGB").convert("RGBA")

    if edge is not None:
        # A band drawn as the difference of two filled rectangles, because this Pillow ignores the
        # `width` argument on a rounded rectangle and fills the whole interior instead.
        width = int(big * 0.008)
        inset = int(width * 1.5)
        radius = int(big * CORNER)
        ring = Image.new("L", (big, big), 0)
        draw = ImageDraw.Draw(ring)
        draw.rounded_rectangle([inset, inset, big - 1 - inset, big - 1 - inset], radius=radius, fill=255)
        draw.rounded_rectangle(
            [inset + width, inset + width, big - 1 - inset - width, big - 1 - inset - width],
            radius=radius - width,
            fill=0,
        )
        img = Image.composite(Image.new("RGBA", (big, big), (*edge, 255)), img, ring)
    return img


def rounded(img: Image.Image, side: int, inset: float) -> Image.Image:
    """The tile as a rounded square on transparent margin, which is what the Dock draws unmasked."""
    big = side * SS
    disc = int(big * inset)
    margin = (big - disc) // 2
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [margin, margin, big - margin, big - margin], radius=int(disc * CORNER), fill=255
    )
    small = img.resize((side, side), Image.LANCZOS).convert("RGBA")
    small.putalpha(mask.resize((side, side), Image.LANCZOS))
    return small


def square_tile(art: Image.Image, side: int, inset: float | None) -> Image.Image:
    """The supplied artwork at the tile's size, masked to a rounded square or kept full-bleed."""
    big = side * SS
    scaled = art.resize((big, big), Image.LANCZOS)
    if inset is None:
        return scaled.resize((side, side), Image.LANCZOS)
    return rounded(scaled, side, inset)


def build(ground_spec: tuple, mark: Image.Image, side: int, inset: float | None) -> Image.Image:
    """One tile: the mark laid over its ground at the logo's own proportions. `inset=None` keeps the
    square edge to edge, which is the shape the window's own circle is drawn over."""
    big = side * SS
    img = ground(side, *ground_spec)
    img.alpha_composite(mark.resize((big, big), Image.LANCZOS))
    if inset is None:
        return img.resize((side, side), Image.LANCZOS)
    return rounded(img, side, inset)


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    logo = root / SRC
    if not logo.is_file():
        print(f"FAIL no brand artwork at {logo}")
        return 1
    mark, field = matte(Image.open(logo))
    print(f"the supplied field measures {field} and the mark comes off it as one RGBA layer")

    # The two tiles are built differently on purpose. The black one *is* the artwork: it already
    # carries its ground, its glow and its own painted edge, and re-deriving it would lose the glow
    # that makes it read as the supplied logo. The white one can only be built from the matte,
    # because a glow is light added to dark and has nothing to add to white.
    art = Image.open(logo).convert("RGBA")
    white_ground = ((255, 255, 255), (243, 246, 243), (211, 217, 211))
    built = {
        "white": lambda side, inset: build(white_ground, mark, side, inset),
        "black": lambda side, inset: square_tile(art, side, inset),
    }
    for name, number in TILES.items():
        make = built[name]
        webp = root / WEBP_DIR / f"mascot-{number:02d}.webp"
        png = root / PNG_DIR / f"mascot-{number:02d}.png"
        webp.parent.mkdir(parents=True, exist_ok=True)
        png.parent.mkdir(parents=True, exist_ok=True)
        # The window crops the tile to a circle itself, so its copy is the full square; the Dock
        # clips nothing, so the host's copy carries its own rounded margin.
        make(TILE, None).convert("RGB").save(webp, "WEBP", quality=82, method=6)
        make(TILE, DOCK_INSET).save(png, "PNG", optimize=True)
        print(
            f"{name} → {webp.relative_to(root)} {webp.stat().st_size / 1024:.1f} KiB"
            f" | {png.relative_to(root)} {png.stat().st_size / 1024:.1f} KiB"
        )

    # Both tiles on both desktops, because the case that breaks is always the one the sheet is not
    # drawn on: a white ground has to survive a white Dock.
    sheet = Image.new("RGB", (2 * (TILE + 40), 2 * (TILE + 40)), (246, 246, 246))
    for column, (name, number) in enumerate(TILES.items()):
        for row, desktop in enumerate(("white", "black")):
            band = (255, 255, 255) if desktop == "white" else (12, 13, 14)
            x0, y0 = column * (TILE + 40), row * (TILE + 40)
            ImageDraw.Draw(sheet).rectangle([x0, y0, x0 + TILE, y0 + TILE], fill=band)
            face = built[name](TILE, DOCK_INSET)
            sheet.paste(face, (x0, y0), face)
            ImageDraw.Draw(sheet).text((x0 + 6, y0 + 4), f"{number} {name}", fill=(140, 140, 140))
    sheet.save(root / PREVIEW)
    print(f"wrote {root / PREVIEW}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
