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
    assert ring.size == (192, 24) and puff.size == (192, 32) and sparkle.size == (64, 16)
    print("wrote assets/ui/respawn_ring.png, assets/fx/respawn_puff.png + .json, assets/fx/ready_sparkle.png + .json")


if __name__ == "__main__":
    main()
