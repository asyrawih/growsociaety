"""Add hand-drawn glyphs to the PixelLab pixel font.

usage: uv run --with fonttools python docs/art/font_add_glyphs.py
reads  assets/fonts/source/growsociaety_pixel_pixellab.ttf (PixelLab's original, never edited)
writes assets/fonts/growsociaety_pixel.ttf (the font the game loads)

The PixelLab font is pixels on a 16 px grid: 1 px = 64 font units, baseline at y = 0, capitals 12 px tall, 1 px
gap after each glyph, every pixel run one clockwise rectangle. New glyphs are drawn below as rows of '#' (top row
first) in the same heavy 3-4 px stroke, plus the row index the bottom row sits on.
"""

from pathlib import Path

from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent.parent
SOURCE = ROOT / "assets/fonts/source/growsociaety_pixel_pixellab.ttf"
OUT = ROOT / "assets/fonts/growsociaety_pixel.ttf"
PX = 64

# char: (bottom row's y in pixels, rows from top to bottom)
GLYPHS = {
    "#": (0, """
..###..###..
..###..###..
############
############
..###..###..
..###..###..
..###..###..
############
############
..###..###..
..###..###..
..###..###..
"""),
    "·": (4, """
####
####
####
####
"""),
    "|": (0, """
####
####
####
####
####
####
####
####
####
####
####
####
####
####
####
"""),
    "[": (0, """
######
######
####..
####..
####..
####..
####..
####..
####..
####..
####..
####..
####..
######
######
"""),
    "]": (0, """
######
######
..####
..####
..####
..####
..####
..####
..####
..####
..####
..####
..####
######
######
"""),
    "<": (0, """
......####
....######
..######..
######....
####......
####......
######....
..######..
....######
......####
"""),
    ">": (0, """
####......
######....
..######..
....######
......####
......####
....######
..######..
######....
####......
"""),
    "×": (2, """
###..###
.######.
..####..
..####..
.######.
###..###
"""),
    "&": (0, """
..#####.....
.#######....
.###.###....
.###.###....
..#####.....
.######..###
###.####.###
###..#######
###...#####.
###...######
.###########
..######.###
"""),
    "@": (0, """
..########..
.##########.
###......###
##..########
##.###..####
##.##...####
##.##...####
##.###.#####
##..####.##.
###.........
.##########.
..########..
"""),
}


def glyph_for(rows, bottom, glyf):
    rows = [r for r in rows.strip("\n").split("\n")]
    pen = TTGlyphPen(None)
    height = len(rows)
    for i, row in enumerate(rows):
        y0 = (bottom + height - 1 - i) * PX
        x = 0
        while x < len(row):
            if row[x] != "#":
                x += 1
                continue
            start = x
            while x < len(row) and row[x] == "#":
                x += 1
            x0, x1, y1 = start * PX, x * PX, y0 + PX
            pen.moveTo((x0, y0))
            pen.lineTo((x0, y1))
            pen.lineTo((x1, y1))
            pen.lineTo((x1, y0))
            pen.closePath()
    width = max(len(r) for r in rows)
    return pen.glyph(), (width + 1) * PX


def main():
    font = TTFont(SOURCE)
    glyf, hmtx, order = font["glyf"], font["hmtx"], font.getGlyphOrder()
    for char, (bottom, rows) in GLYPHS.items():
        name = f"uni{ord(char):04X}"
        glyph, advance = glyph_for(rows, bottom, glyf)
        glyf[name] = glyph
        hmtx[name] = (advance, 0)
        if name not in order:
            order.append(name)
        for table in font["cmap"].tables:
            table.cmap[ord(char)] = name
    font.setGlyphOrder(order)
    font["maxp"].numGlyphs = len(order)
    font.save(OUT)
    covered = font.getBestCmap()
    missing = [c for c in GLYPHS if ord(c) not in covered]
    assert not missing, f"not mapped: {missing}"
    print("wrote", OUT.relative_to(ROOT), "with", len(GLYPHS), "added glyphs:", "".join(GLYPHS))


if __name__ == "__main__":
    main()
