# Terrain Tileset: 256×256 atlas, 4-bit autotile

Init/guide image: [`terrain-layout-guide.png`](terrain-layout-guide.png). Style reference: [`style-board.png`](style-board.png).

## Layout (the code relies on this, don't change it)

`index = terrain * 16 + mask`, cell = `(index % 8, index / 8)`, each 32×32.

| Rows | Terrain |
|---|---|
| 0–1 | Grass (dirt body, grass on top) |
| 2–3 | Dirt |
| 4–5 | Stone |
| 6–7 | Sand |

`mask` = which neighbors are the **same** material: `1` = up, `2` = right, `4` = down, `8` = left.
A bit that is not set means that side is **open**, so draw an edge there (grass/rim/top highlight).

| mask | open sides | meaning |
|---|---|---|
| 0 | all | single floating block |
| 15 | none | interior fill |
| 14 | up | top surface (grass top / stone top) |
| 12 | up, right | top-right outer corner |
| 6 | up, left | top-left outer corner |
| 11 | down | underside |
| … | | the other combinations follow the bits |

Corners, edges and "transitions" are all covered by the mask: a neighbor of a different material counts as open,
so grass→dirt is simply a grass tile (top drawn) sitting on dirt. No separate transition tiles are needed.

## Workflow in PixelLab

Generating all 64 cells in one pass rarely comes out correct. Do it **per material** (16 tiles = 2 rows, 256×64):

1. Crop 2 rows from the guide (e.g. rows 0–1 for grass) and use them as the **init image** (strength ~0.3–0.5,
   so the edge positions are kept but the texture is fully redrawn).
2. Style reference = `style-board.png`.
3. Prompt below, swapping `[TERRAIN]`.
4. Stack the 4 results into one 256×256 → `assets/tiles/terrain.png`.
5. Check: put mask 15 in a 3×3 grid (no seams), and line up masks 14 + 12 + 6 (the surface must connect).

## Prompt

```
Seamless terrain tile strip for an original 2D side-view sandbox platformer.
Match the attached style reference: rich painterly pixel art, warm earthy palette, soft natural
light from the top-left, controlled pixel clusters, consistent pixel density.

Content: [TERRAIN] blocks, 16 tiles of 32x32 px in an 8x2 grid (256x64 total), following the
init image exactly: each cell is a full solid block; the lighter bands in the init image mark
the OPEN edges of that cell. Draw the [TERRAIN] edge treatment only on those open edges and
make closed edges continue seamlessly into the neighboring tile.

Keep every tile fully inside its 32x32 cell, no shadows crossing cell borders, same light
direction on every tile, no perspective, orthographic side view.
```

`[TERRAIN]`:
- **Grass:** `grass-topped dirt: thick lush grass lip with small blades and tiny flowers on open top edges, rich brown dirt body with small stones and roots, slight grass overhang on open side edges`
- **Dirt:** `rich brown dirt with small embedded pebbles and thin roots, darker rim on open edges`
- **Stone:** `grey natural rock with irregular stone shapes and subtle cracks, light chipped highlight on open top edges, cool dark rim on other open edges`
- **Sand:** `warm fine sand with subtle grain and tiny pebbles, soft lighter crest on open top edges`

## Negative prompt

```
labels, text, grid lines, borders, frames, mockup, character, scenery, UI, isometric, top-down,
perspective, 3D render, photorealistic, blur, smooth gradient, gaps between tiles,
inconsistent lighting, tile content crossing cell borders
```

## Cut from the original prompt (on purpose)

- **Separate transition tiles:** already covered by the mask (see above).
- **Variants (cracked/mossy stone, dirt texture variations):** the 64 cells are already full. Add them later as
  their own block types (`mossy_stone` = its own 16 rows), with the atlas extended downward (256×512).
  For fill variants, horizontal flipping in code is enough for now.
- **"Transparent background":** every cell is a full block, so there's no transparency in this atlas.
