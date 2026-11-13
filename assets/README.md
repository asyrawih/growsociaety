# Assets (PixelLab, style: `docs/art/STYLE.md`)

| Path | Contents |
|---|---|
| `tiles/terrain.png` | 256×256 autotile atlas, `index = terrain*16 + mask` (see `docs/art/tileset-terrain.md`). Terrain: 0 grass, 1 dirt, 2 stone, 3 sand |
| `tiles/*_raw.png` + `*_meta.json` | PixelLab corner-Wang source. Rebuild: `uv run --with pillow python docs/art/wang_to_mask.py assets/tiles/terrain.png grass dirt stone sand` |
| `blocks/NN_name.png` | 32×32 blocks/items. `ore_*_overlay` is drawn on top of a stone block. `bg_*` = background walls (darkened copies of their block). `64_wrench`, `65_door` (1-tile door). `66_seed_gray` = the one seed sprite for every block: greyscale with a white highlight, so tint it with the block's colour (sprite colour multiply) |
| `objects/*.png` | 64×64 large objects (door, chest, forge, trees, portal, …), sitting on the floor line |
| `player/player_east.png` | 48×48 cells, facing east (mirror for west). Rows: idle 4, walk 6, jump 9, punch 3 frames. Feet at y=44 |
| `player/player_east_break.png` | Break (pickaxe swing), 7 frames of **68×68** (room for the pickaxe). Feet at y=57: draw at offset (-10, -13) from the 48×48 cell |
| `parallax/layer1..4.png` | 480×270, scale ×4. 1 = sky (opaque), 2 mountains, 3 hills, 4 treeline (transparent top). Scroll slower the lower the number |

## Player rig (Growtopia-style, animated in code)

`player_rig/<part>.png` are full 48×48 layers (facing east). Draw in `rig.json` `order`, rotating each part around
its `pivots` entry: walk = legs ±14°, arms opposite; punch = `arm_front` 0→95°. Skin is exactly the 5 colours of
`skin_ramps.light`; swap them for another ramp (`tan`, `brown`, `dark`, `pale`, or any 5 hex colours) at load.

`items/<name>/<part>.png` are drawn right on top of the matching body part, with the same pivot. An item only has
the parts it touches. Slot = name prefix: `hair_*`, `hat_*`, `face_*`, `shirt_*`, `pants_*`, `shoes_*`.
Layer order inside one body part: body, then pants, shoes, shirt, hair, face, hat (hat last so it covers hair).

Two slots that are not body parts:
- `wings_*/back.png`: a 48×48 layer drawn **behind everything** (before `arm_back`), pivot = torso. It stays still or
  gets a small flap rotation.
- `pickaxe_*/icon.png`: a 32×32 sprite, used both as the inventory icon and as the item held in the hand. The handle end is
  at about (3, 29) in the icon, with the head at top-right. Put that point on the front hand at about (22, 36) in the
  48×48 rig, as a child of `arm_front` so it swings with punch/break. Draw it after `arm_front`.

123 items: hair ×26, hat ×5, face ×1, shirt ×26, pants ×23, shoes ×3, wings ×19, pickaxe ×20.
Names are the folder names. Every PixelLab job id is in `items/pixellab_ids.txt`.

**20 items for 10 generations (preferred):** `docs/art/item_sheet.py build <head|shirt|pants|shoes> <dir>` makes a 5×4
sheet of the base plus a mask for that slot. One PixelLab `inpaint_image` on it ("give EACH character a DIFFERENT
<item>: <20 options>") fills every cell, and pixels outside the mask stay identical. Then run
`item_sheet.py split <result.png> <prefix> <20 names>`. The first 20 hair items (`hair_spiky_orange` … `hair_fluffy_white`)
were made this way.

Single item (about 10 generations each, the older way):
1. PixelLab `create_character_state` on the base `d2e7354b-4000-4996-b948-260da736fbf9`:
   "add/wearing <item>; change nothing else". Download its `east` rotation to `items/<name>/source_east.png`.
2. `uv run --with pillow python docs/art/item_extract.py assets/player_rig/source/base_east.png assets/items/<name>/source_east.png assets/items/<name>`

Rebuild the body: `uv run --with pillow python docs/art/rig_split.py assets/player_rig/source/base_east.png assets/player_rig`

PixelLab tileset ids: grass `f2ae29cf-6aa7-4af8-8b89-9732243c3449`, dirt `d4357c74-d336-4234-ac31-5c88b3f76686`,
stone `b7d0c1de-e98b-486a-aed7-a0c50949b06d`, sand `41917343-5351-4341-a94d-e1bf93e2842b`.
Player character id: `b654b0ae-2ad5-48d1-ad8c-3aebed0604b0`.
