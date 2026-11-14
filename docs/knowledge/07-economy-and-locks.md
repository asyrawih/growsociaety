# 07 · Economy and Locks

## Knowledge

### Gems

Gems are the soft currency. They are a number on the player, not an item.

| Rule | Value | Label |
|---|---|---|
| Source: breaking a block | uniform `0 ..= 1 + rarity / 4` | Tuning (reference: about one extra gem per 4 rarity) |
| Source: harvesting a tree | uniform `0 ..= 1 + rarity / 2` | Tuning |
| Delivery | Credited straight to the player who broke/harvested | Rule (ours; the reference drops gem pickups on the ground) |
| Tradeable | **No.** Gems cannot be traded, dropped or given | Reference |
| Sinks | Claims, backpack upgrades, later a store | Rule |
| Storage | `u32`, saturating add | Rule |

Crediting directly instead of spawning pickups removes thousands of entities and a whole class of pickup races. If
"breaking blocks for someone else's gems" becomes a wanted feature, revisit.

### Why the claim item becomes the currency

Gems cannot change hands, but items can. The most expensive item everyone needs and anyone can buy at a fixed gem
price becomes the unit players price things in. In this game that is the **world claim** (2,000 gems). This is
expected and wanted: it gives the trade economy a stable anchor backed by a gem sink. Do not make the world claim
untradeable.

Later (not now): a compression item worth 100 world claims, convertible both ways, so large trades fit in 4 trade
slots. Add it when trade values actually exceed 200 world claims.

### Claims

A claim is a block. Placing it gives the placer ownership of a set of tiles.

| Claim | Tiles | Price | Id | Label |
|---|---|---|---|---|
| Small claim | 10 | 50 gems | 200 | Reference |
| Medium claim | 48 | 200 gems | 201 | Reference |
| Large claim | 200 | 500 gems | 202 | Reference |
| World claim | The whole world | 2,000 gems | 203 | Reference |

All four are `NO_SEED`: they come only from the store, which is what makes them a gem sink. Tile counts and prices
live in one table, `claims::CLAIMS`. Claim blocks take `CLAIM_HITS = 8` punches (Tuning) and are drawn as the padlock
sprite tinted per tier: bronze, silver, gold, cyan.

### Which tiles an area claim takes

Rule (ours; chosen because it is deterministic and one sentence long):

> Sort every tile in the world by (Chebyshev distance to the claim, then row, then column). Walk that order and take
> tiles that are not already claimed until the claim has its number. The claim's own tile is first.

A small claim is therefore the claim tile plus the 9 nearest free tiles; a large claim is roughly a 14×14 square.
Near a world edge or another claim, the area bends around the obstacle but keeps its tile count. The claimed set is
computed once at placement and stored; it never changes afterwards.

### Permission: who may edit a tile

One rule, used by every punch and every placement:

1. If the tile is inside an **area claim**: allowed for that claim's owner, anyone on its access list, or everyone if
   the claim is set public.
2. Otherwise, if the world has a **world claim**: the same test against the world claim.
3. Otherwise the tile is free: anyone may edit.

Area claims take precedence over the world claim, so a world owner can hand a plot to a friend by putting them on
the world claim's access list and letting them place an area claim. Setting a claim public lets everyone build on it,
not claim land inside it.

Additional rules:

