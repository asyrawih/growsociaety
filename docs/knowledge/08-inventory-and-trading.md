# 08 · Inventory and Trading

## Knowledge

### Inventory

| Rule | Value | Label |
|---|---|---|
| Starting slots | 16, two of them taken by the fist and the wrench | Reference |
| A slot holds | One item type | Rule |
| Max per item type | **200**, across the whole inventory (one slot per type) | Reference |
| Slots per upgrade | +10 | Reference |
| Max slots | 396 (38 upgrades) | Reference |
| Upgrade price | `cost(0) = 100`, `cost(n + 1) = cost(n) × 6 / 5` in integer maths; the 38th costs about 85,000 gems | Tuning (reference: 100 rising exponentially to 137,000) |
| Fist and wrench | Always present, cannot be dropped, traded or removed | Reference |

The cap of 200 is the most load-bearing number in the economy. It forces large holdings out of backpacks and into
worlds (as placed blocks or drops), which is what makes owning and protecting land necessary.

Because each type has one slot and a cap of 200, "can this player receive N of item X" is a single comparison:
`current + N <= 200` and (`current > 0` or a free slot exists).

### Adding items

`add(item, count)` returns how many were actually added. It never silently discards: the caller decides what
happens to the remainder (a pickup leaves it on the ground; a purchase or a trade refuses the whole thing).

### Dropping and trashing

| Action | Effect |
|---|---|
| Drop N | Removes N from the inventory and creates a drop on the tile next to the player, on the side the action names. Fails if that tile's fg is solid or outside the world, or the world drop limit is reached (details in [05](05-punch-and-build.md)) |
| Trash N | Destroys N. Asks for confirmation in the UI |

Dropped items belong to nobody. Dropping is how items leave the 200 cap, and how unsafe "trust trades" happen, which
is the reason a real trade system exists.

### Trading

Trading is an atomic swap between two players in the same world. Either both sides change or neither does.

| Rule | Value | Label |
|---|---|---|
| Slots per side | 4 item stacks | Reference |
| Same world | Required for the whole trade | Reference |
| Receiving limit | A trade that would put a receiver over 200 of an item, or need slots they lack, cannot complete | Reference |
| Gems | Cannot be offered | Reference |
| `UNTRADEABLE` items | Cannot be offered | Rule |
| Moving more than 200 | Not possible in one trade by design. Players trade a claimed world's ownership instead (later feature) | Reference |

### Trade flow (ours; the reference source did not document its confirmation steps)

```
Idle ──request──> Requested ──accept──> Editing ──both lock──> Review ──both confirm──> Done
                      │                    │  ^                   │
                    decline              any edit unlocks both   either cancels
                      v                    v                      v
                    Idle                 Editing                 Idle
```

1. **Request.** A asks B. B gets a prompt. B may have only one open request or trade at a time.
2. **Editing.** Each side adds or removes up to 4 stacks from their own offer. Both see both offers live.
3. **Lock.** Each side presses lock when satisfied. **Any change to either offer clears both locks.** This is the
   core anti-scam rule: you can never have locked in a view that differs from what is now on the table.
4. **Review.** With both locked, a final screen shows exactly what each side gives and gets. Offers can no longer be
   edited, only confirmed or cancelled. A short delay (`TRADE_REVIEW_DELAY = 2 s`, Tuning) before confirm is enabled
   defeats last-moment swaps and reflex clicks.
5. **Commit.** When both confirm, the server re-validates everything from scratch and swaps.
6. **Cancel.** Either side leaving the world, disconnecting or pressing cancel returns to Idle with nothing moved.

Items in an offer **stay in the owner's inventory** until commit. An offer is a list of promises, not an escrow.
Consequence: the commit must re-check that each side still owns what they offered.

## Technical

### Data

```rust
pub struct Inventory {
    pub capacity: u16,                 // slots
    slots: Vec<(ItemId, u16)>,         // one entry per item type, count 1..=200
}

impl Inventory {
    pub fn count(&self, item: ItemId) -> u16
    pub fn room_for(&self, item: ItemId) -> u16          // how many more would fit
    pub fn add(&mut self, item: ItemId, n: u16) -> u16   // returns amount added
    pub fn remove(&mut self, item: ItemId, n: u16) -> bool // all or nothing
}

pub const MAX_STACK: u16 = 200;
```

Keep `slots` private. The cap and the one-slot-per-type rule are enforced inside `add`; no other code may push into
the vector. Slot order is the order items were first acquired, which is also the hotbar order.

### Trade types (agreed shape, 2026-11-13)

The state machine lives in `game_shared` (`trade.rs`) and runs through `apply`, so offline tests and the server share
it. The coding session routes the events and writes both inventories in one database transaction on commit.

