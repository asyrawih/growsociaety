# UX · Bottom HUD: slide-up bag, chat, and everything at the bottom

Spec for 8e. The genre-standard idea (the bag slides up out of the hotbar) with our own art and layout. The existing
systems stay as they are (TextFocus, the inventory chips/search/sort, the U3 tooltip, drop/trash, MoveItem); only
where they live and how they appear changes. The Store and Settings stay centre panels.

## 1. The bag slides up from the hotbar

The hotbar is the first `HOTBAR_SLOTS` (8) carried items, so the bag grid uses **exactly 8 columns** and the hotbar
is literally its bottom row: one continuous grid.

```
closed                                       open (Inventory tab)
                                             ┌ [Inventory | Wardrobe | Craft] ────────────┐  tab row
                                             │ (All)(Blocks)(Seeds)… [🔍____] Sort 54/396 │  header
                                             │ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢                            │
                                             │ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢   3 rows, scrolls          │  grid
                                             │ ▢ ▢ ▢ ▢ ▢ ▢ ▢ ▢                            │
                                             │ Golden Pickaxe · Dig 2× · Hand  [Drop][Trash]│  strip
               ⌃  ← pull tab                 ├──────────────────── ⌄ ──────────────────────┤  pull tab
   [1][2][3][4][5][6][7][8]                  │ [1][2][3][4][5][6][7][8]                    │  hotbar = row 0
                                             └────────────────────────────────────────────┘
```

| Part | Rule |
|---|---|
| Pull tab | Centred on the hotbar's top edge. `ui/pull_tab.png` (arrow up) closed, `_open` (arrow down) open, `_hover` variants. Hit area 64×32 px (≥ 44 px tall on touch) |
| Open / close | Click the tab, **E / I** (the existing inventory keys; open on the last-used tab, like gamepad Y), or drag the tab up/down. Open 0.18 s ease-out, close 0.15 s ease-in. The hotbar never moves; the panel slides up from behind it, clipped at the hotbar's top edge |
| Open height | Tab row + header + **3 grid rows** + the selection strip, capped at 60 % of the window height (fewer rows on short windows, never fewer than 2: 2 rows at 720 px, 3 from ~1080 px). The chips are one row that scrolls sideways (Shift+[ / ] cycles them); they never wrap |
| Grid | Slots after the hotbar's 8, in rows of 8. Chips/search/sort filter **only the grid rows**; the hotbar row is never filtered or re-sorted |
| Wardrobe / Craft | Tabs in the panel's top row. Switching replaces the area between the tab row and the hotbar; the hotbar stays. Wardrobe gets its own height (its preview must never scroll): the panel grows to fit it, up to 85 % of the window, with the same eased slide. Store stays a centre panel (B) |
| Drag and drop | Grid ↔ hotbar cell = `MoveItem` (swap), as today. Dragging a grid cell out of the panel onto the world drops it (inventory-categories.md); dragging from the hotbar row never drops |
| Closing | Esc, E/I, the tab, or opening the Store/Settings. Opening the bag closes a centre panel |

C (workbench.md) opens the bag on the Craft tab. Gamepad: **Y** opens the bag on its last tab (replacing workbench.md's
"Y opens craft": in the bag, LB/RB reach Craft).

## 2. Chat

- **Log:** bottom-left, its bottom edge 8 px above the top of whatever is highest in the bottom stack (the hotbar when
  closed, the bag's top edge when open), so it's **pushed up with the bag** and never overlaps it. Width ≤ 40 % of
  the window, max 420 px.
- Semi-transparent dark backing (`PANEL_COLOR`) only behind lines. The last 6 lines show for 10 s after arriving
  (today's behaviour), then fade over 0.5 s. When the bag is open, at most 4 lines show.
- **Input:** Enter opens the line at the log's bottom (TextFocus), Esc cancels; while typing, the full 50-line history
  shows and scrolls with the wheel / PgUp-PgDn.
- On short windows where the pushed-up log would reach the top bar, it shows 2 lines instead.

## 3. Everything else at the bottom

| Element | Closed | Open |
|---|---|---|
| Selection strip (item info, Drop/Trash when the bag is open) | Just above the pull tab | Inside the panel, between the grid and the hotbar (as drawn) |
| Toasts | Above the strip | Above the panel's top edge |
| First-run / one-time hints | Above the toasts | Above the toasts, over the panel's top edge |
| Voice "Hold V to talk" | Bottom-right, level with the hotbar | Same place; if the panel would cover it (narrow windows), above the panel's right edge |
| Chat | Bottom-left, above the hotbar | Pushed up above the panel |

Nothing may overlap in either state at 1280×720 and up; below that, chat shows fewer lines first, then toasts stack
higher.

## Input

- **Keyboard:** E/I toggle; 1–8 select hotbar slots in both states; arrows move in the grid while open; Enter is chat
  (or the focused control when the bag has focus).
- **Gamepad:** Y toggles; d-pad moves; A picks up/places in the grid (drag equivalent); B closes.
- **Touch:** tap the tab, or **swipe up on the hotbar** to open; swipe down on the tab row to close. All cells ≥ 44 px.

## Art (done by 6e, 32×16 each; flat bottom sits on the hotbar frame)

| Asset | Spec |
|---|---|
| `ui/pull_tab.png`, `ui/pull_tab_hover.png` | ~32×16, a small wood grip tab with an **up** arrow, wood kit palette |
| `ui/pull_tab_open.png`, `ui/pull_tab_open_hover.png` | The same tab with a **down** arrow |

The slid panel reuses `ui/panel.png` (9-slice), so no new frame.

## Do NOT build

- No floating inventory window anymore (it becomes the slid panel); no second hotbar row; no hotbar paging.
- No resizable bag, no pinned-open setting.
- Don't change the Store or Settings into slide panels.
