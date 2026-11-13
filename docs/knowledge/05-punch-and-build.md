# 05 · Punch and Build

## Knowledge

Punching and placing are the two verbs of the game. Everything else (farming, claims, doors) is one of these two
verbs applied to a special tile.

### Range

| Rule | Value | Label |
|---|---|---|
| Punch range | 2 tiles beyond the player's own tile. The own tile is reachable too (distance 0–2) | Reference |
| Build range | Same as punch range | Reference |
| Shape | Chebyshev: `max(|dx|, |dy|) <= REACH` (a 5×5 square around the player) | Rule (ours; simple and predictable) |
| Line of sight | Not required | Rule |

"The player's tile" is the tile containing the hitbox center. Reaching your own tile matters: it is how you put a
background behind yourself. A solid block there is refused anyway by the "no player overlaps" rule.

If a filled world could make blocks unbreakable, anyone could freeze a world by littering it; that is why breaks
ignore the drop limit. `REACH = 2` is a constant; items that extend reach are
a later feature and would add to it.

### Punching a tile

1. Target the **foreground** if the tile has one; otherwise the **background**. You must break the block before the
   wall behind it.
2. Each punch adds 1 damage to that tile's layer. When damage reaches the item's `hits`, it breaks.
3. If the tile is not punched again for `DAMAGE_RESET = 5 s` (Tuning; reference blocks use 4–8 s per item), its damage
   returns to 0. One global value is used instead of per-item values until a design reason appears.
4. Damage is per tile, shared by all players: two players punching the same block break it together.
5. `UNBREAKABLE` items ignore punches. So does anything the player has no permission to edit
   ([07](07-economy-and-locks.md)).
6. `PUNCH_COOLDOWN = 0.2 s` (Tuning) between punches from one player. Holding the button repeats.

Special targets:

| Target | Punch does |
|---|---|
| Grown tree | Harvests it, see [06](06-farming-and-splicing.md) |
| Sapling (not grown) | Damages and eventually destroys it; returns nothing |
| Main door | Leaves the world |
| Another player | Small knockback (later; needs multiplayer) |
| Empty tile | Nothing; plays the miss animation |

### What a broken block drops

Each break makes one roll (Rule: exactly one outcome, so a block is never duplicated):

| Outcome | Chance | Label |
|---|---|---|
| The block itself | `DROP_BLOCK = 0.40` | Tuning |
| Its seed | `DROP_SEED = 0.20` | Tuning |
| Nothing | the remaining 0.40 | Tuning |

`NO_SEED` items turn the seed outcome into the block outcome. Gems drop **in addition**, see
[07](07-economy-and-locks.md). The seed chance matters for the economy: it must satisfy the sustainability equation
in [06](06-farming-and-splicing.md).

Drops appear as a floating item at the broken tile, not directly in the inventory. This is deliberate: it is what
makes "break for someone else" and stealing unprotected farms possible, which is what makes claims valuable.

### Placing

A place action names a tile and an inventory item. It succeeds only if **all** of these hold:

| Check | Applies to |
|---|---|
| Tile is in range | all |
| Player has at least 1 of the item | all |
| Player may edit the tile (claims) | all |
| Target layer is empty (fg for blocks, bg for backgrounds) | blocks, backgrounds |
| Tile is not the main door tile | all |
| No player hitbox overlaps the tile | items with `Solid` collision only |
| Fg is empty **and** the tile below has a solid fg | seeds |
| Fg holds a sapling of a different seed that has not been spliced | seeds (this is a splice, see [06](06-farming-and-splicing.md)) |

On success one item leaves the inventory. Placement does not need a neighbouring block: you can build in mid-air.

### Dropped items

