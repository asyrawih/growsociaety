"""Make 20 equipment items with ONE PixelLab inpaint call (10 generations).

1. build:  uv run --with pillow python docs/art/item_sheet.py build SLOT OUT_DIR
   writes OUT_DIR/sheet.png (5x4 grid of the bald base body, 240x192) and OUT_DIR/mask.png (SLOT area in every cell)
2. PixelLab inpaint_image(image=sheet.png, mask=mask.png, description="20 identical characters; give EACH a
   DIFFERENT <item>: <list of 20>"). Outside the mask nothing changes, so every body stays pixel-identical to the base.
3. split:  uv run --with pillow python docs/art/item_sheet.py split RESULT.png PREFIX name1 name2 ... (20 names, row by row)
   writes assets/items/PREFIX_<name>/source_east.png and extracts the rig parts (item_extract.py)

Masks are rectangles in 48x48 cell coordinates (x0, y0, x1, y1 inclusive). Items can't grow outside their mask,
so a long ponytail or wide brim gets a flat edge where the mask ends: widen the box if that matters.
"""

import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
from item_extract import extract  # noqa: E402

ROOT = HERE.parent.parent
BASE = ROOT / "assets/player_rig/source/base_east.png"
CELL, COLS, ROWS = 48, 5, 4

SLOT_MASKS = {
    # top of head + back of head, face left out so eyes don't get redrawn
    "head": [(8, 0, 40, 12), (8, 13, 23, 22)],
    "shirt": [(16, 21, 35, 38)],  # torso + near arm
    "pants": [(16, 38, 35, 44)],  # waist to ankles; starts below the hand, feet left out
    "shoes": [(16, 41, 35, 47)],
    "wings": [(0, 4, 30, 46)],  # behind the back (character faces east); body pixels are cut out below
}
# slots drawn behind the whole body: the mask skips every body pixel, and the item is one "back" layer
BEHIND_BODY = {"wings"}


def build(slot, out):
    out.mkdir(parents=True, exist_ok=True)
    base = Image.open(BASE).convert("RGBA")
    for y in range(8):  # drop PixelLab's stray corner cluster
        for x in range(8):
            base.putpixel((x, y), (0, 0, 0, 0))
    cell_mask = Image.new("L", (CELL, CELL), 0)
    for box in SLOT_MASKS[slot]:
        ImageDraw.Draw(cell_mask).rectangle(box, fill=255)
    if slot in BEHIND_BODY:
        for y in range(CELL):
            for x in range(CELL):
                if base.getpixel((x, y))[3]:
                    cell_mask.putpixel((x, y), 0)
    sheet =Image.new("RGBA", (CELL * COLS, CELL * ROWS))
    mask = Image.new("L", sheet.size, 0)
    for i in range(COLS * ROWS):
        at = ((i % COLS) * CELL, (i // COLS) * CELL)
        sheet.paste(base, at)
        mask.paste(cell_mask, at)
    sheet.save(out / "sheet.png")
    mask.save(out / "mask.png")
    print("wrote", out / "sheet.png", out / "mask.png")


def split(result, prefix, names):
    assert len(names) == COLS * ROWS, f"need {COLS * ROWS} names, got {len(names)}"
    sheet = Image.open(result).convert("RGBA")
    for i, name in enumerate(names):
        item = ROOT / "assets/items" / f"{prefix}_{name}"
        assert not item.exists(), f"{item} already exists, pick another name"
        item.mkdir(parents=True)
        x, y = (i % COLS) * CELL, (i // COLS) * CELL
        cell = sheet.crop((x, y, x + CELL, y + CELL))
        cell.save(item / "source_east.png")
        if prefix in BEHIND_BODY:  # one layer behind everything, pivot = torso
            extract(Image.open(BASE).convert("RGBA"), cell).save(item / "back.png")
            print("wrote", item, "parts: back")
            continue
        subprocess.run(
            [sys.executable, str(HERE / "item_extract.py"), str(BASE), str(item / "source_east.png"), str(item)],
            check=True,
        )


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "build":
        build(sys.argv[2], Path(sys.argv[3]))
    elif cmd == "split":
        split(sys.argv[2], sys.argv[3], sys.argv[4:])
    else:
        sys.exit(__doc__)
