# 06 · Farming and Splicing

## Knowledge

Farming is how items multiply and how new items are discovered. A block is not crafted at a bench; it is **grown**.

### The cycle

```
break block ──(chance)──> seed ──plant──> sapling ──wait grow time──> grown tree ──punch──> 2–5 blocks (+ maybe a seed)
```

### Planting

- A seed is planted on an **empty foreground tile directly above a tile with a solid foreground** (Reference).
- The sapling occupies the foreground of its tile. It has no collision: players walk through trees.
- If the block under a tree is removed, the tree stays (Rule, ours; avoids cascade logic).
- Planting needs edit permission on the tile, like any placement.

### Grow time

```
grow_seconds(rarity) = rarity³ + 30 × rarity
```

(Reference. Checked against two published data points: rarity 100 gives 1,003,000 s, rarity 999 gives 997,032,969 s.)

| Rarity | Grow time |
|---|---|
| 1 | 31 s |
| 2 | 68 s |
| 5 | 4 min 35 s |
| 10 | 21 min 40 s |
| 20 | 2 h 23 min |
| 50 | 1 d 11 h |
| 100 | 11 d 14 h |

The cubic term is the whole economy: doubling rarity multiplies the wait by about eight. Low items are spammable,
high items need days of protected land. The two coefficients are constants (`GROW_CUBIC = 1`, `GROW_LINEAR = 30`)
so the curve can be flattened without touching logic.

Trees grow on **wall-clock time**, including while nobody is in the world and while the server is off. A tree stores
when it was planted; "is it grown" is a comparison, not a ticking timer.

### Harvest

Punching a grown tree once harvests it:

| Result | Value | Label |
|---|---|---|
| Blocks of the tree's item | `fruit`, rolled **when planted**: uniform 2–5 | Tuning (reference: "at least 1") |
| A seed of the same item | `TREE_SEED_CHANCE = 0.20` | Tuning |
| Gems | See [07](07-economy-and-locks.md) | |
| The tree | Removed; the tile becomes empty | Rule |

Fruit is rolled at planting and stored, so re-entering the world or re-punching cannot re-roll it. Rarity does not
affect the fruit count (Reference).

Punching a sapling that is not grown damages it like a block (`SAPLING_HITS = 3`, Tuning) and destroys it with no
drops. This lets players clear mistakes and lets intruders wreck unprotected farms.

### Sustainability equation

A farmable item must return more than one seed per seed planted, or it goes extinct. With the defaults from
[05](05-punch-and-build.md) (`DROP_BLOCK = 0.40`, `DROP_SEED = 0.20`), a block that is placed and broken until it is
gone yields on average `DROP_SEED / (1 - DROP_BLOCK) = 0.333` seeds. So per seed planted:

```
seeds back = TREE_SEED_CHANCE + avg_fruit × DROP_SEED / (1 - DROP_BLOCK)
           = 0.20 + 3.5 × 0.333
           = 1.37
```

A farm grows about 37% per cycle if every block is ground back into seeds, and 0.20 (it shrinks) if every block is
kept for building. That tension is intended. **Whenever one of these four constants changes, recompute this number;
it must stay above 1.** There is a test for it.

### Splicing

Splicing makes the tree of a **new** item from two different seeds.

1. Plant seed A. A sapling of A appears.
2. Plant seed B on that sapling. If `(A, B)` is a recipe, the sapling becomes a sapling of the result item, the
   message "A and B spliced into C" is shown, and B is consumed.
3. If `(A, B)` is not a recipe, nothing changes, B is **not** consumed, and the player is told it cannot be spliced.
4. A tree that has already been spliced rejects a third seed (Reference).
5. Order does not matter: `(A, B)` and `(B, A)` are the same recipe.
6. Splicing resets the grow timer to now and uses the **result's** rarity (Rule, ours). It can be done at any time
   before harvest.

Result rarity = rarity(A) + rarity(B) (Rule). The item table must agree; a test enforces it.

