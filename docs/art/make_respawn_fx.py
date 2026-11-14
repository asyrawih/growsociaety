"""Draw the respawn art for docs/knowledge/ux/respawn.md (no generation; exact pixel geometry).

usage: uv run --with pillow python docs/art/make_respawn_fx.py
writes assets/ui/respawn_ring.png  (8 frames x 24x24, strip 192x24): hold-to-go-home ring, fills clockwise
       assets/fx/respawn_puff.png  (6 frames x 32x32, strip 192x32) + respawn_puff.json (pixelfx/Aseprite sidecar,
       same layout as rock-break-coins.json)
"""

import json
import math
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent.parent
OUTLINE = (43, 26, 22, 255)  # the dark outline used across the UI kit
FILL = (255, 210, 122, 255)  # warm light gold
FILL_LIGHT = (255, 239, 196, 255)  # inner highlight row
TRACK = (43, 26, 22, 110)  # unfilled part: a faint dark groove, so progress reads


def ring_frame(fraction):
    size, c = 24, 11.5
    im = Image.new("RGBA", (size, size))
    px = im.load()
    for y in range(size):
        for x in range(size):
            d = math.hypot(x - c, y - c)
            # clockwise from 12 o'clock, 0..1
            a = (math.degrees(math.atan2(x - c, -(y - c))) % 360) / 360
            filled = a < fraction
            if 7.5 <= d < 10.5:
                px[x, y] = (FILL_LIGHT if d >= 9.5 else FILL) if filled else TRACK
            elif 10.5 <= d < 11.5 or 6.5 <= d < 7.5:
                px[x, y] = OUTLINE if filled else (OUTLINE[:3] + (150,))
    return im


def puff_frame(i, frames=6):
    size, c = 32, 15.5
    t = i / (frames - 1)  # 0..1
    im = Image.new("RGBA", (size, size))
    px = im.load()
    alpha = int(235 * (1 - t) ** 1.2) + 20 * (i < frames - 1)
    # soft dust: 8 round blobs flying out and shrinking
    for k in range(8):
        ang = k * math.tau / 8 + 0.35
        dist = 3 + 10 * t ** 0.8
        bx, by = c + math.cos(ang) * dist, c + math.sin(ang) * dist * 0.8 - 2 * t
        r = 3.2 * (1 - 0.6 * t)
        for y in range(size):
            for x in range(size):
                if math.hypot(x - bx, y - by) < r:
                    shade = 255 if (x - bx) + (y - by) < 0 else 214  # light from the top-left
                    px[x, y] = (shade, shade, shade - 10, max(px[x, y][3], alpha))
    # a few sparkle crosses that blink in the middle frames
    if 1 <= i <= 3:
        for sx, sy in ((c - 7, c - 6), (c + 8, c - 3), (c + 1, c + 8)):
            for dx, dy in ((0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)):
                x, y = int(sx + dx), int(sy + dy)
                px[x, y] = (255, 247, 214, 255)
    return im


def sparkle_frame(i):
    """'Ready to harvest' glint (docs/knowledge/ux/farming-feedback.md): a 4-point star that grows, flashes, fades."""
    arm, alpha, core = [(2, 255, (255, 230, 160)), (4, 255, (255, 244, 205)), (6, 255, (255, 255, 240)),
                        (3, 150, (255, 230, 160))][i]
    im = Image.new("RGBA", (16, 16))
    px = im.load()
    lit = set()
    for c in (7, 8):  # 2x2 core so the star sits on the exact centre of a 16 px frame
        for d in (7, 8):
            lit.add((c, d))
    for k in range(1, arm + 1):  # arms taper: the last pixel of each arm is the dimmer gold
        for c in (7, 8):
            lit |= {(c, 7 - k), (c, 8 + k)} if k < arm else {(c, 7 - k), (c, 8 + k)} & {(7, 7 - k), (8, 8 + k)} | {(8, 7 - k), (7, 8 + k)}
            lit |= {(7 - k, c), (8 + k, c)} if k < arm else {(7 - k, 8), (8 + k, 7)}
    if i == 2:  # flash frame: short diagonal rays
        lit |= {(5, 5), (10, 10), (5, 10), (10, 5)}
    for x, y in lit:
        if 0 <= x < 16 and 0 <= y < 16:
            px[x, y] = core + (alpha,)
    # 1 px dark outline around everything lit, so it reads on sky and cave
    for x, y in list(lit):
        for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            n = (x + dx, y + dy)
            if n not in lit and 0 <= n[0] < 16 and 0 <= n[1] < 16 and px[n][3] == 0:
                px[n] = OUTLINE[:3] + (int(alpha * 0.8),)
    return im


def _blob(px, size, cx, cy, r, color, alpha):
    for y in range(max(0, int(cy - r - 1)), min(size, int(cy + r + 2))):
        for x in range(max(0, int(cx - r - 1)), min(size, int(cx + r + 2))):
            if math.hypot(x - cx, y - cy) < r:
                lit = (x - cx) + (y - cy) < 0  # light from the top-left
                c = color[0] if lit else color[1]
                px[x, y] = c + (max(px[x, y][3], alpha),)


