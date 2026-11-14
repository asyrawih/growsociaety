# UX · Item tooltip (U3), store names and stats (U4), top-bar labels (U9)

Spec for 1a. One tooltip component in `hud.rs` used by every item grid, plus the existing info line above the hotbar
for touch and selection. Today `refresh_item_info` already writes the hovered item's name and mods into that bottom
line, so the text logic exists; this spec moves it next to the pointer and adds a description line.

## Tooltip box (desktop)

```
        ┌──────────────────────────┐
 [cell] │ Golden Pickaxe      ×1   │  line 1: name (title case) + count if > 1
        │ ⛏ Dig 2x                 │  line 2: mod icons + mod_label text (ui/mods/*.png, 16 px)
        │ Hand · worn              │  line 3: what it is (table below)
        │ ◆ 15000                  │  line 4: store only: price, red if you can't afford
        └──────────────────────────┘
```

- Shows after the pointer rests 300 ms on an item cell; hides at once when it leaves. Follows the cell, not the
  pointer: anchored to the cell's right edge (left edge if that would leave the window), top-aligned, kept fully on
  screen. 16 px text, `INK_COLOR` on `ui/tooltip.png` (9-slice, inset 6).
- Lines with nothing to say are left out (no empty rows). Max width 280 px, text wraps.
- Grids that get it: inventory, wardrobe, store (all tabs), trade (both sides + picker), craft (recipe list and input
  slots), hotbar.

### Line 3 by item kind

| Kind | Text |
|---|---|
| Seed | `Plant on soil · grows in <t>` (`farming::grow_seconds(rarity)`, as `45s`, `3m 20s`, `1h 5m`) |
| Block | `Block · <hits> punches` (fist hits from `ItemDef::hits`) |
| Workbench | `Crafting station · press C nearby` (workbench.md) |
| Background | `Background` |
| Claim | the store line from [claims.md](claims.md) §6, e.g. `Protects the 10 tiles nearest it` |
| Tool | Wrench: `Click your claim or a Workbench` · Fist: `Punch blocks, harvest trees` |
| Equip | `<Slot>` + `· worn` when equipped (e.g. `Back · worn`) |
| Resource | `Used in crafting` |

## Store (U4)

Every price in the store (equipment cells and claim rows) is the gem icon + number, never `N gems`. Hover gives the full tooltip,
so pickaxes and wings can be compared by their mod line (`Dig 2x`, `Glide`). A worn item's tooltip says `· worn`.
The store header shows your balance: gem icon + number, top right of the panel, same text style as prices.

## Selection strip (touch, keyboard, gamepad)

The line above the hotbar already shows the selected item. It now uses the same lines 1–3 on one row, parts
separated by ` · ` (`Golden Pickaxe · Dig 2× · Hand`). Inside a panel, the last tapped or focused cell drives it instead of the hover,
so nothing in this spec is hover-only. No tooltip box on touch.

## Top bar (U9)

Same tooltip box, 300 ms hover, under the button: `Store (B)` · `Inventory (E)` · `Wardrobe` · `Chat (Enter)` ·
`Voice (N)` · `Settings`. Keys come from the constants in `hud.rs`/`settings.rs`, never typed twice.

## States

| State | Behaviour |
|---|---|
| Empty cell | No tooltip |
| Item disappears while shown (used up, traded) | Tooltip hides |
| Dragging an item | No tooltip until the drag ends |
| Panel closes | Tooltip hides |

## Do NOT build

- No rich comparison panels (`+0.5x vs worn`); the mod line is enough for now.
- No lore/flavour text, no rarity colours on names.
- No tooltip on world tiles (that is the claim name tag in claims.md only).
