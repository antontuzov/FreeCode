#!/usr/bin/env python3
"""The three themed logos, drawn once — the shared half of every script that paints the brand.

A themed logo is a composition rather than a photograph: a ground the style family paints, a spotlight
in that family's accent, a matted duck head, and a soft shadow under it so the head reads as an object
on the ground instead of a sticker pasted onto one. `render-duck-theme-logos.py` cuts the Dock tiles
from it, `render-windows-icons.py` bakes the Windows tiles, and the installer art borrows the same
ground — so the composition lives here, and each of those scripts only says what it writes.

Nothing here reads or writes a file: `THEMES` names the three, and the drawing functions take a spec.
"""

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

# styles/tokens.css: each style family's ground, with the accent it paints with. `duck` is the face
# the family wears — the calm teal head on the forest ground the window opens in, the laughing head on
# the Bee family's warm cream, the wry purple head on Parchment's terracotta. `stroke` is the hairline
# a light ground needs to hold its outline on a white desktop; `None` where the ground is dark enough.
THEMES = {
    "forest": {
        "duck": 1,
        "top": (22, 55, 44),
        "bottom": (7, 18, 14),
        "glow": (86, 200, 166),  # --style-mint, lifted
        "glow_alpha": 0.36,
        "stroke": None,
    },
    "bee": {
        "duck": 2,
        "top": (255, 249, 226),
        "bottom": (229, 205, 143),
        "glow": (255, 214, 110),
        "glow_alpha": 0.45,
        "stroke": ((176, 156, 78), 105),  # --style-bee, taken down so it holds on white
    },
    "parchment": {
        "duck": 5,
        "top": (190, 96, 64),
        "bottom": (130, 55, 34),
        "glow": (248, 190, 155),
        "glow_alpha": 0.40,
        "stroke": None,  # --style-parchment is the ground itself
    },
}
# How much of the tile the head takes, and where the photograph's bottom crop lands. Above 1 so the
# neck runs off the tile edge instead of ending as a horizontal cut inside it.
HEAD_WIDTH = 1.04
HEAD_BOTTOM = 1.07
# How dark the shadow the head casts on the ground is.
SHADOW = 0.26

LUMA = np.array([0.299, 0.587, 0.114])


def smoothstep(x: np.ndarray, low: float, high: float) -> np.ndarray:
    t = np.clip((x - low) / (high - low), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def matte(duck: Image.Image) -> Image.Image:
    """The head as an RGBA layer that can be laid on any ground. The sheet's field is black, so alpha
    is how far a pixel's luminance sits above it — the same quantity the feathering painted."""
    a = np.asarray(duck.convert("RGB"), dtype=np.float64)
    lum = a @ LUMA
    alpha = smoothstep(lum, 8.0, 70.0)
    weight = np.maximum(alpha[..., None], 1e-3)
    colour = a / weight
    rgba = np.dstack([np.clip(colour, 0, 255), alpha * 255.0]).astype(np.uint8)
    return Image.fromarray(rgba, "RGBA")


def ground(side: int, spec: dict) -> Image.Image:
    """A tile's background: a top-to-bottom ramp lit by a round spotlight behind the head."""
    ramp = np.linspace(0.0, 1.0, side)[:, None, None]
    band = np.array(spec["top"]) + (np.array(spec["bottom"]) - np.array(spec["top"])) * ramp
    field = np.broadcast_to(band, (side, side, 3)).astype(np.float64)

    yy, xx = np.mgrid[0:side, 0:side]
    distance = np.hypot(xx - 0.5 * side, yy - 0.40 * side) / (0.60 * side)
    weight = np.clip(1.0 - distance, 0.0, 1.0) ** 2 * spec["glow_alpha"]
    field = field * (1.0 - weight[..., None]) + np.array(spec["glow"]) * weight[..., None]

    return Image.fromarray(np.clip(field, 0, 255).astype(np.uint8), "RGB").convert("RGBA")


def compose(base: Image.Image, head: Image.Image) -> Image.Image:
    """Lay the matted head on the ground: a blurred copy of its own alpha as the shadow, then the head
    itself, its bottom crop past the tile's edge."""
    side = base.width
    width = int(side * HEAD_WIDTH)
    face = head.resize((width, width), Image.LANCZOS)
    x0 = (side - width) // 2
    y0 = int(side * HEAD_BOTTOM) - width

    placed = Image.new("L", (side, side), 0)
    placed.paste(face.split()[-1], (x0 + int(side * 0.012), y0 + int(side * 0.02)))
    shadow = placed.filter(ImageFilter.GaussianBlur(side * 0.022)).point(
        lambda v: int(v * SHADOW)
    )
    base = Image.composite(Image.new("RGBA", (side, side), (0, 0, 0, 255)), base, shadow)
    base.alpha_composite(face, (x0, y0))
    return base


def rounded(img: Image.Image, side: int, inset: float, corner: float) -> Image.Image:
    """The tile as a rounded square on transparent margin, which a Dock draws unmasked."""
    disc = int(side * inset)
    margin = (side - disc) // 2
    mask = Image.new("L", (side, side), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [margin, margin, side - margin, side - margin], radius=int(disc * corner), fill=255
    )
    out = img.resize((side, side), Image.LANCZOS).convert("RGBA")
    out.putalpha(mask)
    return out


def outlined(img: Image.Image, radius_inset: tuple[float, float], stroke) -> Image.Image:
    """The accent hairline a light ground carries, drawn inside the rounded shape so it does not
    square off corners that are meant to be transparent."""
    if stroke is None:
        return img
    colour, alpha = stroke
    side = img.width
    inset = radius_inset[0]
    layer = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    ImageDraw.Draw(layer).rounded_rectangle(
        [inset, inset, side - 1 - inset, side - 1 - inset],
        radius=radius_inset[1],
        outline=(*colour, alpha),
        width=1,
    )
    out = img.copy()
    out.alpha_composite(layer)
    return out