# 18 · Crafting and Item Mods

Owner-approved 2026-11-14 (relayed by the coordinating session), from the takeaways in
[17](17-growtopia-reference.md): movement and work abilities come from **what you wear, by slot**, and a crafting
station turns farmed blocks into gear. All names here are ours.

The tables are written to be pasted into `game_shared` as they are. All numbers are Tuning unless marked.

## Knowledge

### Mods

A **mod** is an ability an equipment item gives while it is worn. Mods do not stack across items of the same slot
(one item per slot anyway), and each slot only carries the kinds of mod listed for it:

| Slot | Mods it can carry | Why |
|---|---|---|
| `Back` (wings) | `ExtraJump`, `Glide` | Air movement |
| `Shoes` | `RunBoost` | Ground movement |
| `Hand` (pickaxes) | `DigBoost`, `Reach` | Working blocks |
| `Face` | `GemFinder` | Luck |
| Hair, Hat, Shirt, Pants | none | Cosmetic |

| Mod | Effect | Numbers |
|---|---|---|
| `ExtraJump` | One more jump while airborne, refilled on landing | Impulse `EXTRA_JUMP_VELOCITY = 15.0 tiles/s` (the ground jump is 17.5). Ground jump + extra jump reaches about 4 tiles, so wings clear the 3-tile walls that block everyone else (deliberate) |
| `Glide` | Holding jump while falling caps the fall speed | `GLIDE_FALL_SPEED = 3.0 tiles/s` (normal cap 24). Horizontal speed unchanged. Releasing jump falls normally |
| `RunBoost(pct)` | Walk speed + pct % | `pct` 10 or 20: 6.6 or 7.2 tiles/s (normal 6.0). Still far below one tile per tick, so the tunnelling rule holds |
| `DigBoost(points)` | Damage per punch | See the pickaxe ladder below |
| `Reach(n)` | Punch and build reach + n tiles | `n = 1` only (reach 3 instead of 2) |
| `GemFinder(n)` | Every gem roll's upper bound + n | `n = 1`: a rarity-1 block rolls `0..=2` instead of `0..=1` |

### The pickaxe ladder, capped at 2× the fist

Damage moves to **quarter points** so tiers can differ by less than a whole hit:

```
block health = item.hits × DAMAGE_PER_HIT      (DAMAGE_PER_HIT = 4)
damage per punch = FIST_DAMAGE (4) or the worn pickaxe's DigBoost points
```

| Tier | DigBoost points | × fist | Stone (6 hits = 24) breaks in | Pickaxes |
|---|---|---|---|---|
| fist | 4 | 1.0 | 6 punches | — |
| 1 | 4 | 1.0 | 6 | wood, rusty, bone, candy (cosmetic choice) |
| 2 | 5 | 1.25 | 5 | stone, copper, iron, ice, nature |
| 3 | 6 | 1.5 | 4 | gold, emerald, ruby, fire, lightning, shadow |
| 4 | 8 | 2.0 | 3 | diamond, obsidian, drill, rainbow, royal |

