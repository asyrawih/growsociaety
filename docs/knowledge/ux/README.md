# UX · Audit and specs

Owned by the UI/UX session. **Rule (owner, 2026-11-14): every new UI or feature gets a UX spec here before anyone
builds it.** Implementers build from the spec; a change to the flow goes through this folder first.

| File | What it is |
|---|---|
| `README.md` (this) | UX principles for this game, the audit of the current client, the backlog by priority |
| [claims.md](claims.md) | Spec: claims (the owner's top complaint). Owned by growsociaety-64; being built by growsociaety-1a |

A spec has: the problem, the flow, layout, every state, the exact copy (messages), discoverability, acceptance
checks, and who owns which file. Copy in specs is final text; change it here, not in code.

## Principles

1. **Never a silent no.** Every rejected action tells the player *why*, at the place they acted (tile flash + a short
   line). A tile flash alone only works for "too far" and "too fast".
2. **Show before commit.** If an action changes the world or spends gems, show its effect before the click
   (claim area ghost, price vs. balance, craft result).
3. **Names over icons.** An icon without a name is a guess. Every item cell gives its name on hover (desktop) and on
   select (touch, where there is no hover).
4. **One panel, one size.** Tabs inside one panel switch content, never the panel's width or the close button's
   position.
5. **Players, not ids.** A player is always shown by display name. `#id` only in admin/command contexts.
6. **Keys are discoverable.** Every top-bar button and every panel action shows its key in its tooltip.
7. **Touch-ready by default** (doc 15): 44 px targets (already asserted in code), nothing that works *only* on hover,
   safe areas respected.
8. **Wood kit everywhere.** In-world HUD pieces use the same wood-and-parchment art as panels; flat dark boxes only for
   chat/overlays that must stay transparent over the world.

## Audit of the current client (2026-11-14)

Method: read `client/game_client/src/*` (hud, interact, craft_ui, trade_ui, settings, chat, levels_ui), docs 07, 08,
15, 18, and captured every panel with the screenshot mode (offline, `/giveall`):

```sh
GAME_SCREENSHOT=out.png GAME_SCREENSHOT_UI=<inventory|wardrobe|craft|store|store-pickaxes|trade|settings-audio> \
GAME_SCREENSHOT_FILL=1 target/debug/game_client      # run from the repo root
```

What already works well: one consistent wood kit for panels; 44 px targets enforced with `const` asserts; the
wardrobe (preview + worn columns + slot filter row) is clear; disabled store rows are greyed; panels fit the window and
scroll inside; UI clicks never reach the world; event-driven updates.

### P0: players cannot understand what happened

| # | Finding | Evidence | Fix |
|---|---|---|---|
| U1 | **Claims are confusing**: no preview of which tiles a claim takes, claimed land is invisible until you are refused, the panel shows `Claim of player 7`, no access-list UI (only `/access` in chat) | Owner report; doc 07 "Client presentation"; `hud.rs` `refresh_claim_panel` | [claims.md](claims.md) |
| U2 | **Silent refusals.** `OutOfRange`, `Occupied`, `NoItem`, `Unbreakable`, `Cooldown`, `Invalid` show no text (`toast_for_reject` returns `None`). `Occupied` is also what a claim placed inside a claim, or with no free tile, returns, so a failed claim placement says nothing | `hud.rs:1929` | Copy table below. Keep `Cooldown` and `OutOfRange` silent (the tile flash is enough and they fire constantly) |
| U3 | **No item names anywhere in grids.** Inventory, wardrobe, store equipment, trade picker and craft list show icons only. 20+ seeds are near-identical brown/grey dots; pickaxes differ only by colour | Screenshots inventory, store-pickaxes | One shared **item tooltip** (below) |

Reject copy (U2), shown in the toast line and, where a tile is involved, with the tile flash:

| Reject | Text |
|---|---|
| `Occupied` (place on a filled tile) | none (the tile is visibly full) |
| `Occupied` (claim inside a claim / no free tile) | see [claims.md](claims.md) |
| `Unbreakable` | `This block can't be broken` |
| `NoItem` | `You don't have that item` |
| `Invalid` | `That didn't work` (catch-all; log the action so we can give it a real message) |
| `NoPermission` | see [claims.md](claims.md) (owner name replaces the generic text) |

### Item tooltip (U3), one component for every grid

- **Desktop:** hover any item cell ≥ 300 ms → a small parchment box next to the pointer, kept on screen:
  line 1 item name (title case), line 2 count `×200` if > 1, line 3 the mod line with mod icons (the existing
  `mod_label` text), line 4 a short description when the item has one (claims, seeds: `Plant on soil. Grows in 1m 30s`,
  Workbench: `Wrench it to craft`). Store cells add the price with the gem icon.
- **Touch / selection:** the line above the hotbar (today it shows `fist`) becomes the same info for the selected item,
  and panels get the same strip at the bottom of the panel for the cell last tapped. Nothing is hover-only.
- Item display names are title case in UI (`Small Claim`, `Workbench`, `Fist`); the item table keeps its ids.

### P1: friction

| # | Finding | Fix |
|---|---|---|
| U4 | **Store equipment has prices but no names or stats**; you cannot compare pickaxes (dig speed) or wings (glide/jump) | Tooltip (U3) with mods; under each cell keep the price but add the gem icon |
| U5 | **Store shows no balance**; greyed rows don't say how far off you are | Store header: gem icon + balance. Greyed price turns into `need 150 more` on hover/select |
| U6 | **Store tabs wrap to two rows** (9 tabs) | 4 tabs: `Claims` · `Clothes` (with the wardrobe's slot filter row: hair, hat, face, shirt, pants, shoes) · `Wings` · `Pickaxes`. Reuse the wardrobe filter row component |
| U7 | **One click spends any amount**: a 2,000-gem world claim or a 15,000-gem pickaxe buys instantly | Price ≥ 1,000 gems: the click turns the price into a `Buy for 2000?` button for 3 s; a second click buys (same pattern as the X-twice trash) |
| U8 | **Bag panel changes width per tab** (Inventory ~850 px, Wardrobe ~1210 px, Craft ~740 px); the close button jumps | One width for all three tabs (the widest content's), content left-aligned |
| U9 | **Top bar icons have no labels or keys** | Tooltip per button: `Store (B)`, `Inventory (E)`, `Wardrobe`, `Chat (Enter)`, `Voice (N)`, `Settings` |
| U10 | **Craft counts read backwards**: `200/10` = have 200, need 10 | Under each input: need count big (`×10`), and `have 200` small below it, red when short |
| U11 | **Station recipes are hidden** away from a Workbench: the Craft tab shows one recipe, nothing says more exist | List them greyed with `Needs a Workbench nearby`; Craft button disabled with that text |
| U12 | **Trade window is one long column** with tiny state words `(editing)` / `(locked)` | Two columns: `You give` left, `<name> gives` right, each with a state banner (`Editing` / `Locked ✓`). Bottom bar: one primary button whose label follows the state: `Lock offer` → `Waiting for <name>…` → `Confirm (2)` countdown for `TRADE_REVIEW_DELAY` → `Confirm`. `Cancel` secondary |
| U13 | **Gem/level box is a flat dark rectangle**, not the wood kit; XP bar has no numbers | Wood-kit chip (same art as the top bar); tooltip on the XP bar: `Level 3 · 120 / 300 XP` |
| U14 | **Inventory capacity is invisible**; the backpack upgrade lives only in the store | Inventory header: `54 / 396 slots` and a small `+` that opens the store's upgrade row |

### P2: polish and later

| # | Finding | Fix |
|---|---|---|
| U15 | No first-run guidance | First world only: three context hints near the hotbar, each gone after it is done once: `A / D to move, Space to jump` → `Hold left mouse to punch` → `Select a seed and click soil to plant`. Stored in settings |
| U16 | Keys tab is read-only | Fine until rebinding is planned |
| U17 | Mobile (doc 15 P2) | Everything above already avoids hover-only; the touch layout spec gets its own file when P2 starts |

## Backlog order for implementers

Approved by the owner 2026-11-14; specs owned by the UI/UX session (growsociaety-64):

1. [claims.md](claims.md) (U1, gaps C1–C8) → 1a.
2. [refusals-and-big-buys.md](refusals-and-big-buys.md) (U2 + U7) → 1a.
3. [item-tooltip.md](item-tooltip.md) (U3, with U4 + U9) → 1a.
4. [first-run-hints.md](first-run-hints.md) (U15) → f0.
Feature specs added since (owner requests via c8): [platforms.md](platforms.md) · [respawn.md](respawn.md) ·
[farming-feedback.md](farming-feedback.md) · [ladders.md](ladders.md) · [workbench.md](workbench.md) ·
[vfx-farming-punch.md](vfx-farming-punch.md) · [inventory-categories.md](inventory-categories.md) ·
[almanac.md](almanac.md) · [reset-world.md](reset-world.md) · [world-select.md](world-select.md).

5. Not yet specced: U5, U6, U8, U10, U11, U12, U13, U14 (U10/U11 are also in the craft review sent to 1a).
6. Gap found while implementing: Inventory and Wardrobe grids have no keyboard/gamepad focus yet (store, craft and
   tabs do). Spec it with the gamepad pass.
7. Later (note only): decorative plant blocks (Fern, flowers) look too much like saplings; give decor a distinct
   read (e.g. no soil mound, sway animation, or a different base) when the plant art is next touched.

Files: `hud.rs`, `interact.rs`, `world_view.rs` belong to the scripting subagent; `craft_ui.rs`, `trade_ui.rs` to
whoever wrote them (ask in the README owners table); anything in `net.rs`/server to the coding session.
