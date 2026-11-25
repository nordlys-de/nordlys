#!/usr/bin/env python3
"""Render the default Nordlys wallpaper: soft aurora orbs on near-black,
the same colours as the design (needs Pillow).

    python3 scripts/make-wallpaper.py session/wallpaper.jpg [width height]
"""
import sys
from PIL import Image, ImageDraw, ImageFilter

out = sys.argv[1] if len(sys.argv) > 1 else "wallpaper.png"
W, H = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (3840, 2160)
s = W / 1440  # the design canvas is 1440 wide

base = Image.new("RGB", (W, H), (10, 13, 19))
# (x, y, diameter, colour, opacity, blur) from the design's orbs
orbs = [
    (-120, -160, 760, (47, 79, 201), 0.55, 110),
    (820, 380, 720, (109, 63, 217), 0.45, 120),
    (520, 120, 380, (14, 159, 154), 0.35, 100),
]
for x, y, d, colour, alpha, blur in orbs:
    layer = Image.new("RGB", (W, H), colour)
    mask = Image.new("L", (W, H), 0)
    ImageDraw.Draw(mask).ellipse(
        [x * s, y * s, (x + d) * s, (y + d) * s], fill=int(255 * alpha)
    )
    mask = mask.filter(ImageFilter.GaussianBlur(blur * s))
    base = Image.composite(layer, base, mask)

# A touch of grain so gradients don't band.
noise = Image.effect_noise((W, H), 6).convert("RGB")
base = Image.blend(base, noise, 0.025)
base.save(out, optimize=True, quality=92)
