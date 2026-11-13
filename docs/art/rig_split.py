"""Split the 48x48 side-view base body (PixelLab state d2e7354b-4000-4996-b948-260da736fbf9, east) into
Growtopia-style rig parts. Every part is a full 48x48 layer, so parts line up by default and the engine only
rotates each one around its pivot.

usage: uv run --with pillow python docs/art/rig_split.py BASE_EAST.png assets/player_rig
writes <out>/<part>.png and <out>/rig.json (pivots, draw order, skin ramps)

Skin is snapped to a 5-colour ramp (SKIN_RAMPS["light"]). Recolour at load time by swapping those 5 colours
for another ramp: shading stays, outlines and clothes are untouched.
Part boxes below are tuned by hand for THIS sprite; a new base needs new boxes.
"""

import colorsys
import json
import sys
from pathlib import Path

from PIL import Image

SIZE = 48
STRAY = 8  # PixelLab leaves a stray pixel cluster in the top-left 8x8 corner of its rotations; ignore it
OUTLINE = (34, 24, 26, 255)

# 5 skin levels, highlight -> deep shadow. "light" is the base sprite's own colours.
SKIN_RAMPS = {
    "light": ["#f8dcb3", "#f5c596", "#e1936f", "#c37a60", "#8e5143"],
    "tan": ["#f0c08e", "#d99a68", "#b8704c", "#965740", "#6a3a2c"],
    "brown": ["#c98e62", "#a96d47", "#8a5236", "#6c3e2a", "#4a291d"],
    "dark": ["#9a6a4b", "#7c5038", "#5f3a28", "#47291e", "#2f1b14"],
    "pale": ["#fdeee0", "#f7dcc8", "#e8b9a0", "#cf9a84", "#9c6b5a"],
}

# part -> (pixel predicate on x, y), pivot. Draw order is back to front. Shorts stay on the torso so they never tear.
PARTS = {
    "head": (lambda x, y: y <= 20, (24, 20)),
    "torso": (lambda x, y: 21 <= y <= 40 and (y < 24 or y >= 38 or x >= 24), (27, 38)),
    "arm_front": (lambda x, y: 24 <= y <= 37 and 19 <= x <= 24, (21, 25)),
    "leg_front": (lambda x, y: y >= 41 and x <= 26, (24, 41)),
    "leg_back": (lambda x, y: y >= 41 and x >= 26, (28, 41)),
}
ORDER = ["arm_back", "leg_back", "leg_front", "torso", "head", "arm_front"]


def hexrgb(h):
    return tuple(int(h[i : i + 2], 16) for i in (1, 3, 5))


def is_skin(r, g, b):
    _, l, s = colorsys.rgb_to_hls(r / 255, g / 255, b / 255)
    return l >= 0.25 and s >= 0.15 and r > b


def snap_skin(im):
    ramp = [hexrgb(c) for c in SKIN_RAMPS["light"]]
    lum = lambda c: 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
    px = im.load()
    for y in range(SIZE):
        for x in range(SIZE):
            r, g, b, a = px[x, y]
            if a and is_skin(r, g, b):
                px[x, y] = min(ramp, key=lambda c: abs(lum(c) - lum((r, g, b)))) + (255,)


def far_arm(near):
    """Far arm = near arm shifted back 2px and one shade darker (it sits behind the torso).
    Skin moves one step down the ramp so skin-tone swapping still finds it."""
    ramp = [hexrgb(c) for c in SKIN_RAMPS["light"]]
    back = Image.new("RGBA", (SIZE, SIZE))
    back.paste(near, (2, 0))
    px = back.load()
    for y in range(SIZE):
        for x in range(SIZE):
            r, g, b, a = px[x, y]
            if not a:
                continue
            if (r, g, b) in ramp:
                px[x, y] = ramp[min(ramp.index((r, g, b)) + 1, len(ramp) - 1)] + (a,)
            else:
                px[x, y] = (int(r * 0.78), int(g * 0.78), int(b * 0.82), a)
    return back


def main():
    src, out = Image.open(sys.argv[1]).convert("RGBA"), Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    for y in range(STRAY):
        for x in range(STRAY):
            src.putpixel((x, y), (0, 0, 0, 0))
    snap_skin(src)

    layers = {}
    for name, (inside, _) in PARTS.items():
        layer = Image.new("RGBA", (SIZE, SIZE))
        for y in range(SIZE):
            for x in range(SIZE):
                if inside(x, y) and src.getpixel((x, y))[3]:
                    layer.putpixel((x, y), src.getpixel((x, y)))
        layers[name] = layer

    # torso lost its left edge under the arm: give each row an outline pixel on its leftmost column
    t = layers["torso"]
    for y in range(24, 38):
        xs = [x for x in range(SIZE) if t.getpixel((x, y))[3]]
        if xs:
            t.putpixel((xs[0], y), OUTLINE)

    layers["arm_back"] = far_arm(layers["arm_front"])

    # check: every opaque source pixel is drawn by some front-facing part (arm_back is extra)
    missing = [
        (x, y)
        for y in range(SIZE)
        for x in range(SIZE)
        if src.getpixel((x, y))[3] and not any(layers[n].getpixel((x, y))[3] for n in PARTS)
    ]
    assert not missing, f"pixels not covered by any part: {missing[:10]}"

    for name, layer in layers.items():
        layer.save(out / f"{name}.png")
    pivots = {n: p for n, (_, p) in PARTS.items()}
    pivots["arm_back"] = (pivots["arm_front"][0] + 2, pivots["arm_front"][1])
    rig = {"size": SIZE, "facing": "east", "order": ORDER, "pivots": pivots, "skin_ramps": SKIN_RAMPS}
    (out / "rig.json").write_text(json.dumps(rig, indent=2))
    print("wrote", out, "parts:", ", ".join(ORDER))


if __name__ == "__main__":
    main()