| Rule | Detail |
|---|---|
| Who may place an area claim | Anyone who may edit the claim's own tile, provided (a) that tile does not already belong to an area claim (yours included): placing a claim inside a claim is `Occupied`; and (b) inside a world claim, only the world claim's owner and its access list may place one. `public` lets everyone build there, not take land away from the owner (`NoPermission`) |
| Who may place a world claim | Anyone, if the world has no world claim and **no area claim owned by someone else** |
| One world claim per world | A second placement is rejected |
| Breaking a claim | Owner only. It always drops itself (never a seed, never nothing) and gives **no gems**, or placing and breaking would mint gems. Its tiles become free. At the world drop limit the last punch is refused (`DropLimit`) so a claim is never destroyed without its item |
| Placing where nothing is free | An area claim whose search finds no unclaimed tile at all is refused (`Occupied`) |
| Editing access | Owner only, by using the wrench on the claim. At most `MAX_CLAIM_ACCESS = 32` players (Tuning); adding the owner or a 33rd player is `Invalid`. Also `/access <player> on\|off` in chat: it changes the nearest claim you own within reach; use `#<id>` for someone offline |
| Dropped items | Not protected. Anyone standing on a drop takes it, claimed tile or not (Tuning; revisit if griefing) |
| Inactivity | A claim whose owner and access list have all been offline 180 days can be broken by anyone (Reference; Phase 7) |
| Main door | Always usable by everyone, even in a claimed world |

Entering a claimed world is always allowed. Claims stop editing, not visiting.

### Store

The store is a fixed price list. Buying is an action: the server checks gems and inventory space, takes the gems,
adds the item. M6 needs only the four claims in it.

## Technical

### Data

```rust
pub struct Claim {
    pub owner: PlayerId,
    pub access: Vec<PlayerId>,
    pub public: bool,
    pub tiles: Vec<u32>,   // tile indices; empty for a world claim
}
// stored in World.extras as TileExtra::Claim, keyed by the claim block's tile index

// World:
claim_of: HashMap<u32, u32>,   // tile index -> tile index of the area claim that owns it
world_claim: Option<u32>,      // tile index of the world claim block
```

`claim_of` is derived data. It is rebuilt from the claims when a world is loaded, never persisted on its own, so the
two cannot disagree after a crash.

### The one permission function

```rust
pub fn can_edit(world: &World, player: PlayerId, tile: u32) -> bool
```

**Every** code path that changes a tile calls this, from inside `apply`. There must be exactly one implementation.
A second copy of the rule in a UI system or a network handler is how protection bypasses are born; QC will grep for it.

### Actions and events

```rust
Action::Buy { item: ItemId }
Action::SetClaimAccess { x, y, player: PlayerId, allow: bool }
Action::SetClaimPublic { x, y, public: bool }
// placing and breaking claims use the existing Place / Punch

Event::ClaimPlaced { x, y, claim }
Event::ClaimRemoved { x, y }
Event::ClaimUpdated { x, y, claim }

Reject::NotOwner, Reject::NotEnoughGems, Reject::WorldAlreadyClaimed, Reject::ForeignClaims, Reject::InventoryFull
```

### Client presentation

- Claimed tiles are not drawn differently in normal play. Using the wrench on a claim you own tints its tiles and
  opens a small panel (owner, tile count, public on/off). A world claim tints nothing. The access-list UI waits for
  multiplayer; the action already exists.
- The store opens with **B**: the four claims and the backpack upgrade, with prices; rows you cannot afford are
  greyed out (display only: the rules still decide when you click).
- A rejected punch or placement on a protected tile shows a short "locked" indicator on that tile. The player must
  always learn **why** nothing happened.

### Tests that must exist

- A small claim owns exactly 10 tiles including its own; medium 48; large 200.
- Two adjacent small claims own disjoint sets and each still owns 10.
- A claim in a corner still owns its full count.
- A non-owner punch and placement inside a claim return `NoPermission`; the owner's succeed; after
  `SetClaimAccess`, the added player's succeed; after `SetClaimPublic`, anyone's succeed.
- Breaking a claim frees its tiles and drops exactly one claim item.
- World claim: rejected when another player owns an area claim in the world; rejected when one already exists; once
  placed, a stranger cannot edit any tile but can still use the main door.
- An area claim inside a claimed world overrides the world claim for its tiles.
- `Buy` with too few gems returns `NotEnoughGems` and changes nothing; with enough, gems drop by the exact price.
- Rebuilding `claim_of` from `extras` gives the same map as the one built incrementally.
