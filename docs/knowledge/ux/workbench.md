# UX · Workbench discoverability

Problem (owner): "I don't know what the Workbench does." Today the only way in is a wrench click on a placed
Workbench, and nothing says so. The station rule (recipes need a Workbench within `CRAFT_STATION_REACH`) stays.

## Smallest fix: one key, two hints

1. **`C` opens the craft panel from anywhere** (and closes it). It is the bag's Craft tab, which already lists every
   recipe and greys the ones that need a Workbench. Inside the panel, **Enter / A crafts** (Enter is free now that
   chat respects `TextFocus`). The button label becomes `Craft (Enter)`. C no longer crafts, so a double-tap can't
   craft by accident.
2. **Wrench click on a Workbench keeps working** (same panel). Nothing else changes about clicking a Workbench: a
   plain click with the fist still punches it, so it can still be broken.
3. **Hover name tag** on a placed Workbench (like claims' name tag): `Workbench · press C to craft`.
4. **One-time hint** the first time the player places a Workbench (or first stands within reach of one):
   `Workbench nearby · press C to craft` above the item strip, stored in settings.
5. **Craft panel line**, shown when no Workbench is in reach and some recipes are greyed:
   `Stand near a Workbench to unlock the greyed recipes`. Hidden when one is in reach.
6. **Top bar:** the existing craft icon (`ui/icon_craft.png`) gets a button with tooltip `Craft (C)`, so touch has a
   way in without the bag.

## Text summary

| Where | Text |
|---|---|
| Item tooltip (U3), line 3 | `Crafting station · press C nearby` |
| Name tag (hover) | `Workbench · press C to craft` |
| One-time hint | `Workbench nearby · press C to craft` |
| Craft panel, no station | `Stand near a Workbench to unlock the greyed recipes` |
| Craft button | `Craft (Enter)`; disabled reasons unchanged (`Need 3 more Planks`, `Find a Workbench`) |
| Top-bar tooltip | `Craft (C)` |

## Input

- Keyboard: C opens/closes; arrows move; Enter crafts; `[` `]` switch bag tabs; Esc closes.
- Gamepad: Y opens the bag (bottom-hud.md), LB/RB to its Craft tab; A crafts; B closes.
- Touch: the top-bar Craft button; tap a recipe, tap Craft.

## Do NOT build

- No "interact" key that does different things on different blocks.
- No auto-opening the panel when walking up to a Workbench.
- No crafting queue or "craft all".