| Rule | Value | Label |
|---|---|---|
| Position | A drop is a **point** at the centre of its tile | Rule |
| Pickup | Automatic when the player's hitbox contains that point and the inventory can take it | Rule |
| Drop action | Lands on the tile next to the player's tile, on the side the action names (left or right). Rejected if that tile is solid or outside the world. Because pickup needs the point inside the hitbox, the dropper does not take it straight back | Rule |
| Partial pickup | Takes as many as fit under the 200 cap, leaves the rest | Rule |
| Merge | Drops of the same item on the same tile merge into one stack | Rule |
| Despawn | Never (initially) | Tuning |
| Limit | `MAX_DROPS_PER_WORLD = 2000`. At the limit a **break or harvest still succeeds and its drop is lost**; an explicit Drop action is rejected (`Reject::DropLimit`). Merging into an existing stack is always allowed | Tuning |
| Physics | None. A drop stays where it was created | Rule (cheap; revisit only if it feels wrong) |

## Technical

### Everything goes through Actions

Punch and place are never applied directly by client code. They are `Action`s passed to one function in
`game_shared` (full contract in [09](09-architecture.md)):

```rust
pub enum Action {
    Punch { x: u32, y: u32 },
    Place { x: u32, y: u32, item: ItemId },
}

pub enum Event {
    TileDamaged { x: u32, y: u32, layer: Layer, damage: u8 },
    TileChanged { x: u32, y: u32, tile: Tile },
    ItemDropped { drop: Drop },
    DropTaken { drop_id: u32, by: PlayerId, count: u16 },
    InventoryChanged { player: PlayerId, item: ItemId, count: u16 }, // new total
    GemsChanged { player: PlayerId, gems: u32 },
}

pub enum Reject { OutOfRange, NoPermission, Occupied, NoItem, Unbreakable, Cooldown, Invalid }

pub fn apply(state: &mut WorldState, player: PlayerId, action: Action, now: f64)
    -> Result<Vec<Event>, Reject>
```

The checks in the tables above live inside `apply`, in `game_shared`. The client's job is to turn a click into an
`Action` and to play the `Event`s it gets back. This is what makes the single-player build and the future server
share one rule set.

### Transient damage

```rust
pub struct Damage { pub hits: u8, pub layer: Layer, pub last_hit: f64 }
// WorldState.damage: HashMap<u32, Damage>, keyed by tile index. Not persisted.
```

On each punch: if an entry exists and `now - last_hit > DAMAGE_RESET`, reset it first. There is no ticking system
that expires entries; expiry is checked lazily on the next punch, and the client hides cracks with its own timer.
Remove the entry when the tile breaks.

### Input mapping (client)

| Input | Action |
|---|---|
| Left mouse on a tile (held repeats) | `Punch` if the fist is selected, otherwise `Place` with the selected item |
| Number keys / click a hotbar slot | Select an item |
| Touch: tap a tile | Same as left mouse (README principle 9: mobile-friendly input) |

Convert the cursor to a tile with the camera's viewport-to-world conversion, then `game_shared::world_to_tile`.
Show a highlight on the hovered tile, tinted differently when it is out of range, so range is learnable.

### Tests that must exist (all in `game_shared`, no Bevy)

- Punching a 3-hit block three times breaks it; twice then waiting past `DAMAGE_RESET` then once does not.
- Punch at Chebyshev distance 0 and 2 succeeds, at distance 3 returns `OutOfRange`.
- Bedrock returns `Unbreakable`; punching the main door returns `LeftWorld`; both tiles stay unchanged.
- At the drop limit a break still removes the block and yields no drop; a Drop action returns `DropLimit`.
- A Drop action puts the stack on the adjacent tile and the dropper does not pick it up until they walk onto it.
- A tile with fg and bg: the first break removes fg only; bg needs its own hits.
- Placing a solid block on a tile overlapping the player returns `Occupied`; a background there succeeds.
- Placing removes exactly one item; placing with zero returns `NoItem`.
- Across 10,000 seeded breaks the three drop outcomes land within 2 percentage points of their chances, and no break
  yields both a block and a seed.