```rust
// WorldState gains:
pub trades: Vec<Trade>,          // at most one open request or trade per player

pub struct Trade {
    pub id: u32,
    pub a: PlayerId, pub b: PlayerId,           // a requested, b was asked
    pub stage: TradeStage,                      // Requested | Editing | Review { since: f64 } (Done/cancelled = removed)
    pub offers: [[Option<(ItemId, u16)>; TRADE_SLOTS]; 2],   // index 0 = a, 1 = b
    pub locked: [bool; 2],
    pub confirmed: [bool; 2],
}

Action::TradeRequest { to: PlayerId }
Action::TradeRespond { accept: bool }                  // by the asked player, on their pending request
Action::TradeSetOffer { slot: u8, item: ItemId, count: u16 }   // count 0 clears the slot
Action::TradeLock { locked: bool }
Action::TradeConfirm
Action::TradeCancel

Event::TradeRequested { trade: u32, from: PlayerId, to: PlayerId }
Event::TradeUpdated { trade: Trade }                   // full state after any change (opened, offer, lock, review)
Event::TradeCompleted { trade: u32 }                   // followed by InventoryChanged events for both players
Event::TradeCancelled { trade: u32, reason: TradeEnd } // Declined | Cancelled | Left | Invalid | CarryingTooMuch

pub fn end_trades_of(state: &mut WorldState, player: PlayerId) -> Vec<Event>   // called when a player leaves the world
```

- Trade events are **private to the two players** (server routing done); `end_trades_of` runs in `Room::remove`; a
  `TradeCompleted` triggers an immediate save of the world and both inventories in one database transaction.
- `TRADE_SLOTS = 4`, `TRADE_REVIEW_DELAY = 2.0` s (confirm before then is `Reject::Cooldown`), counts `1..=200`, no
  duplicate items in one offer, no `UNTRADEABLE` or `PERMANENT` items, the item must be held in that amount when offered.
- Any `TradeSetOffer` by either side clears both `locked` and both `confirmed`; both locked moves to `Review`.
- On `TradeConfirm` by both: the commit algorithm below, then `TradeCompleted` + `InventoryChanged` per changed item
  (+ `Equipped { item: None }` if a worn item's last copy left).

### Commit algorithm (server, Phase 5+)

```
1. both players still in the same world, both in Review, both confirmed      else cancel
2. for each side: every offered (item, n) has inventory.count(item) >= n     else cancel
3. no offered item is UNTRADEABLE; at most 4 stacks; no duplicate item types else cancel
4. simulate: clone both inventories, remove outgoing, add incoming;
   every add must return the full amount                                     else cancel with "carrying too many"
5. replace both inventories with the simulated ones, in one database transaction
```

Step 4 simulates on clones instead of reasoning about capacity, so the check cannot drift from what `add` really
does. Counts are unsigned and validated as `1..=200` when the offer is edited; a zero or oversized count is
`Reject::Invalid`. (The reference game shipped a negative-quantity trade bug. Unsigned types plus range checks at
the boundary make that class impossible.)

Both inventories must be written in a **single transaction** ([10](10-networking-and-persistence.md)). A crash
between two separate writes duplicates or deletes items.

### Actions

```rust
Action::Drop { item, count, left: bool }
Action::Trash { item, count }
Action::BuyBackpackUpgrade
Action::TradeRequest { to } | TradeRespond { accept } | TradeSetOffer { slot: u8, item, count }
      | TradeLock | TradeConfirm | TradeCancel
```

Inventory, drop, trash and upgrade are single-player features and ship in M4–M6. The `Trade*` actions need two
connected players and ship after Phase 5; the state machine can and should be written and tested in `game_shared`
before any networking exists.

### UI (client)

- Art: `assets/ui/` (wood-and-parchment kit; 9-slice insets and the reference layout `ui/_mockup.png` are in
  `assets/README.md`, section "UI"). Nearest-neighbour sampling.
- **Equipment never appears in the hotbar** (owner's rule, 2026-11-13). The hotbar shows the first 8 **non-equipment**
  inventory items, selected slot highlighted, count in the corner. Number keys 1–8 or a click select.
- **Inventory and Wardrobe are separate panels**, switched with tabs (owner's rule).
  - **Inventory** (**E** or **I**): a grid of the non-equipment items. Clicking one selects it, even beyond the first 8.
  - **Wardrobe**: the character preview in the middle (the player's current look drawn at 2×, rest pose, in
    `preview_frame.png`), left column hair, hat, face, shirt, right column pants, shoes, back, hand. A filled slot shows
    `slot_worn.png` and the item's icon; an empty one `slot_equip_empty.png` and `slot_icon_<slot>.png`. Clicking a
    filled slot unequips. Below, a grid of the equipment the player owns; clicking an item equips it.
  - **Escape** or the close button closes the panel (and the store).
  Known gap: the grids do not scroll, so near the 396-slot maximum they run off a 720 px window.
- **Q** drops one of the selected item on the side the player faces. **X** trashes one; it must be pressed twice
  within 2 s. Equipment is dropped or trashed from the wardrobe grid with the same keys while it is selected there.
- Clicks on the hotbar or any open panel never reach the world (one shared "pointer is over UI" check).
- Layout must be anchored and scale with the window; touch targets at least 44 px (README principle 9).
- The HUD updates from `InventoryChanged` / `GemsChanged` events, not by polling the inventory every frame.

### Tests that must exist

- `add` past 200 adds only up to 200 and returns the amount added; a new type with no free slot adds 0.
- `remove` of more than held returns false and changes nothing.
- The fist and wrench cannot be dropped, trashed or offered.
- Upgrade: capacity goes 16 → 26, gems drop by `cost(0)`; the 39th upgrade is rejected; `cost` is strictly increasing.
- Drop then pickup returns the same count; pickup with 190 held of a 30-stack takes 10 and leaves 20.
- Trade state machine: an edit after lock clears both locks; commit with a receiver at 199 getting 2 cancels and
  moves nothing; commit after one side dropped their offered items cancels; a successful commit conserves the total
  count of every item across both inventories.
