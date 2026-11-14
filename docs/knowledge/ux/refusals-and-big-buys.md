# UX · Refusal messages (U2) and big-purchase confirm (U7)

Spec for 1a. Two small changes in `hud.rs`; no new art, no rule changes. Exact text everywhere.

## U2 · Every refusal says why

Rule: the player always learns why nothing happened, except where the world already shows it.
Extend `toast_for_reject` (claims cases are in [claims.md](claims.md) §4 and win over this table):

| Reject | Toast | Why |
|---|---|---|
| `Unbreakable` | `This block can't be broken` | Bedrock/main door look like any block |
| `NoItem` | `You don't have that item` | Happens after a stack runs out mid-click |
| `Invalid` | `That didn't work` | Catch-all; also `warn!` the action and reject so we can give it a real message later |
| `Occupied` (placing a seed) | `Seeds need an empty tile or a sapling` | Decor plants (Fern, flowers: `Collision::None`) don't look full, so the tile seems free |
| `Occupied` (non-seed item on a tree/sapling) | `To splice, use a seed (e.g. <Item> seed) on the sapling`; without a seed form: `To splice, use a seed on the sapling` | Players try splicing with the block, not its seed |
| `Occupied` (place a block on a full tile) | none | The tile is visibly full |
| `AlreadySpliced` | `This tree has already been spliced. Harvest it and plant a fresh seed` | Says what to do next, not just no |
| `OutOfRange` | none | The grey hover tint already says it; fires constantly |
| `Cooldown` | none | Fires constantly while holding punch |

- **Rate limit:** the same text is not shown again within 2 s (holding the mouse on bedrock must not flicker).
- **Tile flash:** when the refused action targets a tile, that tile flashes red for 0.3 s with the toast (reuse the
  hover square, red at 40 %). No flash for store/craft/trade refusals.
- **Sound:** keep the existing `denied` sfx; play it at most once per 2 s too.
- **Test:** every `Reject` variant is listed in a test that asserts it is either mapped to text or in an explicit
  "silent on purpose" list, so a new variant can't be silently forgotten.

## U7 · Confirm before spending ≥ 1,000 gems

Today one click buys anything, including a 2,000-gem world claim or a 10,000-gem wing.

```
normal row:        [icon] World Claim        ◆ 2000
first click:       [icon] World Claim   [ Buy for ◆ 2000? ]   ← primary button art, 3 s
second click:      bought → row back to normal, purchase toast as today
```

| State | Look | Behaviour |
|---|---|---|
| Price < 1,000 | as now | One click buys (no confirm for cheap items) |
| Price ≥ 1,000, idle | as now | First click arms the row |
| Armed | price cell becomes `ui/lobby/button_primary.png` with `Buy for <price>?` (gem icon + number) | Second click on the same row within 3 s sends `Action::Buy`. Click elsewhere, another row, closing the store, or 3 s passing disarms it. Only one row armed at a time |
| Armed + not affordable | n/a | Rows you can't afford never arm: the click goes straight to the server and gets `Not enough gems` (U2 rule) |
| Hover on armed button | `button_primary_hover.png` | — |

- Narrow cells (store equipment grid): the armed cell keeps its size; its frame switches to the primary button art
  and the price line reads `Buy?`. The full `Click again to buy for ◆ 15000` goes in the selection strip above the
  hotbar. Wide rows (claims) keep the inline `Buy for ◆ 2000?`.
- Threshold `CONFIRM_PRICE = 1000` is one constant in `hud.rs`. It is the same idea as X-twice trash
  (`TRASH_CONFIRM_SECS`), so keep the timing consistent: use 3 s here because reading a price takes longer.
- Keyboard/gamepad: the store gets cell/row focus (arrows / d-pad move, starting on the first row when the store
  opens). Enter/A behaves like a click (first press arms, second buys); Esc/B disarms before it closes the store.
  Tabs: `[` / `]` (gamepad LB / RB) switch to the previous / next tab, wrapping, focus back on the first row; the
  same keys switch tabs in every tabbed panel (bag tabs too), and the tab bar shows `[` and `]` at its ends.
- Touch: same two taps; the armed button is the full row height (≥ 44 px).

## Do NOT build

- No modal "Are you sure?" dialog (blocks the screen, gets clicked through).
- No confirm on trades, drops or cheap buys. Trash keeps its own X-twice.
- No "don't ask again" setting.
