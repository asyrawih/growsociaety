# 02 · World

## Knowledge

### Size

| Constant | Value | Label |
|---|---|---|
| `WORLD_WIDTH` | 100 tiles | Rule |
| `WORLD_HEIGHT` | 50 tiles | Rule (open decision D1: the genre reference uses 60) |
| `TILE_SIZE` | 32 px | Rule (art is drawn for it) |

Coordinates: tile `(0, 0)` is the **bottom-left**, y goes up. This is already what `game_shared::tile_to_world` does.
A world is 3200 × 1600 px.

### Vertical layout

Rows are counted from the bottom. All boundaries are constants so D1 can change the height without touching logic.

| Rows (H = 50) | Constant | Contents |
|---|---|---|
| 0–4 | `BEDROCK_ROWS = 5` | Bedrock foreground, cave background. Unbreakable floor |
| 5–8 | `LAVA_ROWS = 4` (bottom of the ground band) | Ground, with single lava tiles scattered in |
| 5–29 | `GROUND_ROWS = 25` | Dirt with stone scattered in, cave background behind every tile |
| 29 | `SURFACE_ROW = BEDROCK_ROWS + GROUND_ROWS - 1` | Top ground row. Dirt with no background, drawn as grass |
| 30–49 | sky | Empty foreground and background |

Reference proportions (Reference): 60 rows = 6 bedrock + 30 ground + 24 sky, lava only in the 4 lowest ground rows.
Ours keeps the same ratios at 50.

### Generation rules

Generation is a pure function `generate(name) -> World`. Same name, same world, on any machine.

1. Seed = FNV-1a 64-bit hash of the upper-cased world name.
2. Fill bedrock rows with bedrock fg + cave bg.
3. Fill ground rows with dirt fg + cave bg. The surface row has **no** background, so it reads as open sky and
   draws as grass (see "Grass is a look" below).
4. Stone: for each ground tile below the surface row, replace dirt with stone with probability rising with depth:
   `p = STONE_MIN + (STONE_MAX - STONE_MIN) * depth_fraction` (Tuning: 0.03 at the top, 0.20 at the bottom).
5. Sand: in the top third of the ground band, below the surface row, replace dirt with sand with probability
   `SAND_CHANCE` (Tuning: 0.04). Sand is a base item for recipes, so a world must contain some.
6. Lava: in the lava rows only, replace with lava with probability `LAVA_CHANCE` (Tuning: 0.10).
7. Main door: pick `x` uniformly in `[2, WORLD_WIDTH - 3]`. Put the door at `(x, SURFACE_ROW + 1)` and force the tile
   under it, `(x, SURFACE_ROW)`, to bedrock so the spawn can never be dug out from below (Reference).
8. No caves, ores or trees at generation. The world starts plain; players make it interesting.

**The roll order is part of the world format.** Each step is one row-major pass that rolls exactly once per tile,
whether or not the tile is replaced (stone, then sand, then lava, then the door). Changing the order, the number of
rolls or the RNG changes every existing world, so once worlds are persisted this needs a generator version number
stored with the world.

### Grass is a look, not an item

There is no grass item. A **dirt** tile is drawn with the grass terrain when the tile above it has no solid
foreground **and** the dirt tile itself has no background (it is open to the sky). Underground dirt always has a
cave wall behind it, so digging a tunnel never grows grass on the tunnel floor. Dirt placed in the open air does
get a grass top, which is the natural result. This is a client drawing rule only: the rules, the inventory and the
network see dirt.

### The main door

- It is the spawn point: players entering the world appear on it.
- It is the exit: interacting with it leaves the world.
- It and the bedrock under it are unbreakable and cannot be covered (placing on the door tile is rejected).
- Respawn (after a hazard) returns the player to it.

### World identity

| Rule | Value | Label |
|---|---|---|
| Name length | 1–24 characters | Reference |
| Allowed characters | ASCII letters and digits, stored upper-case | Reference |
| Blocked names | A deny-list (staff words, profanity) checked on the server | Rule |
| Entering an unused name | Generates and persists a new world | Rule |
| Players per world | `MAX_PLAYERS_PER_WORLD = 64` | Tuning (reference: 65–100 by server load) |

Name validation lives in `game_shared` (`validate_world_name`) so client and server agree, but only the server's
answer counts.

## Technical

### Tile storage

```rust
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Tile {
    pub fg: ItemId, // 0 = empty
    pub bg: ItemId, // 0 = empty
}

pub struct World {
    pub name: String,
    tiles: Vec<Tile>,                   // WORLD_WIDTH * WORLD_HEIGHT, row-major, row 0 = bottom
    pub extras: HashMap<u32, TileExtra>, // by tile index: tree, claim, sign text, door target
    pub drops: Vec<Drop>,               // items lying in the world
    pub door: (u32, u32),
}

pub enum TileExtra {
    Tree(Tree),   // see 06
    Claim(Claim), // see 07
}
```

- Index = `y * WORLD_WIDTH + x`. Expose `get(x, y) -> Option<Tile>` and `set`; keep `tiles` private so bounds are
  checked in one place.
- `ItemId` is a `u16` newtype. A tile is 4 bytes, a world is 20 KB. That is why there are **no storage chunks**
  (D5): a whole world is smaller than one texture. Add chunking only if a profile shows a need.
- Per-tile state that most tiles do not have (trees, claims) goes in `extras`, not in `Tile`. Do not grow `Tile`.
- Break progress is transient and lives outside `World` persistence (see [05](05-punch-and-build.md)).

### Deterministic RNG

`game_shared` gets a tiny PRNG (SplitMix64, about 10 lines) instead of a `rand` dependency. Reasons: the crate stays
dependency-free, and world generation must give identical results on client and server forever. A library upgrade
that changes its stream would silently change every world.

