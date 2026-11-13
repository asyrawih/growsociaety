"""Make a 32x32 icon.png for every equipment item that has none, from its own rig layers (no body).

usage: uv run --with pillow python docs/art/item_icons.py
The item's layers are stacked as worn, trimmed to what they draw and centred on a 32x32 canvas. Anything bigger
than 32 px (wings, long hair) is shrunk by the smallest whole factor that fits; small pieces (shoes) grow by the
largest whole factor that fits. Pixels stay square either way.
Items that already have icon.png (pickaxes) are left alone.
"""

import json
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent.parent
ICON = 32


def icon(item_dir, order):
    stack = Image.new("RGBA", (48, 48))
    for part in ["back"] + order:
        p = item_dir / f"{part}.png"
        if p.exists():
            stack.alpha_composite(Image.open(p).convert("RGBA"))
    box = stack.getbbox()
    if not box:
        return None
    art = stack.crop(box)
    factor = max(1, -(-max(art.size) // ICON))  # ceil
    if factor > 1:
        art = art.resize((art.width // factor, art.height // factor), Image.NEAREST)
    grow = (ICON - 2) // max(art.size)  # small pieces (shoes, sleeves) grow by a whole factor to fill the slot
    if grow > 1:
        art = art.resize((art.width * grow, art.height * grow), Image.NEAREST)
    out = Image.new("RGBA", (ICON, ICON))
    out.alpha_composite(art, ((ICON - art.width) // 2, (ICON - art.height) // 2))
    return out


def main():
    order = json.loads((ROOT / "assets/player_rig/rig.json").read_text())["order"]
    made = 0
    for item_dir in sorted(p for p in (ROOT / "assets/items").iterdir() if p.is_dir()):
        if (item_dir / "icon.png").exists():
            continue
        im = icon(item_dir, order)
        assert im is not None, f"{item_dir.name}: no layers to build an icon from"
        im.save(item_dir / "icon.png")
        made += 1
    print("icons written:", made)


if __name__ == "__main__":
    main()