def dust_frame(i, n=6):
    """Block break (docs/knowledge/ux/vfx-farming-punch.md): near-white dust + chips, tinted per block at runtime."""
    t = i / (n - 1)
    im = Image.new("RGBA", (32, 32))
    px = im.load()
    a = int(255 * (1 - t) ** 1.1)
    for k in range(7):  # dust: soft puffs drifting out
        ang = k * math.tau / 7 + 0.2
        d = 2 + 9 * t ** 0.7
        _blob(px, 32, 15.5 + math.cos(ang) * d, 15.5 + math.sin(ang) * d * 0.7, 3.4 * (1 - 0.5 * t),
              ((245, 245, 245), (200, 200, 200)), int(a * 0.8))
    for k in range(6):  # chips: small squares thrown out on arcs, with gravity
        ang = k * math.tau / 6 + 0.6
        d = 3 + 12 * t
        x = int(15.5 + math.cos(ang) * d)
        y = int(15.5 + math.sin(ang) * d * 0.8 - 6 * t + 14 * t * t)
        for dx in (0, 1):
            for dy in (0, 1):
                if 0 <= x + dx < 32 and 0 <= y + dy < 32 and a:
                    v = 230 if dx + dy == 0 else 150  # value contrast only, no hue
                    px[x + dx, y + dy] = (v, v, v, a)
    return im