```rust
pub struct Rng(u64);
impl Rng {
    pub fn next_u64(&mut self) -> u64 { /* splitmix64 */ }
    pub fn chance(&mut self, p: f32) -> bool
    pub fn range(&mut self, lo: u32, hi_inclusive: u32) -> u32
}
```

### Rendering (client)

- Terrain materials (grass, dirt, stone, sand) come from `assets/tiles/terrain.png`: `index = terrain * 16 + mask`,
  cell = `(index % 8, index / 8)`, 32×32 each. `mask` bits: 1 = up, 2 = right, 4 = down, 8 = left, set when that
  neighbour is the **same** material. Full rules: [`../art/tileset-terrain.md`](../art/tileset-terrain.md).
- Other blocks are single 32×32 sprites in `assets/blocks/`.
- Z order, back to front: parallax, background tiles, trees/objects, foreground tiles, dropped items, players, UI.
- When a tile changes, re-evaluate the mask of that tile **and its four neighbours**. Never rebuild the whole map.
- Start with one sprite entity per non-empty tile (at most 10,000 with both layers; Bevy batches these). Move to
  chunk meshes or Bevy's built-in tilemap chunk only if the frame time says so. Check the Bevy 0.20 docs for the
  current sprite/atlas API before writing; do not rely on older tutorials.

### Tests that must exist

- `generate` is deterministic: same name twice gives equal worlds; two names give different door positions (for a
  fixed pair chosen in the test).
- Layout: bedrock rows are all bedrock, sky rows are all empty, lava only appears in lava rows.
- The tile under the door is bedrock and the door tile is the door item.
- `get` returns `None` out of bounds.

## Foliage (decoration)

Owner request 2026-11-13 (relayed by the asset session). Art: `assets/foliage/` (48 sprites; list and anchoring in
`assets/README.md`, section "Foliage"; preview `assets/foliage/_preview.png`).

### Knowledge

Foliage is **decoration only**: no collision, cannot be punched, never dropped, not in any inventory. It makes the
plain generated world look alive without adding a single rule.

**It is not world state.** Every client computes the same foliage from the world's name and its current tiles, with a
deterministic hash. Nothing is generated, stored, persisted or sent over the network, and the server knows nothing
about it. When a tile changes, the foliage around it is recomputed, so digging away the grass removes the flower on
it and building a roof over a tree removes the tree.

### Rules (all chances Tuning)

A tile can hold foliage only if its foreground is empty, it is not the main door tile, and no tree or seed is on it.
`h = hash(world name, x, y)` decides, so the same world always looks the same.

| Ground (the tile below) | Chance | Sprites |
|---|---|---|
| Dirt drawn as grass (open to the sky) | `FOLIAGE_TREE = 0.06` a big plant, if it has room | 64×64: tree_pine, tree_birch, tree_willow, tree_maple_autumn, tree_apple, tree_dead, tree_mushroom, bush_large, hedge_flowers, boulder_ferns, log_hollow, bamboo, reeds_tall, crystal_plant |
| | otherwise `FOLIAGE_SMALL = 0.25` a small plant | 32×32: grass_short, grass_tall, grass_seeds, clover, dandelions, tulips, bluebells, daisies, poppies, lavender, pink_flowers, sunflower, mushrooms_brown, toadstool, fern_sprout, bush_round, berry_bush, bramble, reeds, rock_mossy, pebbles, stump, log_short, ivy, pumpkin |
| Sand open to the sky | `FOLIAGE_PALM = 0.04` palm if it has room, otherwise `FOLIAGE_DESERT = 0.15` | tree_palm; cactus, desert_shrub |
| Underground (the tile has a cave-wall background) on any solid block | `FOLIAGE_CAVE = 0.08` | mushrooms_glow, mushrooms_brown, pebbles |
| **Hanging**: the tile *above* is solid and the tile is empty with no solid block below | `FOLIAGE_HANGING = 0.20` | vines_hanging, moss_hanging (anchored to the top edge) |

"Room" for a 64×64 plant (2×2 tiles, centred on its tile, base on the ground row): the three tiles of its row
(x−1, x, x+1) and the three above them are free of solid foreground, and no other big plant is within 2 columns
(evaluated left to right, so the result is deterministic). Not used yet: grass_snowy, tree_pine_snowy (no snow biome),
lily_pads (no surface water).

### Technical

- A pure function in the client, `foliage_at(world: &World, x, y) -> Option<Foliage>`, plus the hash (FNV-1a over the
  world name, then the coordinates, finished with SplitMix64). No dependency on `game_shared` internals beyond `World`
  and the item table; no changes to `game_shared`, the protocol or the server.
- Drawn behind trees, objects, foreground tiles, drops and players, in front of background walls.
- On `TileChanged` at `(x, y)`, recompute foliage for the tiles within `RECOMPUTE_COLUMNS = 3` columns and 1 row of
  it. Three, not two: a big plant is suppressed when either of the two columns to its left is also a candidate, so
  a tile's result depends on tiles from 3 columns left to 1 column right.
- Room means **no foreground at all** in the 3×2 area (not just no solid), so a big plant never covers the door or a
  sapling.
- Placing a block on a foliage tile is allowed and simply hides the foliage; the placement rules ignore foliage.

### Tests that must exist

- The same world name gives the same foliage on every call; two names differ.
- No foliage on a tile with foreground, on the door tile, or on a tree/seed tile.
- Grass, sand, cave and hanging rules each pick only from their own sprite list.
- A big plant appears only with room, and never two within 2 columns.
- After a block is placed above a big plant's base, it is gone; after the grass under a small plant is dug, it is gone.
- Over a generated world, the share of surface tiles with foliage is roughly the configured chances (wide tolerance).
