# UX · Inventory categories, search and sort

Spec for the implementer. Only the **Inventory** tab of the bag changes. Wardrobe keeps its slot filter row, Craft is
unchanged. Everything here is a client-side **view**: the server's slot order, the hotbar and trading don't change.

## Layout

```
[ Inventory | Wardrobe | Craft ]                                        [x]
(All) (Blocks) (Backgrounds) (Seeds) (Wearables) (Tools) (Claims) (Misc)
[🔍 Search_____________ ]                          [ Sort: Type ▾ ]  54/396
┌──┬──┬──┬──┬──┬──┬──┬──┬──┐
│  │  │  │  │  │  │  │  │  │   grid shows only matching items
└──┴──┴──┴──┴──┴──┴──┴──┴──┘
```

## Categories (chips)

| Chip | Items (`ItemKind`) |
|---|---|
| All | everything (default) |
| Blocks | `Block` |
| Backgrounds | `Background` |
| Seeds | `Seed` |
| Wearables | `Equip(_)` |
| Tools | `Tool` |
| Claims | `Claim` |
| Misc | `Resource` and anything new |

- One chip active at a time (`ui/chip_active.png`, others `ui/chip.png`; 9-slice, inset 6).
- A chip with no items is greyed, keeps its place (so chips never jump around), can't be clicked and is skipped by
  Shift+[ / ]. If the active chip empties (last item dropped), the grid says `No <chip> in your bag`.
- Chips may wrap to a second row on narrow panels; touch scrolls them sideways instead.
- The active chip is remembered while the game runs, not saved.

## Search

- Field with `ui/lobby/input.png` and `ui/icon_search.png`; placeholder `Search`.
- Live filter: case-insensitive substring on the display name. Combines with the active chip (both must match).
- Clicking the field (or **Ctrl+F**) focuses it and sets `TextFocus`, so typing never walks or opens chat.
  **Esc** clears the text; Esc on an empty field gives focus back; Esc again closes the panel.
- No match: the grid area shows `No items match "<text>"` and a `Clear` link that empties the field and chip.

## Sort

One button that cycles `Sort: Slot` → `Sort: Type` → `Sort: Name` → `Sort: Count`. **Slot is the default** and is
today's order, so the bag matches the hotbar until the player chooses otherwise. Type = grouped by chip, slot order
within each; Name = A–Z; Count = most first. Saved in settings. Display only.

## Trash and "junk"

The existing trash (X, then X again within 2 s, on the selected item) stays. The owner wanted a place for items to
throw away later. **Smallest option: no Junk category.** Marking items as junk needs a stored per-item flag and still
ends in the same trash press. Instead:

- **Shift+X twice trashes the whole stack** of the selected item (one confirm for the lot), with the toast
  `Trashed 37 Dirt`.
- With search + sort by Count, cleaning up is select → Shift+X twice.

## Dropping

Dropping exists (Q drops one of the selected item), but nothing says so.

| Way | Result |
|---|---|
| **Q** | Drop 1 of the selected item (as now) |
| **Shift+Q** | Drop the whole stack (`Action::Drop` with the full count) |
| **Drag a cell out of the panel onto the world** | Drop the dragged stack at the player's side (the existing drag system; the cursor shows the item icon while outside the panel) |
| **Drop button** | While the inventory is open, the selection strip ends with two small buttons `Drop` and `Trash`; Drop = 1, Shift+click = stack. Trash needs a second click within 2 s: `Click Trash again to destroy 37 Dirt` |

Drag-to-drop works from the inventory panel only, never from the hotbar, so a slip can't drop a stack.

- Selection strip and item tooltip line: `Q drop · Shift+Q drop stack` (key names from the constants).
- Drops aren't protected on claims (doc 07), so nothing confirms a drop; the toast `Dropped 37 Dirt` appears only for
  stack drops.
- Feel (toss arc, wobble, seed glow): [vfx-farming-punch.md](vfx-farming-punch.md) §(c).
- Gamepad: X drops 1, hold X drops the stack. Touch: the strip's Drop button.

## Input

- **Keyboard:** `[` `]` bag tabs (as now); **Shift+[** / **Shift+]** previous/next chip; Ctrl+F search; arrows
  move in the grid; Esc as above.
- **Gamepad:** LB/RB bag tabs, LT/RT chips; d-pad grid; no search on gamepad (chips cover it).
- **Touch:** chips scroll sideways, each ≥ 44 px tall; tapping the search field opens the on-screen keyboard; the
  sort button cycles on tap.

## Do NOT build

- No custom categories, favourites or pinning, no multi-select, no junk flag.
- No reordering of the real inventory slots (the hotbar mapping stays as it is).
