"""Convert PixelLab sidescroller tilesets (corner Wang, 16 tiles) into the 4-bit edge-mask atlas
described in tileset-terrain.md: index = terrain * 16 + mask, 8 cells per row, 32x32 each.

usage: uv run --with pillow python docs/art/wang_to_mask.py OUT.png NAME [NAME ...]
       each NAME reads NAME_raw.png + NAME_meta.json next to OUT.png (order = terrain id)

Each 32x32 cell is built from four 16x16 quadrants taken from the Wang tile that sits on that corner
of the cell. In PixelLab metadata "lower" = solid terrain, "upper" = air.
ponytail: the diagonal neighbor is assumed solid when both sides are, so inner corners (L-shapes with
an empty diagonal) look like flat fill. Add an 8-bit (47-tile) layout if that ever shows.
"""

import json
import sys
from pathlib import Path

from PIL import Image

T, H = 32, 16
UP, RIGHT, DOWN, LEFT = 1, 2, 4, 8


def load(path_png, path_meta):
    im = Image.open(path_png).convert("RGBA")
    tiles = {}
    for t in json.load(open(path_meta))["tileset_data"]["tiles"]:
        c = {k: v == "lower" for k, v in t["corners"].items()}
        b = t["bounding_box"]
        tiles[(c["NW"], c["NE"], c["SW"], c["SE"])] = im.crop((b["x"], b["y"], b["x"] + T, b["y"] + T))
    assert len(tiles) == 16, f"{path_png}: expected 16 wang tiles, got {len(tiles)}"
    return tiles


def cell(tiles, mask):
    u, r, d, l = (bool(mask & b) for b in (UP, RIGHT, DOWN, LEFT))
    out = Image.new("RGBA", (T, T))
    # (wang corners NW,NE,SW,SE), quadrant of that wang tile to take, where it goes in the cell
    parts = [
        ((u and l, u, l, True), (H, H), (0, 0)),  # top-left
        ((u, u and r, True, r), (0, H), (H, 0)),  # top-right
        ((l, True, l and d, d), (H, 0), (0, H)),  # bottom-left
        ((True, r, d, d and r), (0, 0), (H, H)),  # bottom-right
    ]
    for key, (sx, sy), dst in parts:
        out.paste(tiles[key].crop((sx, sy, sx + H, sy + H)), dst)
    return out


def main():
    out_path, names = Path(sys.argv[1]), sys.argv[2:]
    atlas = Image.new("RGBA", (8 * T, 8 * T))
    for terrain, name in enumerate(names):
        tiles = load(out_path.parent / f"{name}_raw.png", out_path.parent / f"{name}_meta.json")
        for mask in range(16):
            i = terrain * 16 + mask
            atlas.paste(cell(tiles, mask), ((i % 8) * T, (i // 8) * T))
        fill = cell(tiles, 15)
        assert fill.getextrema()[3] == (255, 255),f"{name}: mask 15 (interior) must be fully opaque"
    atlas.save(out_path)
    print("wrote", out_path)


if __name__ == "__main__":
    main()