### Starter recipes (ours; placeholders the owner can redesign)

Base items from a generated world: dirt (1), stone (1), sand (1), cave wall (1).

| A | B | Result | Rarity | Sprite |
|---|---|---|---|---|
| dirt | stone | gravel | 2 | `09_gravel` |
| stone | cave wall | cobblestone | 2 | `03_cobblestone` |
| sand | stone | glass | 2 | `16_glass` |
| dirt | cave wall | vine soil | 2 | `11_vine_soil` |
| dirt | sand | fern | 2 | `62_fern` |
| gravel | stone | stone bricks | 3 | `14_stone_bricks` |
| cobblestone | dirt | mossy stone | 3 | `10_mossy_stone` |
| glass | sand | ice | 3 | `07_ice` |
| fern | dirt | log end | 3 | `13_log_end` |
| log end | stone | planks | 4 | `12_planks` |
| stone bricks | cave wall | stone brick wall (bg) | 4 | `bg_stone_bricks` |
| planks | cave wall | wood wall (bg) | 5 | `bg_wood` |
| planks | gravel | crate | 6 | `17_crate` |
| planks | stone bricks | ladder | 7 | `33_ladder` |

Every result has art in `assets/blocks/` already. Add a recipe only together with its item row and its sprite.

## Technical

### Data

```rust
pub struct Tree {
    pub item: ItemId,     // the block this tree will give
    pub planted_at: f64,  // unix seconds, server clock
    pub fruit: u8,
    pub spliced: bool,
}
// stored in World.extras as TileExtra::Tree; the tile's fg holds the seed's ItemId

pub const RECIPES: &[(ItemId, ItemId, ItemId)] = &[ /* (a, b, result) */ ];

pub fn grow_seconds(rarity: u16) -> u64      // u64: 65535³ fits
pub fn is_grown(tree: &Tree, now: f64) -> bool
pub fn splice(a: ItemId, b: ItemId) -> Option<ItemId>   // tries both orders
```

- `now` is always passed in. `game_shared` never reads the system clock, which keeps it testable and keeps the
  server the only time authority. A client that changes its clock changes only what it draws.
- `RECIPES` is a linear scan. It will hold tens to a few hundred rows and is hit once per splice.
  `// ponytail: linear scan, switch to a HashMap if recipes pass a few thousand`.

### Actions

No new action types. `Place` with a seed item covers both planting and splicing, and `Punch` on a tree tile covers
harvest and sapling damage. `apply` branches on what is in the tile. New events:

```rust
Event::TreePlanted { x, y, tree }
Event::TreeSpliced { x, y, tree, a: ItemId, b: ItemId }
Event::TreeHarvested { x, y }          // followed by ItemDropped events
```

New reject: `Reject::NoRecipe`, `Reject::AlreadySpliced`.

### Rendering (client)

- Growth uses the six stage sprites `assets/blocks/52_crop_1.png` … `57_crop_6.png`:
  `stage = if grown { 5 } else { floor(progress × 5) }` where `progress = elapsed / grow_seconds`.
- A grown tree shows a small icon of its item on top so players can tell trees apart.
- The client recomputes the stage from `planted_at` about once a second. The server does not send growth updates.

### Tests that must exist

- `grow_seconds(1) == 31`, `grow_seconds(100) == 1_003_000`, `grow_seconds(999) == 997_032_969`.
- A tree is not grown one second before its grow time and is grown at it.
- For every recipe: both parents and the result exist, `rarity(result) == rarity(a) + rarity(b)`, `a != b`, and no
  unordered pair appears twice.
- Splice succeeds in both orders, a non-recipe returns `NoRecipe` without consuming the seed, a third seed returns
  `AlreadySpliced`.
- Planting on a tile with no solid block below is rejected.
- Harvest yields exactly the stored `fruit` and removes the tree; harvesting before growth yields nothing.
- The sustainability equation evaluates above 1 with the current constants.
