# 03 · Items and Tiles

## Knowledge

### One table for everything

Every thing a player can hold or a tile can contain is an **item** with a numeric id. A block in the world and the
same block in a backpack are the same `ItemId`. This is what lets one inventory, one trade system and one drop
system cover the whole game.

### Item kinds

| Kind | Goes in | Example | Notes |
|---|---|---|---|
| `Block` | Tile foreground | dirt, stone, planks | Most have collision |
| `Background` | Tile background | cave wall, wood wall | Never collides |
| `Seed` | Planted on an empty fg tile above a solid block | dirt seed | Becomes a tree, see [06](06-farming-and-splicing.md) |
| `Claim` | Tile foreground | small claim, world claim | Owns tiles, see [07](07-economy-and-locks.md) |
| `Tool` | Inventory only | fist, wrench | Fist and wrench are permanent |
| `Equip(Slot)` | Worn or held by the player | brown hair, red shirt, iron pickaxe, bat wings | See [Equipment](#equipment) below |
| `Resource` | Inventory only | berries, wheat | For later crafting/consumables |

Gems are **not** items. They are a number on the player (see [07](07-economy-and-locks.md)).

### Collision kinds

| Kind | Behaviour | Used by | Milestone |
|---|---|---|---|
| `None` | Walk through | empty, backgrounds, trees, flowers, signs | M1 |
| `Solid` | Blocks movement from every side | most blocks | M1 |
| `Platform` | Solid only from above; hold down to drop through | platform | later |
| `Ladder` | Up/down moves the player, gravity off while on it | ladder | later |
| `Hazard` | Touching it respawns the player at the main door | lava, spikes | later |
| `Bounce` | Landing on it launches the player upward | bounce mushroom | later |
| `Liquid` | Slows movement, reduces gravity | water | later |

Implement `None` and `Solid` first. The enum should list all seven from the start so adding behaviour later does not
change the item table's shape.

### Rarity

- Every item has a `rarity: u16`. Base items found in a generated world have rarity 1 (lava: 2).
- A spliced item's rarity is the **sum of its two parents' rarities** (Rule).
- Rarity drives exactly two things: tree grow time and gem drops. It does **not** change how many blocks a tree
  gives or how often seeds drop (Reference).

### Item flags

| Flag | Meaning |
|---|---|
| `UNBREAKABLE` | Punching does nothing (bedrock, main door) |
| `UNTRADEABLE` | Cannot be put in a trade or dropped (fist, wrench) |
| `NO_SEED` | Has no seed and never drops one (bedrock, main door, claims, tools) |
| `PERMANENT` | Cannot leave the inventory by any means |

## Technical

### Definition

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct ItemId(pub u16);
pub const EMPTY: ItemId = ItemId(0);

pub struct ItemDef {
    pub id: ItemId,
    pub name: &'static str,
    pub kind: ItemKind,
    pub collision: Collision,
    pub rarity: u16,
    pub hits: u8,          // punches to break with a bare fist; 0 for non-placeables
    pub flags: u8,
    pub sprite: Sprite,    // Terrain(u8) | Block(&'static str) | Tinted(&'static str, u32)
}

pub const ITEMS: &[ItemDef] = &[ /* … */ ];
pub fn item(id: ItemId) -> Option<&'static ItemDef>
```

**Why a const table and not a data file (D3):** it needs no parser, no dependency and no load-error path, and the
compiler checks every row. "Data-driven" here means behaviour reads the table instead of `match`ing on ids.
`item()` is the only way systems learn about an item; **no system may hard-code an id comparison** except for the
named constants `EMPTY`, `BEDROCK`, `MAIN_DOOR`, `FIST`, `WRENCH`.

World generation is the one other place that names ids (`DIRT`, `STONE`, `SAND`, `LAVA`, `CAVE_WALL`): it has to
say what it places and replaces. The ban is on **gameplay systems** branching on ids.

### Sprites and icons

| Variant | In the world | In the inventory / as a drop |
|---|---|---|
| `Terrain(n)` | Autotiled from `assets/tiles/terrain.png` | Atlas cell `n * 16 + 0` (mask 0, the single floating block) |
| `Block(file)` | `assets/blocks/<file>`, 32×32 | The same image |
| `Tinted(file, rgb)` | `assets/blocks/<file>` multiplied by `rgb` (`Sprite.color`) | The same |

- Main door: `Block("65_door.png")`, a 1-tile door.
- Wrench: `Block("64_wrench.png")`.
- Seeds: every seed is `Tinted("66_seed_gray.png", rgb)`. The sprite is greyscale; `rgb` is the parent block's
  representative colour, written as a column in the seed's table row (pick it from the block's art, palette in
  [`../art/STYLE.md`](../art/STYLE.md)). Two seeds must not share a colour that reads the same; the item name in the
  UI is still the reliable way to tell them apart.
- There is one `sprite` field, not a separate icon field: the icon is derived by the table above.
- Large 64×64 objects in `assets/objects/` have no sprite variant yet; add one when an item needs it.

Move to a RON/JSON file when someone who does not compile Rust needs to edit items, or when the server must hot-reload
them. Until then the table is the lazy, correct choice.

### Id assignment

- `0` is empty, forever.
- For blocks with art, the id is the `NN` prefix of `assets/blocks/NN_name.png`. The file name is the source of
  truth, so ids and art cannot drift.
- Terrain materials drawn from the autotile atlas use the free numbers: `1` dirt, `5` sand. Stone is `2` (its id matches
  `02_stone.png`, but it is drawn from the atlas like the other terrain materials; that file is unused).
  Grass has no id: it is how exposed dirt is drawn ([02](02-world.md)).
- Backgrounds start at `100` (`bg_dirt` = 100 is the cave wall, then `bg_stone`, `bg_wood`, `bg_brick`, `bg_stone_bricks`).
- Seeds: `seed id = 1000 + item id` (backgrounds have seeds too: cave wall `100` → seed `1100`). A seed is found with arithmetic, not a lookup field. `NO_SEED` items have no row there.
- Claims start at `200`. The main door is `300`. Tools: fist `47`, pickaxe `46` (from their sprite files), wrench `301`.
- Ids are **persisted and sent over the network**. Never renumber an id that has shipped; add new ones.

### Starter table (M1 needs only these rows)

Id `0` (empty) has **no row**: `item(EMPTY)` returns `None`, and callers treat `None` as "nothing here, no collision".

| Id | Name | Kind | Collision | Rarity | Hits | Flags |
|---|---|---|---|---|---|---|
| 1 | dirt | Block | Solid | 1 | 3 | |
| 2 | stone | Block | Solid | 1 | 6 | |
| 4 | bedrock | Block | Solid | 0 | 0 | UNBREAKABLE, NO_SEED |
| 5 | sand | Block | Solid | 1 | 2 | |
| 21 | lava | Block | Hazard | 2 | 4 | |
| 100 | cave wall | Background | None | 1 | 2 | |
| 300 | main door | Block | None | 0 | 0 | UNBREAKABLE, NO_SEED |
| 47 | fist | Tool | — | 0 | 0 | UNTRADEABLE, PERMANENT, NO_SEED |
| 301 | wrench | Tool | — | 0 | 0 | UNTRADEABLE, PERMANENT, NO_SEED |

All `hits` values are Tuning.

The rest of the art (`03_cobblestone` … `63_rock`) becomes rows as recipes are designed in
[06](06-farming-and-splicing.md). Do not add a row before something can produce the item.

### Tests that must exist

- Every id in `ITEMS` is unique, and `item(id)` returns the row with that id.
- Every `Block`/`Background` has `hits > 0` or `UNBREAKABLE`.
- Every item without `NO_SEED` has a seed row at `1000 + id`, and every seed row points back to an existing block.
- Every `Sprite::Block` and `Sprite::Tinted` file name exists under `assets/blocks/` (a test that reads the directory).
- Every row has a sprite; there is no `None` variant.

## Equipment

First pass, designed 2026-11-13 at the owner's request (relayed by the asset session). Art: 123 items in
`assets/items/<name>/`, layer rules in `assets/README.md`.

### Knowledge

#### Slots

One item per slot. The slot is the folder-name prefix.

| Slot | Prefix | Drawn as | Items now |
|---|---|---|---|
| `Hair` | `hair_` | layers on the head | 26 |
| `Hat` | `hat_` | layers on the head, over hair | 5 |
| `Face` | `face_` | layers on the head | 1 |
| `Shirt` | `shirt_` | layers on torso and arms | 26 |
| `Pants` | `pants_` | layers on legs | 23 |
| `Shoes` | `shoes_` | layers on legs | 3 |
| `Back` | `wings_` | one layer behind everything | 19 |
| `Hand` | `pickaxe_` | the icon, held in the front hand | 20 |

#### Rules

| Rule | Value | Label |
|---|---|---|
| Equipping | The item must be in your inventory. It **stays in the inventory** while worn and still counts toward the 200 cap and slots | Rule |
| Replacing | Equipping into a filled slot swaps the old item out (it stays in the inventory) | Rule |
| Losing the last copy | If a worn item's count reaches 0 (drop, trash, later trade or admin take), it is unequipped in the same action | Rule |
| Placing | Equipment cannot be placed in the world (`Invalid`) | Rule |
| Seeds | Equipment is `NO_SEED`, rarity 0 | Rule |
| Trading | Tradeable (trading arrives with M7) | Rule |
| Others see it | What you wear is part of the world state, so every player in the world sees it | Rule |
| Starter outfit | A new player owns and wears `hair_brown`, `shirt_red`, `pants_jeans_blue`, `shoes_boots_brown` | Rule |
| Skin tone | Not an item; a player setting, out of scope for this pass (default skin) | Rule |

#### Effects

Clothing, hair and wings are **cosmetic** in this pass. Pickaxes have one effect:

> **Punch power.** A punch adds the equipped `Hand` item's `power` to a tile's damage instead of 1. A block breaks
> when its damage reaches `hits`. With no pickaxe, power is 1 (the fist).

It applies to blocks, backgrounds, saplings and claims. Harvesting a grown tree is still one punch.

| Tier | Power | Price (gems) | Pickaxes |
|---|---|---|---|
| 1 | 1 | 200 | wood, rusty, bone, candy |
| 2 | 2 | 1,000 | stone, copper, iron, ice, nature |
| 3 | 3 | 4,000 | gold, emerald, ruby, fire, lightning, shadow |
| 4 | 4 | 15,000 | diamond, obsidian, drill, rainbow, royal |

Tier 1 is a cosmetic choice (same power as the fist). Powers and prices are Tuning. With tier 4, stone (6 hits)
breaks in 2 punches instead of 6. Wings gaining a double jump is a later feature: it changes physics, so it needs
the equipped state inside `step` and its own tests.

#### How equipment is obtained (first pass: the store only)

| Category | Price (gems) | Label |
|---|---|---|
| Hair | 150 | Tuning |
| Shirt, pants, shoes | 200 each | Tuning |
| Hat, face | 300 each | Tuning |
| Wings | 10,000 | Tuning (the rare, aspirational item) |
| Pickaxes | By tier, table above | Tuning |

Each item is bought directly from the store; there are no random packs. Random packs need published odds and are
deferred. Splicing recipes for wings, drops, and crafting pickaxes from blocks are later additions; they need no new
item data, only new ways to grant the same ids.

### Technical

#### Data

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Slot { Hair, Hat, Face, Shirt, Pants, Shoes, Back, Hand }   // index = slot as usize

pub enum ItemKind { Block, Background, Seed, Claim, Tool, Equip(Slot), Resource }

pub enum Sprite {
    Terrain(u8),
    Block(&'static str),
    Tinted(&'static str, u32),
    Equip(&'static str),   // folder name under assets/items/
}

// Player (rules.rs) gains:
pub equipped: [Option<ItemId>; 8],
```

#### Ids (append-only)

Equipment ids are `400 + index` into one append-only table in `items.rs`:

```rust
pub const EQUIPMENT: &[&str] = &["hair_brown", "hair_bob_blue", /* … every folder, in the order first added */];
```

- The slot is derived from the name's prefix; the price and power from the tables above. No per-item columns.
- **Never reorder or remove an entry.** Ids are persisted and sent over the network. New art is appended.
- Range `400..=999` (600 items). If it fills, start a new range above 2000 and say so here.
- A test asserts that every folder in `assets/items/` (except `pixellab_ids.txt`) has exactly one entry and every
  entry has a folder, and that every name's prefix maps to a slot.

#### Icons

`Sprite::Equip(name)` draws `items/<name>/icon.png`. Every equipment folder has one (the asset session generates
them with `docs/art/item_icons.py`); a test asserts it, so the client needs no runtime compositing for icons.

#### Actions and events

```rust
Action::Equip { item: ItemId }       // Invalid if not equipment; NoItem if not held
Action::Unequip { slot: Slot }       // Ok with no events if the slot is already empty
Event::Equipped { player: PlayerId, slot: Slot, item: Option<ItemId> }
```

- Drop, Trash (and later Trade, `/take`) that bring a worn item's count to 0 also emit `Equipped { item: None }`.
- `Buy` accepts equipment ids, priced from the tables above. Buying does not equip.
- `punch` reads the power from `equipped[Hand]`; `WorldState::join` gives and equips the starter outfit.
- New types get the same `cfg_attr(feature = "serde", …)` derives as the rest of `game_shared`.

#### Client

| Part | Owner | Does |
|---|---|---|
| Store panel (hud.rs) | Scripting subagent | Tabs: Claims, Hair, Hat, Face, Shirt, Pants, Shoes, Wings, Pickaxes. Icon grid per tab (at most 26 per tab, so no scrolling), price, greyed when unaffordable, owned items marked |
| Wardrobe panel (hud.rs) | Scripting subagent | Separate from the inventory; layout and art in [08](08-inventory-and-trading.md) "UI (client)". Equipment never takes a hotbar slot |
| Rig (player.rs, player/rig.rs) | Coding session | Builds each player's `Look.items` from `Player.equipped` (folder names via `item(id)`), redrawn on `Equipped` events, for every player in the world. Draws wings behind everything and the pickaxe in the hand per `assets/README.md` |

#### Tests that must exist

- Every `assets/items/` folder has one `EQUIPMENT` entry and vice versa; ids are `400 + index`; every prefix maps to
  a slot; every equipment row is `NO_SEED` with rarity 0.
- Equip needs the item in the inventory; equipping into a filled slot replaces it; Unequip empties it.
- Dropping or trashing the last copy of a worn item unequips it in the same result; dropping one of two keeps it worn.
- Equipment cannot be placed.
- Punch power: with no pickaxe a 6-hit block takes 6 punches; with a power-4 pickaxe it takes 2; harvest is still one
  punch; the damage reported in `TileDamaged` is the accumulated total.
- Buying equipment charges the table price and does not equip; a new player owns and wears the starter outfit.
- Every reject leaves the state unchanged (extend the existing test).