def soil_frame(i, n=5):
    """Seed planted: a low, wide brown soil puff hugging the ground (bottom of the frame)."""
    t = i / (n - 1)
    im = Image.new("RGBA", (32, 32))
    px = im.load()
    a = int(240 * (1 - t) ** 1.2)
    for k in range(6):
        side = -1 if k % 2 else 1
        spread = (k // 2 + 1) * (2 + 4 * t)
        _blob(px, 32, 15.5 + side * spread, 27 - 2.5 * t - (k // 2) * 0.6, 3.2 * (1 - 0.45 * t),
              ((168, 120, 84), (107, 73, 55)), a)
    _blob(px, 32, 15.5, 27.5, 3.6 * (1 - 0.6 * t), ((168, 120, 84), (107, 73, 55)), a)
    return im


def swirl_frame(i, n=8):
    """Splice: green leaves + violet sparks on a spiral that tightens into the centre."""
    t = i / (n - 1)
    im = Image.new("RGBA", (32, 32))
    px = im.load()
    a = 255 if t < 0.85 else int(255 * (1 - t) / 0.15)
    for k in range(10):
        phase = k / 10
        r = 13 * (1 - t) * (0.55 + 0.45 * phase) + 1
        ang = phase * math.tau * 1.5 + t * math.tau * 1.2
        x, y = int(15.5 + math.cos(ang) * r), int(15.5 + math.sin(ang) * r)
        if k % 2 == 0:  # leaf: 2x2 green with a darker corner
            for dx, dy, c in ((0, 0, (147, 170, 54)), (1, 0, (147, 170, 54)), (0, 1, (147, 170, 54)), (1, 1, (70, 113, 67))):
                if 0 <= x + dx < 32 and 0 <= y + dy < 32:
                    px[x + dx, y + dy] = c + (a,)
        else:  # violet spark: small plus
            for dx, dy in ((0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)):
                if 0 <= x + dx < 32 and 0 <= y + dy < 32:
                    px[x + dx, y + dy] = ((200, 180, 255) if (dx, dy) == (0, 0) else (122, 97, 194)) + (a,)
    if i == n - 1:  # final frame: a bright pop at the centre
        _blob(px, 32, 15.5, 15.5, 2.5, ((230, 220, 255), (147, 170, 54)), 200)
    return im


def drop_glow():
    """Under seed drops: soft warm ring, transparent centre (the code pulses its alpha)."""
    im = Image.new("RGBA", (16, 16))
    px = im.load()
    for y in range(16):
        for x in range(16):
            d = math.hypot(x - 7.5, y - 7.5)
            a = max(0.0, 1 - abs(d - 5.0) / 2.6)  # peak on a ring of radius 5, fading both ways
            if a > 0.05 and d > 2:
                px[x, y] = (255, 214, 140, int(200 * a))
    return im


def chip(active):
    """Inventory category filter chip: small wood pill like ui/tab.png, 40x20, 9-slice inset 6."""
    w, h, r = 40, 20, 4
    im = Image.new("RGBA", (w, h))
    px = im.load()
    fill = (242, 217, 174) if active else (77, 42, 31)
    for y in range(h):
        for x in range(w):
            # rounded-rectangle mask, radius r
            cx = min(max(x, r), w - 1 - r)
            cy = min(max(y, r), h - 1 - r)
            if math.hypot(x - cx, y - cy) > r + 0.3:
                continue
            edge = math.hypot(x - cx, y - cy) > r - 0.7 or x in (0, w - 1) or y in (0, h - 1)
            if edge:
                px[x, y] = (5, 1, 1, 255)
            elif y == 1:
                px[x, y] = (242, 195, 114, 255)  # gold top trim, as on tab.png
            elif y == 2 or x == 1:
                px[x, y] = (179, 120, 54, 255)
            elif y == h - 2:
                px[x, y] = (50, 27, 25, 255)  # bottom shade
            else:
                px[x, y] = fill + (255,)
    return im


def icon_search():
    """16x16 magnifier in the settings-icon style: gold rim, cream glass, wood handle, dark outline."""
    im = Image.new("RGBA", (16, 16))
    px = im.load()
    cx, cy = 6.0, 6.0
    for y in range(16):
        for x in range(16):
            d = math.hypot(x - cx, y - cy)
            if d < 3.2:
                px[x, y] = (242, 232, 213, 255) if (x - cx) + (y - cy) > -2 else (255, 255, 255, 255)
            elif d < 4.6:
                px[x, y] = (230, 170, 70, 255) if (x - cx) + (y - cy) < 0 else (179, 120, 54, 255)
            elif d < 5.6:
                px[x, y] = (43, 26, 22, 255)
    for k in range(5):  # handle down-right
        for o in (0, 1):
            x, y = 10 + k, 10 + k + o
            if x < 16 and y < 16:
                px[x, y] = (151, 96, 64, 255) if o == 0 else (107, 73, 55, 255)
    for k in range(5):  # handle outline
        for x, y in ((9 + k, 11 + k), (11 + k, 10 + k)):
            if 0 <= x < 16 and 0 <= y < 16 and px[x, y][3] == 0:
                px[x, y] = (43, 26, 22, 255)
    return im


def strip(frames, w, h):
    out = Image.new("RGBA", (w * len(frames), h))
    for i, f in enumerate(frames):
        out.paste(f, (i * w, 0))
    return out


def sidecar(name, frames, w, h, fps):
    ms = round(1000 / fps)
    return {
        "frames": [
            {
                "filename": f"{name} {i}",
                "frame": {"x": i * w, "y": 0, "w": w, "h": h},
                "rotated": False,
                "trimmed": False,
                "spriteSourceSize": {"x": 0, "y": 0, "w": w, "h": h},
                "sourceSize": {"w": w, "h": h},
                "duration": ms,
            }
            for i in range(frames)
        ],
        "meta": {
            "app": "growsociaety docs/art/make_respawn_fx.py",
            "version": "1",
            "image": f"{name}.png",
            "format": "RGBA8888",
            "size": {"w": w * frames, "h": h},
            "scale": "1",
            "frameTags": [{"name": name, "from": 0, "to": frames - 1, "direction": "forward"}],
            "pixelfx": {
                "id": f"spawn/{name}",
                "name": name.replace("-", " ").replace("_", " ").capitalize(),
                "fps": fps,
                "loop": False,
                "frames": frames,
                "frameSize": [w, h],
                "pivot": [w // 2, h // 2],
                "columns": frames,
            },
        },
    }


def main():
    ring = strip([ring_frame((k + 1) / 8) for k in range(8)], 24, 24)
    ring.save(ROOT / "assets/ui/respawn_ring.png")
    puff = strip([puff_frame(i) for i in range(6)], 32, 32)
    puff.save(ROOT / "assets/fx/respawn_puff.png")
    (ROOT / "assets/fx/respawn_puff.json").write_text(json.dumps(sidecar("respawn_puff", 6, 32, 32, 15), indent=1))
    sparkle = strip([sparkle_frame(i) for i in range(4)], 16, 16)
    sparkle.save(ROOT / "assets/fx/ready_sparkle.png")
    (ROOT / "assets/fx/ready_sparkle.json").write_text(json.dumps(sidecar("ready_sparkle", 4, 16, 16, 10), indent=1))
    for name, frames in (("block_break_dust", [dust_frame(i) for i in range(6)]),
                         ("soil_puff", [soil_frame(i) for i in range(5)]),
                         ("splice_swirl", [swirl_frame(i) for i in range(8)])):
        strip(frames, 32, 32).save(ROOT / f"assets/fx/{name}.png")
        (ROOT / f"assets/fx/{name}.json").write_text(json.dumps(sidecar(name, len(frames), 32, 32, 12), indent=1))
    glow = drop_glow()
    glow.save(ROOT / "assets/fx/drop_glow.png")
    (ROOT / "assets/fx/drop_glow.json").write_text(json.dumps(sidecar("drop_glow", 1, 16, 16, 12), indent=1))
    chip(False).save(ROOT / "assets/ui/chip.png")
    chip(True).save(ROOT / "assets/ui/chip_active.png")
    icon_search().save(ROOT / "assets/ui/icon_search.png")
    print("wrote block_break_dust, soil_puff, splice_swirl, drop_glow (fx) + chip, chip_active, icon_search (ui)")
    assert ring.size == (192, 24) and puff.size == (192, 32) and sparkle.size == (64, 16)
    print("wrote assets/ui/respawn_ring.png, assets/fx/respawn_puff.png + .json, assets/fx/ready_sparkle.png + .json")


if __name__ == "__main__":
    main()