This replaces today's `power` 1–4 (up to 4× the fist), which made the best pickaxe break stone in 2 punches and
flattened the value of block hardness. Store prices per tier stay as they are.
`TileDamaged.damage` reports quarter points from now on (the client's crack overlay divides by `hits × 4`).

### Which existing items get which mod (folder names = equipment ids)

| Item | Mod |
|---|---|
| `wings_angel_white`, `wings_bat_black`, `wings_cosmic`, `wings_demon_red`, `wings_dragon_blue`, `wings_eagle_brown`, `wings_feather_gold`, `wings_flame_red`, `wings_raven_black`, `wings_steel` | `ExtraJump` |
| `wings_bone`, `wings_butterfly_orange`, `wings_cloud_white`, `wings_crystal_purple`, `wings_dragonfly_cyan`, `wings_fairy_pink`, `wings_frost_blue`, `wings_leaf_green`, `wings_rainbow` | `Glide` |
| `shoes_sneakers_red` | `RunBoost(20)` |
| `shoes_boots_iron` | `RunBoost(10)` |
| `shoes_boots_brown` (starter) | none |
| every pickaxe | `DigBoost(points of its tier)` |
| `pickaxe_drill` | also `Reach(1)` |
| `face_goggles` | `GemFinder(1)` |
| everything else | none |

An item carries at most two mods (only `pickaxe_drill` has two).

### Crafting

- **The station is the Workbench**, a placeable block. It is solid, `hits = 6`, `NO_SEED`, rarity 0, and needs edit
  permission to place or break like any block. Art: `assets/blocks/67_workbench.png` (32×32, exists); id **67** follows
  the file-prefix rule in [03](03-items-and-tiles.md).
- **Using it:** with the wrench selected, click a Workbench within reach to open the craft panel. Crafting does not
  change the tile, so anyone in reach may use any Workbench, including one on someone else's claim.
- **A recipe** = a station (or none) + up to 4 inputs `(item, count)` + one output `(item, count)`.
- **Crafting** is one atomic action: the server checks the station is within reach (if the recipe needs one), every
  input is held in full, and the output fits the inventory (200 cap, free slot); then it removes the inputs and adds
  the output. Nothing changes on a refusal.
- **Bootstrapping:** the Workbench recipe itself needs no station, so a player can craft their first one anywhere.
- Crafting gives **no XP** (doc 14: no XP from converting items, or it becomes an XP farm). Crafted equipment is not
  worn automatically.

### Starter recipes

All inputs are items that exist today (farmed blocks, store pickaxes). Higher tiers consume the previous pickaxe, so
the ladder is a path, not a pile.

| # | Output | Inputs | Station |
|---|---|---|---|
| 0 | 1 Workbench | 10 planks, 4 log end | none |
| 1 | 2 crate | 8 planks, 2 log end | Workbench |
| 2 | 4 ladder | 6 planks | Workbench |
| 3 | 1 pickaxe_wood | 8 planks, 4 log end | Workbench |
| 4 | 1 pickaxe_stone | 1 pickaxe_wood, 20 stone, 10 cobblestone | Workbench |
| 5 | 1 pickaxe_copper | 1 pickaxe_wood, 20 gravel, 10 stone bricks | Workbench |
| 6 | 1 pickaxe_iron | 1 pickaxe_stone, 30 stone bricks, 20 mossy stone | Workbench |
| 7 | 1 pickaxe_gold | 1 pickaxe_iron, 50 sand, 30 glass | Workbench |
| 8 | 1 pickaxe_emerald | 1 pickaxe_iron, 40 vine soil, 40 mossy stone | Workbench |
| 9 | 1 pickaxe_diamond | 1 pickaxe_gold, 100 ice, 60 glass | Workbench |
| 10 | 1 pickaxe_drill | 1 pickaxe_gold, 80 stone bricks, 40 crate | Workbench |
| 11 | 1 shoes_boots_iron | 40 stone bricks, 10 crate | Workbench |
| 12 | 1 shoes_sneakers_red | 30 vine soil, 20 fern | Workbench |
| 13 | 1 face_goggles | 40 glass, 20 ice | Workbench |
| 14 | 1 wings_leaf_green | 100 fern, 50 vine soil | Workbench |
| 15 | 1 wings_feather_gold | 150 crate, 100 glass, 50 ice | Workbench |

Recipe ids are their position in the table and are sent over the network: **append only, never reorder.**

## Technical

### Data (paste-ready)

```rust
// items.rs
pub const WORKBENCH: ItemId = ItemId(67);
// ITEMS row: def(WORKBENCH, "workbench", Block, Solid, 0, 6, NO_SEED, Sprite::Block("67_workbench.png"))

// mods.rs (new)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]           // + serde cfg_attr
pub enum Mod { ExtraJump, Glide, RunBoost(u8), DigBoost(u8), Reach(u8), GemFinder(u8) }

pub const DAMAGE_PER_HIT: u16 = 4;
pub const FIST_DAMAGE: u16 = 4;
pub const EXTRA_JUMP_VELOCITY: f32 = 15.0 * TILE_SIZE;
pub const GLIDE_FALL_SPEED: f32 = 3.0 * TILE_SIZE;
pub const DIG_POINTS: [u16; 4] = [4, 5, 6, 8];          // pickaxe tiers 1..=4 (replaces PickaxeTier::power)

pub const ITEM_MODS: &[(&str, &[Mod])] = &[             // equipment folder name -> mods (pickaxes get DigBoost from their tier)
    ("wings_angel_white", &[Mod::ExtraJump]), ("wings_bat_black", &[Mod::ExtraJump]),
    ("wings_cosmic", &[Mod::ExtraJump]), ("wings_demon_red", &[Mod::ExtraJump]),
    ("wings_dragon_blue", &[Mod::ExtraJump]), ("wings_eagle_brown", &[Mod::ExtraJump]),
    ("wings_feather_gold", &[Mod::ExtraJump]), ("wings_flame_red", &[Mod::ExtraJump]),
    ("wings_raven_black", &[Mod::ExtraJump]), ("wings_steel", &[Mod::ExtraJump]),
    ("wings_bone", &[Mod::Glide]), ("wings_butterfly_orange", &[Mod::Glide]),
    ("wings_cloud_white", &[Mod::Glide]), ("wings_crystal_purple", &[Mod::Glide]),
    ("wings_dragonfly_cyan", &[Mod::Glide]), ("wings_fairy_pink", &[Mod::Glide]),
    ("wings_frost_blue", &[Mod::Glide]), ("wings_leaf_green", &[Mod::Glide]),
    ("wings_rainbow", &[Mod::Glide]),
    ("shoes_sneakers_red", &[Mod::RunBoost(20)]), ("shoes_boots_iron", &[Mod::RunBoost(10)]),
    ("pickaxe_drill", &[Mod::Reach(1)]),
    ("face_goggles", &[Mod::GemFinder(1)]),
];

pub fn mods_of(item: ItemId) -> impl Iterator<Item = Mod>;        // table + DigBoost for pickaxes
pub fn worn_mods(equipped: &[Option<ItemId>; SLOT_COUNT]) -> Mods; // only mods allowed for the item's slot

#[derive(Clone, Copy, Default, PartialEq, Debug)]       // + serde cfg_attr
pub struct Mods { pub extra_jump: bool, pub glide: bool, pub run_pct: u8, pub dig: u16, pub reach: u8, pub gem: u8 }

// crafting.rs (new)
pub struct Recipe { pub station: Option<ItemId>, pub inputs: &'static [(ItemId, u16)], pub output: (ItemId, u16) }
pub const RECIPES: &[Recipe] = &[
    /* 0 */ Recipe { station: None,            inputs: &[(PLANKS, 10), (LOG_END, 4)],                    output: (WORKBENCH, 1) },
    /* 1 */ Recipe { station: Some(WORKBENCH), inputs: &[(PLANKS, 8), (LOG_END, 2)],                     output: (CRATE, 2) },
    /* 2 */ Recipe { station: Some(WORKBENCH), inputs: &[(PLANKS, 6)],                                   output: (LADDER, 4) },
    /* 3 */ Recipe { station: Some(WORKBENCH), inputs: &[(PLANKS, 8), (LOG_END, 4)],                     output: (equip("pickaxe_wood"), 1) },
    /* 4 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_wood"), 1), (STONE, 20), (COBBLESTONE, 10)], output: (equip("pickaxe_stone"), 1) },
    /* 5 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_wood"), 1), (GRAVEL, 20), (STONE_BRICKS, 10)], output: (equip("pickaxe_copper"), 1) },
    /* 6 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_stone"), 1), (STONE_BRICKS, 30), (MOSSY_STONE, 20)], output: (equip("pickaxe_iron"), 1) },
    /* 7 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_iron"), 1), (SAND, 50), (GLASS, 30)], output: (equip("pickaxe_gold"), 1) },
    /* 8 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_iron"), 1), (VINE_SOIL, 40), (MOSSY_STONE, 40)], output: (equip("pickaxe_emerald"), 1) },
    /* 9 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_gold"), 1), (ICE, 100), (GLASS, 60)], output: (equip("pickaxe_diamond"), 1) },
    /*10 */ Recipe { station: Some(WORKBENCH), inputs: &[(equip("pickaxe_gold"), 1), (STONE_BRICKS, 80), (CRATE, 40)], output: (equip("pickaxe_drill"), 1) },
    /*11 */ Recipe { station: Some(WORKBENCH), inputs: &[(STONE_BRICKS, 40), (CRATE, 10)],               output: (equip("shoes_boots_iron"), 1) },
    /*12 */ Recipe { station: Some(WORKBENCH), inputs: &[(VINE_SOIL, 30), (FERN, 20)],                   output: (equip("shoes_sneakers_red"), 1) },
    /*13 */ Recipe { station: Some(WORKBENCH), inputs: &[(GLASS, 40), (ICE, 20)],                        output: (equip("face_goggles"), 1) },
    /*14 */ Recipe { station: Some(WORKBENCH), inputs: &[(FERN, 100), (VINE_SOIL, 50)],                  output: (equip("wings_leaf_green"), 1) },
    /*15 */ Recipe { station: Some(WORKBENCH), inputs: &[(CRATE, 150), (GLASS, 100), (ICE, 50)],         output: (equip("wings_feather_gold"), 1) },
];
// `equip(name)` = the const lookup of an EQUIPMENT folder name to its ItemId (400 + index); a test asserts every name exists.

pub const CRAFT_STATION_REACH: u32 = REACH;    // the station must be within normal reach (Reach mod does not extend it)
```

### Rules and protocol

- `Action::Craft { recipe: u16 }` → `Reject::Invalid` (unknown recipe), `Reject::OutOfRange` (needs a station and
  none within `CRAFT_STATION_REACH` of the player's tile), `Reject::NoItem` (an input short),
  `Reject::InventoryFull` (output does not fit after removing inputs; simulate on a cloned inventory like the trade
  commit). On success: `InventoryChanged` per changed item (+ `Equipped { item: None }` if a worn input's last copy was
  consumed), plus `Event::Crafted { player, recipe }` (private) for the client's feedback and sound.
- **Physics needs the mods.** `step` gains the worn mods (`step(body, input, world, mods: Mods)` or the fields folded
  into `Input`); client prediction and the server pass the same values, so reconciliation stays exact. `Body` gains
  `extra_jump_used: bool`. Order inside the step: an airborne jump press with `extra_jump && !extra_jump_used` sets
  `vel.y = EXTRA_JUMP_VELOCITY`; `glide && jump_held && vel.y < 0` clamps `vel.y` to `-GLIDE_FALL_SPEED`; `run_pct`
  scales `WALK_SPEED`. Landing clears `extra_jump_used`.
- `punch` uses `mods.dig` (default `FIST_DAMAGE`) and health `hits × DAMAGE_PER_HIT`; saplings `SAPLING_HITS × 4`;
  harvest stays one punch. `in_reach` takes `REACH + mods.reach` for punch and place. Gem rolls add `mods.gem` to the
  upper bound.
- New serde derives on `Mod`, `Mods` and the new Action/Event variants; mirror arms for `Crafted`.

### Client (step 2)

- Wrench-click on a Workbench in reach opens a craft panel (wood kit): the recipe list (output icon and name), the
  selected recipe's inputs with **have / need** counts (red when short), and a Craft button that sends
  `Action::Craft { recipe }`, greyed when an input is short or no station is in reach (display only; the server
  decides). Recipes with `station: None` are also listed from the inventory panel ("Craft" tab or button).
- Item tooltips show the mods in one line, e.g. "Extra jump", "Glide", "+20% run", "Dig 1.5×", "+1 reach",
  "+1 gems".
- `Crafted` plays `sfx/craft.ogg` and a short toast.
- Art (all exist, spec in `assets/README.md` "Crafting and mods"): `ui/icon_craft.png` (Craft tab/button),
  `ui/craft/arrow.png` (between inputs and result), Craft button `ui/lobby/button_primary.png` (+ `_hover`,
  `_pressed`) and `ui/craft/button_craft_disabled.png` (9-slice 10), recipe slots `ui/slot.png` /
  `ui/slot_selected.png`, tooltip mod icons `ui/mods/mod_{extra_jump,glide,run_boost,reach,dig_boost,gem_finder}.png`.

### Tests that must exist

- Every `ITEM_MODS` name exists in `EQUIPMENT`; every mod sits on a slot allowed for it; no item has more than two.
- `worn_mods` ignores mods on the wrong slot and defaults to `FIST_DAMAGE`.
- Pickaxe ladder: stone takes 6/6/5/4/3 punches with fist/tier 1/2/3/4.
- Physics: ExtraJump clears a 3-tile wall and not a 5-tile wall; only one extra jump per airtime; Glide caps the fall
  speed only while jump is held; RunBoost(20) walks at 7.2 tiles/s; tunnelling assertions still hold; client and
  server steps with the same mods stay bit-identical.
- Reach(1) allows a punch at distance 3, not 4.
- Crafting: every recipe's items exist and counts are 1..=200; recipe 0 works without a station and the others need a
  Workbench in reach; a short input, a full inventory and a missing station each refuse with nothing changed; a
  successful craft conserves everything except the declared inputs and output; consuming a worn pickaxe unequips it.
