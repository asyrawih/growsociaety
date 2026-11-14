# UX · Almanac

Spec for f0 (new file, e.g. `client/game_client/src/almanac.rs`). An in-game book that explains everything in one
place. **Every recipe and item page is generated from the shared tables** (`items::ITEMS`, `farming::RECIPES`,
`crafting::RECIPES`, `claims::CLAIMS`, `equip_price`, `ITEM_MODS`, `farming::grow_seconds`). No hand-written lists,
so it can't go stale. Only the Basics text is written by hand, and its key names come from the binding constants.

## Open and navigate

- **J** opens/closes (unbound today; one `ALMANAC_KEY` constant). **F1** opens it on the Basics page.
- Top-bar button with the existing `ui/settings/icon_help.png`, tooltip `Almanac (J)`.
- Wide centre panel (`ui/panel.png`). A section list on the left, the page on the right, the search field on top.
- **Back** (`Backspace`, mouse back button, or a `‹ Back` link at the top of the page) returns to the previous page.
  The history holds 20 pages.
- Every item name or icon on any page is a **link** to that item's page (hover = underline + pointer).

```
┌ Almanac ─────────────────────────────────────────────── [x] ┐
│ [🔍 Search______________]                                    │
│ Basics        │ ‹ Back                                        │
│ Splicing      │ [icon] Planks          Block · rarity 4       │
│ Crafting      │ How to get:  Break it · Farm its seed ·       │
│ Items         │              Splice Dirt + Sand               │
│ Controls      │ Used in:     Workbench · Crate · Ladder · …   │
│               │ Seed: Planks Seed · grows in 1m 4s            │
└───────────────┴───────────────────────────────────────────────┘
```

## Sections

| Section | Content (source) |
|---|---|
| **Basics** | Short hand-written pages, 2–4 sentences each: Moving · Punching · Placing · Planting · Harvesting · Splicing · Claims · Trading · Respawning · Ladders & platforms · Crafting. Each links to the related pages (e.g. Splicing → the Splicing list) |
| **Splicing** | One row per `farming::RECIPES` entry: `[A] A + [B] B → [C] C` with C's grow time and rarity. Sorted by result rarity |
| **Crafting** | One row per `crafting::RECIPES` entry: station (`Anywhere` or `Workbench`), inputs with counts → output with count |
| **Items** | Every item except `EMPTY`/internal ones: icon, name, kind; filterable with the inventory chips (inventory-categories.md) |
| **Controls** | Generated from the key constants: one row per action (`Move  A / D`, `Jump  Space`, `Craft  C`, `Go home  hold R`…) |

### Item page (generated)

| Line | Source |
|---|---|
| Header | icon, name, kind, `rarity N` |
| How to get | every true source, each a link: `Break it` (blocks found in generated worlds), `Farm its seed` (`seed_of`), `Splice A + B` (a `farming::RECIPES` result), `Craft at <station>` (a `crafting::RECIPES` output), `Store · ◆ price` (`claims::CLAIMS` / `equip_price`) |
| Used in | recipes (splice or craft) that take it as an input, as links |
| Seed | `<Item> Seed · grows in <t>` when it has one; a seed's own page says `Grows: <Item>` |
| Mods | the `mod_label` line with mod icons, for equipment |
| Rules | one generated line where it applies: claims (`Protects the N tiles nearest it`), Workbench, platforms, ladders, lava (`Touching it sends you home`) |

## Search

Live, case-insensitive substring over item names, Basics page titles and recipe rows. Results are grouped
`Items · Recipes · Basics`, and Enter opens the first one. Uses `TextFocus`; Esc clears, then unfocuses, then closes.

## Discovered state

**Show everything from the start.** It's a reference book, and the owner asked for "every explanation in one place";
hiding recipes would bring back the "I don't know what this does" problem. If secrecy is wanted later, add a
`?` silhouette for splice results the player has never held (client-side, stored in settings) without changing the
layout.

## Input

- Keyboard: J / F1 open; arrows move the section list and rows; Enter follows a link; Backspace goes back; Esc closes.
- Gamepad: Select+Y opens (or the top-bar button through focus); d-pad moves; A follows; B goes back, then closes.
- Touch: the top-bar button; tap links; swipe right (or `‹ Back`) to go back; rows ≥ 44 px.

## Do NOT build

- No lore or story, no achievements, no external wiki links, no screenshots.
- No editing or player notes.
- No hand-maintained item or recipe lists anywhere in the code.
