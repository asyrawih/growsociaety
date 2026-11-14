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

## UI (`assets/ui/`)

Wood-and-parchment kit from PixelLab (template `bbe2b58c-499b-4f00-93f1-3efc79e80878`). Reference layout: `ui/_mockup.png`.
Use nearest-neighbour sampling. 9-slice insets (px, every side unless noted) keep the corners crisp when stretched:

| File | Size | 9-slice | Use |
|---|---|---|---|
| `panel.png` | 139×125 | 18 | Window background: inventory, wardrobe, store |
| `slot.png` | 37×37 | 8 | Item slot (inventory grid, hotbar) |
| `slot_selected.png` | 37×37 | 8 | Selected slot (gold rim) |
| `slot_worn.png` | 37×37 | 8 | Wardrobe slot holding an item (green check badge, bottom-right) |
| `slot_equip_empty.png` | 37×37 | 8 | Empty wardrobe slot; draw `slot_icon_<slot>.png` on top |
| `slot_icon_{hair,hat,face,shirt,pants,shoes,back,hand}.png` | 32×32 | – | Grey silhouette for an empty slot, names = `Slot` enum |
| `preview_frame.png` | 94×97 | 20 | Wardrobe character preview. Stretch it to **at least 136×150** for the rig at 2× (96 px plus room for wings and big hair), centre the rig and put its feet ~10 px above the inner bottom edge |
| `tab.png` | 51×23 | 8 | Tab button (e.g. Inventory / Wardrobe, store categories) |
| `close.png` | 27×27 | – | Close button, top-right corner of a panel |
| `tooltip.png` | 48×32 | 6 | Item tooltip frame: slim dark-wood border on parchment, no ornate corners. Stays readable stretched to ~280×90 with 16 px dark-brown text (spec: docs/knowledge/ux/item-tooltip.md) |
| `bubble.png` | 32×24 | 6 | Chat speech bubble (spec: docs/knowledge/ux/chat-bubbles.md). Parchment fill, dark 1 px outline, rounder corners than the tooltip so it reads as speech, soft bottom shade |
| `bubble_tail.png` | 10×6 | – | Tail pointing down, centred under the bubble. Its top row is fill, so draw it **1 px up** (overlapping the bubble's bottom outline row) and the tail opens into the box with no seam |
| `hotbar.png` | 138×47 | – | Has 5 cells baked in, so don't stretch it. Prefer a row of `slot.png` |

Wardrobe layout (see mockup): left column hair, hat, face, shirt; right column pants, shoes, back, hand; character
preview in the middle. **Equipment never appears in the hotbar**: the hotbar lists non-equipment items only. Clothes
are equipped from the wardrobe/inventory.

Overhead player name: plain text with a 1 px dark outline, no background. Its colour comes from the player's level
(Growtopia-style tiers, white until levels exist).

Every equipment item has `items/<name>/icon.png` (32×32) for slots. Pickaxe icons are hand-made; the rest come from
`docs/art/item_icons.py`, which stacks the item's own layers. Run it again after adding items.

## Font (`assets/fonts/`)

`growsociaety_pixel.ttf`: a pixel font made in PixelLab (job `397c882a-b5f9-4d47-9864-88def5302ace`). Glyphs are drawn
on a 16 px grid, so use font sizes that are multiples of 16 (16, 32) to keep pixels crisp; other sizes blur.
Preview: `fonts/_preview.png`. It covers A-Z a-z 0-9, space and `. , ! ? : ; ' " - + ( ) / % * = _ $`, plus
hand-drawn `# · | @ [ ] < > & ×` added on the same grid by `docs/art/font_add_glyphs.py`. PixelLab's original is
kept untouched in `fonts/source/`, and the script rebuilds the game font from it.
**Still missing:** `{ } ~ ^ \` and backtick, plus accented letters. To add more, draw them in that script.

### Voice chat icons (`ui/voice_*.png`, `ui/settings_gear.png`), 32×32

`voice_mic_on`, `voice_mic_muted`, `voice_mic_talking` (green, while transmitting), `voice_speaker_on`,
`voice_speaker_muted`, `voice_speaking` (bubble above a talking player's head), `voice_level_bars` (mic level),
`voice_key_v` (push-to-talk hint), `voice_headphones`, `settings_gear` (voice settings menu).

### Lobby (`ui/lobby/`)

Login and register screen. Reference layout: `ui/lobby/_mockup.png`.
- `background.png` (480×270, scale ×4): parallax layers and a terrain strip with trees, made from existing assets.
- `input.png` (241×37, 9-slice 8): text field. Draw the text inside the dark area.
- `button_primary.png` (green, Login/Register) and `button_secondary.png` (brown, Register/Back), 151×45,
  9-slice 10, each with `_hover` and `_pressed` versions.
- `checkbox_off.png` / `checkbox_on.png` (29×29) for "Remember me". `eye.png` (+ `_hover`/`_pressed`) toggles
  showing the password.
- Panel = `ui/panel.png`. Title in the pixel font at 32 px.

## Flags (`assets/flags/`)

247 country flags, about 16×11, named by ISO 3166-1 alpha-2 code (`id.png`, `jp.png`, …), plus `england`,
`scotland`, `wales`, `catalonia`, `europeanunion`. Drawn left of the overhead name, vertically centred on the text.
Source: famfamfam flag icons, **public domain** (`gg.png` left out because it needs attribution). A few flags are not
16×11 (`np` 9×11, `ch`/`va` 11×11), so centre them in a 16×11 box.

## Foliage (`assets/foliage/`)

Decoration only: no collision, can't be punched. Preview: `foliage/_preview.png`.
Every sprite's base touches its **bottom edge**, so place it with the bottom on top of a solid block. Two exceptions,
`vines_hanging` and `moss_hanging`, touch the **top edge** and hang under a block.
- 32×32 (one tile): grass_short, grass_tall, grass_seeds, clover, dandelions, tulips, bluebells, daisies, poppies,
  lavender, pink_flowers, sunflower, mushrooms_brown, mushrooms_glow, toadstool, fern_sprout, bush_round, berry_bush,
  bramble, reeds, lily_pads (on water), rock_mossy, pebbles, stump, log_short, vines_hanging, moss_hanging, ivy,
  cactus and desert_shrub (sand), grass_snowy, pumpkin.
- 64×64 (2×2 tiles, centred on its tile): tree_pine, tree_birch, tree_willow, tree_maple_autumn, tree_palm (sand),
  tree_dead, tree_apple, tree_mushroom, bush_large, hedge_flowers, reeds_tall, boulder_ferns, log_hollow, bamboo,
  tree_pine_snowy, crystal_plant.
Suggested world-gen use: on surface grass tiles, each tile rolls for a small plant (~25%) or, with room, a tree
(~6%). Sand gets cactus/desert_shrub/tree_palm. Undersides of floating blocks get the hanging pieces.

## Audio (`assets/audio/`)

Made with ElevenLabs (flow `2urSGR1SMmQCjMh7gbr8`). Files are **OGG Vorbis**, which Bevy plays with its default features.
They were converted from the MP3 originals with ffmpeg's built-in `vorbis` encoder (this Homebrew ffmpeg has no libvorbis).
- `bgm/world.ogg`: 120 s cozy village loop (acoustic guitar, flute, marimba) for in-world play. Loop it. Both ends
  are at the same level with a 15 ms fade, so the loop point doesn't click.
- `bgm/lobby.ogg`: 60 s title theme (strings, guitar, glockenspiel) for the login/register lobby. It ends on a
  fade-out, so looping sounds like the song restarting.
- `sfx/`, 0.5–2 s, dry (no reverb): `punch_dirt`, `block_place`, `gem` (collect), `land`, `footstep_grass`,
  `harvest`, `door` (enter), `equip`, `purchase`, `chat` (incoming message), `level_up`, `punch_stone`, `block_break`,
  `denied` (action refused), `item_pickup`, `jump`, `ui_click`, `panel_open`, `plant` (seed planted).

### Top bar and settings (`ui/settings/`)

Reference: `ui/settings/_mockup.png`. PixelLab kit `cbfdcc96-9393-4bbc-bf52-0131bb7249d7`, icons `a74d9866-9437-4a32-bfeb-6b372a9c4dc0`.
- `topbar.png` (242×35, 9-slice 10): strip in the **top-right corner** holding round icon buttons, from left: shop,
  inventory, wardrobe, chat, voice mic (`ui/voice_mic_on`/`_muted`), settings (`ui/settings_gear`).
- `icon_button.png` (41×41, round, empty face): draw a 32×32 icon centred on top.
- Icons, 32×32: `icon_shop`, `icon_inventory`, `icon_wardrobe`, `icon_chat`, `icon_friends`, `icon_world`,
  `icon_exit`, `icon_help`, `icon_home`, `icon_notification`. Settings tabs use `icon_audio`, `icon_voice`,
  `icon_display`, `icon_controls`, `icon_account` and `icon_language`.
- Controls: `slider_track.png` (151×16, 9-slice 7) + `slider_knob.png` (25×25); `toggle_off.png` / `toggle_on.png`
  (57×29); `dropdown.png` (100×29, 9-slice 10, arrow on the right); `divider.png` (246×17, 9-slice left/right 8).
- The settings panel is `ui/panel.png` with tabs (`ui/tab.png`): Audio (music / SFX / voice volume sliders),
  Voice (push-to-talk V and open mic toggles, mic dropdown), Display, Controls, Account (skin tone, language).

## Game icon (`assets/icon/`)

A grass block with a sprout (PixelLab `dba1d15a-f247-47fe-b7eb-0d76d2838d7e`, candidate 1).
- `icon_{16,32,48,64,128,256,512}.png`. 128 and up are whole-number nearest upscales; 16–64 are smooth downscales.
- `icon.ico` (Windows exe and window icon) and `icon.icns` (macOS .app bundle).
- Bevy window icon: load `icon_64.png` (or 32) into a winit `Icon` on the primary window at startup.

## Crafting and mods

PixelLab object `3603a442-5b78-4601-902b-07c57f56cd41` (frames 0, 3, 4, 6–11); sound from ElevenLabs.
- `blocks/67_workbench.png` (32×32): the Workbench crafting station as a world tile, also used as its item icon. The
  larger `objects/workbench.png` (64×64) stays as decoration.
- `ui/icon_craft.png` (32×32): hammer and saw, for a "Craft" tab or button.
- Craft panel: recipe slots reuse `ui/slot.png` / `ui/slot_selected.png`. `ui/craft/arrow.png` (32×32, orange →)
  goes between the ingredients and the result. The Craft button is `ui/lobby/button_primary.png` (+ `_hover`,
  `_pressed`), with `ui/craft/button_craft_disabled.png` (151×45, grey) when ingredients are missing. 9-slice 10
  for all of them.
- Mod icons for tooltips, `ui/mods/mod_<name>.png` (32×32): `extra_jump`, `glide`, `run_boost`, `reach`,
  `dig_boost`, `gem_finder`.
- `audio/sfx/craft.ogg` (0.8 s): item crafted, a wood knock and metal clink.
- `audio/sfx/splice.ogg` (0.8 s): two seeds spliced into a new tree, a leafy shimmer and chime (PixelLab sound
  `4cc3ba80-c22b-43d4-ade3-ab7c6716607d`). Meant to feel more special than `plant.ogg`.

### One-way platforms (`blocks/68_wood_platform.png`, `blocks/69_stone_platform.png`)

32×32. Only the top 8 px are drawn: a plank (palette of `12_planks`) or a stone slab (palette of `14_stone_bricks`),
with a 1 px lighter highlight on the walkable top edge. Two small brackets hang under the left and right ends, and
everything else is transparent, so they never read as solid blocks. Adjacent tiles join into one continuous ledge.
Made from the existing block pixels (no generation). Spec: docs/knowledge/ux/platforms.md. Item ids:
`WOOD_PLATFORM` = 68, `STONE_PLATFORM` = 69.

### Respawn (spec: docs/knowledge/ux/respawn.md)

Drawn by `docs/art/make_respawn_fx.py` (no generation; rerun it to rebuild).
- `ui/respawn_ring.png`: the hold-to-go-home ring above the player's head. 8 frames of 24×24 in one strip (192×24),
  filling clockwise from 12 o'clock (frame k = (k+1)/8). Warm gold with a dark 1 px outline; the unfilled part is a
  faint dark groove. Readable on sky and on cave walls. Pick the frame from hold progress (0–1 s).
- `fx/respawn_puff.png` + `fx/respawn_puff.json`: a dust and sparkle puff where the player vanishes and where they
  appear at the door. 6 frames of 32×32 (192×32 strip), 15 fps, no loop, pivot at the centre. Neutral white/grey,
  so it works on any background. Same pixelfx/Aseprite sidecar format as `rock-break-coins.json`.
- `fx/ready_sparkle.png` + `fx/ready_sparkle.json` (spec: docs/knowledge/ux/farming-feedback.md): a "ready to
  harvest" glint on the item icon above a grown tree. 4 frames of 16×16 (64×16 strip), 10 fps, no loop, centre
  pivot. A warm white/gold 4-point star that grows, flashes (short diagonal rays) and fades, with a dark 1 px
  outline. Kept small so a farm playing it every 4 s per tree doesn't look noisy. Made by the same
  `docs/art/make_respawn_fx.py`.

### Farming, punch and inventory feedback (specs: docs/knowledge/ux/vfx-farming-punch.md, inventory-categories.md)

Drawn by `docs/art/make_respawn_fx.py` (no generation). The fx strips use 12 fps, no loop, a centre pivot and the
same JSON sidecar as `respawn_puff`.
- `fx/block_break_dust.png` (+ `.json`): 6 × 32×32. Near-white dust plus grey chips bursting out with gravity.
  Value contrast only, so tint it with the broken block's colour at runtime.
- `fx/soil_puff.png` (+ `.json`): 5 × 32×32. A low, wide brown soil puff on the ground line (bottom of the frame),
  for a planted seed.
- `fx/splice_swirl.png` (+ `.json`): 8 × 32×32. Green leaves and violet sparks spiralling inward, ending in a small
  pop at the centre. It reads clearly unlike the soil puff.
- `fx/drop_glow.png` (+ `.json`): 1 × 16×16. A soft warm ring with a transparent centre, drawn under seed drops;
  the code pulses its alpha.
- `ui/chip.png` / `ui/chip_active.png`: 40×20 wood pill, 9-slice inset 6, inventory category filter chips styled
  like `tab.png`. The active one has a parchment fill.
- `ui/icon_search.png`: 16×16 magnifier (gold rim, cream glass, wood handle).
