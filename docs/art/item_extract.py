"""Cut an equipment item out of a PixelLab state of the base body and split it into rig parts.

usage: uv run --with pillow python docs/art/item_extract.py BASE_EAST.png ITEM_EAST.png assets/items/NAME
The item is a create_character_state of the base (d2e7354b-...) with only the item added, so its pose matches
pixel for pixel. Item pixels = pixels that differ from the base, minus tiny clusters (re-render noise).
ponytail: an item coloured like skin (brown leather on dark skin) blends in; that's the item's palette, not a bug.
Writes one 48x48 layer per rig part the item touches, drawn on top of that body part.
"""

import sys
from pathlib import Path

from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))
from rig_split import PARTS, SIZE, STRAY, far_arm  # noqa: E402

THRESHOLD = 40  # summed RGBA difference that counts as "changed"
MIN_CLUSTER = 4  # changed-pixel clusters smaller than this are re-render noise


def extract(base, item):
    mask = Image.new("RGBA", (SIZE, SIZE))
    for y in range(SIZE):
        for x in range(SIZE):
            if x < STRAY and y < STRAY:
                continue
            p, q = item.getpixel((x, y)), base.getpixel((x, y))
            if p[3] and sum(abs(a - b) for a, b in zip(p, q)) > THRESHOLD:
                mask.putpixel((x, y), p)
    # drop small specks (diff noise): 8-connected clusters under MIN_CLUSTER pixels
    seen = set()
    for y in range(SIZE):
        for x in range(SIZE):
            if (x, y) in seen or not mask.getpixel((x, y))[3]:
                continue
            cluster, stack = [], [(x, y)]
            seen.add((x, y))
            while stack:
                cx, cy = stack.pop()
                cluster.append((cx, cy))
                for dx in (-1, 0, 1):
                    for dy in (-1, 0, 1):
                        n = (cx + dx, cy + dy)
                        if 0 <= n[0] < SIZE and 0 <= n[1] < SIZE and n not in seen and mask.getpixel(n)[3]:
                            seen.add(n)
                            stack.append(n)
            if len(cluster) < MIN_CLUSTER:
                for c in cluster:
                    mask.putpixel(c, (0, 0, 0, 0))
    return mask


def main():
    base, item = (Image.open(p).convert("RGBA") for p in sys.argv[1:3])
    out = Path(sys.argv[3])
    out.mkdir(parents=True, exist_ok=True)
    pixels = extract(base, item)
    assert pixels.getbbox(), "no item pixels found: is ITEM a state of BASE?"
    written = []
    for name, (inside, _) in PARTS.items():
        layer = Image.new("RGBA", (SIZE, SIZE))
        for y in range(SIZE):
            for x in range(SIZE):
                if inside(x, y):
                    layer.putpixel((x, y), pixels.getpixel((x, y)))
        if layer.getbbox():
            layer.save(out / f"{name}.png")
            written.append(name)
    # pixels outside every part box (hair sticking out, a coat tail behind the back) go to the part at that height
    rests = {}
    for y in range(SIZE):
        for x in range(SIZE):
            if pixels.getpixel((x, y))[3] and not any(inside(x, y) for inside, _ in PARTS.values()):
                part = "head" if y <= 20 else "torso" if y <= 40 else "leg_front"
                rests.setdefault(part, Image.new("RGBA", (SIZE, SIZE))).putpixel((x, y), pixels.getpixel((x, y)))
    for part, rest in rests.items():
        layer = Image.open(out / f"{part}.png") if part in written else Image.new("RGBA", (SIZE, SIZE))
        layer.alpha_composite(rest)
        layer.save(out / f"{part}.png")
        written = sorted(set(written) | {part})
    if "arm_front" in written:  # the far arm wears the same sleeve
        far_arm(Image.open(out / "arm_front.png").convert("RGBA")).save(out / "arm_back.png")
        written.append("arm_back")
    print("wrote", out, "parts:", ", ".join(written))


if __name__ == "__main__":
    main()
