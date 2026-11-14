# UX · Claims

Spec for 1a. Rules are in [07](../07-economy-and-locks.md) and do not change here; this is only what the player sees.
Merged with f0's draft (`claims-audit-draft.md`): area shown after a refusal, and online-player chips in the panel.
Reuse: `ui/panel.png`, `ui/close.png`, `ui/settings/toggle_on|off.png`, `ui/lobby/input.png`,
`ui/lobby/button_primary|secondary.png`, the HUD toast, the existing claim outline (`interact.rs`).
Names: `Net::name(id)` when online, else one `LookupNames` (cache the answer); show `Player #42` only while waiting.
Every message below is exact text. `<owner>` = owner name, `<n>` = tile count.

Gaps from d4's claim audit, renamed C1–C8 so they do not clash with the U-ids in [README](README.md):

| Id | Gap | Fixed in |
|---|---|---|
| C1 | No area shown after placing | §3 |
| C2 | No preview while holding a claim | §2 |
| C3 | Refused edits don't name the owner | §4 |
| C4 | Wrench on someone else's claim does nothing | §5 (read-only view) |
| C5 | `player 42` instead of names | names line above, §5 |
| C6 | No access UI | §5 |
| C7 | Silent `Occupied` / too-far | §2 labels, §4 table |
| C8 | Store doesn't explain area vs world claims | §6 |

## 1. Flow

```
buy in store ─► hold claim ─► hover: preview ─► click: placed ─► wrench on it: manage
                                   │ red = why not            │ toast + 3 s outline
stranger punches claimed tile ─► lock blink + area flash + toast naming owner
stranger wrenches a claim     ─► read-only info panel
```

## 2. Holding a claim (preview)

While a claim item is selected, the hovered tile shows the exact tiles it would take (`claims::claim_area`, pure and
the same function the server uses), drawn with the outline style at 30 % alpha.

| State | Look | Label next to the cursor (16 px) |
|---|---|---|
| Can place | Outline cyan | `Small claim · 10 tiles` (tier name + `<n>`) |
| Out of reach | Outline grey | `Too far` |
| Tile inside a claim | Outline red on that claim's area | `Already claimed` |
| No permission (inside someone's world claim) | Red | `This world belongs to <owner>` |
| World claim, can place | No outline; screen-edge cyan frame | `World claim · the whole world` |
| World claim, refused | Red frame | `Already claimed` / `Someone else has a claim here` |

The preview is the answer to "what does this cover". It replaces any text explanation in-world.

## 3. After placing

Toast: `Claim placed: <n> tiles are yours` (world: `World claimed: every tile is yours`). The outline stays 3 s and then
fades over 0.5 s. One-time hint on the first claim ever placed (stored in settings as `claim_tip_seen`):
`Tip: hold the wrench and click your claim to manage it`.

## 4. Refused edits (stranger)

On `NoPermission`: a lock icon blinks on the tile for 0.6 s (the claim sprite at 50 % size), that claim's area
flashes red for 1 s (so the player sees where they may not build), plus the toast `Locked by <owner>`. In a world
claim: `This world is locked by <owner>` and the screen-edge frame flashes red instead. Silent cases now get text:

| Reject | Toast |
|---|---|
| `Occupied` (placing a claim) | `That spot is already claimed` |
| `Occupied` (placing a claim, no free tile left) | `No free tiles left here` |
| `Occupied` (any other block) | none: the tile is visibly full |
| `OutOfRange` | none: the hover tint is already grey; never toast on every miss |
| `NotOwner` | `Only <owner> can do that` |

At most one claim toast per 2 s, so holding the punch key does not spam.

## 5. Wrench on a claim: the claim panel

Opens top-right under the top bar (where it is now). Esc, the close button or putting the wrench away closes it.
Turning Public on asks for confirmation every time (rare and risky); turning it off never asks.
The claim's tiles are outlined while it is open: cyan if you can build there, red if not.

```
┌ Claim ──────────────────── [x] ┐      Stranger view (read-only):
│ Owner      Ana                 │      ┌ Claim ─────────────── [x] ┐
│ Size       48 tiles            │      │ Owner   Ana                │
│ Public     [toggle]            │      │ Size    48 tiles           │
│ ─────────────────────────────  │      │ You can't build here       │
│ Can build (2/32)               │      │  (or: You can build here)  │
│  Budi                    [x]   │      └────────────────────────────┘
│  Cici                    [x]   │
│ In this world: (Dodi) (Eka)    │  ← chips: players here, not yet on the list
│ [ name________ ] [ Add ]       │
└────────────────────────────────┘
```

| Element | States |
|---|---|
| Public toggle | on / off; hover = toggle hover. Turning it on asks: `Everyone can build here. Turn on?` [Yes] [No] |
| Access row `[x]` | hover = red tint; click removes at once (`SetClaimAccess allow=false`), no confirm |
| Player chips | one per player in the world who is neither owner nor listed; click = add; hidden when none |
| Name field | empty: placeholder `Player name`; Enter = Add. For players not in the world |
| Add button | disabled while the field is empty or the list holds 32; label `Add` |
| Add errors (under the field, red) | `No player called <name>` · `<name> can already build here` · `You are the owner` · `The list is full (32)` |
| Add success | the row appears, the field clears, no toast |

Access names come from the same name lookup; a row shows `Player #42` until resolved.

## 6. Store (claims tab)

Each row: icon, name, price, one line that says what it covers:

| Item | Line |
|---|---|
| Small / Medium / Large claim | `Protects the 10 / 48 / 200 tiles nearest it` |
| World claim | `Protects the whole world. Only if no one else has a claim here` |

Rows you cannot afford: greyed as now. Hover a row: `Hold it in the world to see the area first`.

## 7. Discoverability

- The store line and the preview teach coverage; the first-placement tip teaches the wrench.
- Hovering any claim block shows a name tag above it: `<owner>'s claim` (no wrench needed).
- `/access` stays for power users; the panel is the main path.

## 8. Input

- **Keyboard:** the panel opens with **no** focus, so walking keeps working. Clicking the name field (or Tab) focuses
  it and the keyboard then types into it; Esc, Tab or a click outside gives the keyboard back. Tab cycles toggle →
  rows → chips → field → Add. Enter in the field = Add once chat respects a text-focus flag; until then the Add
  button only. Esc with nothing focused closes the panel.
- **Gamepad:** same order with the d-pad; A activates, B closes. Focus highlight = `slot_selected.png` frame.
- **Mobile:** tap with the wrench opens the panel; rows, chips and buttons ≥ 44 px tall; the preview label sits above
  the finger, not under it.

## 9. Do NOT build

- No tint on claimed tiles during normal play (07: claims stop editing, not visiting). Areas show only while holding
  a claim (preview), while holding the wrench over a claimed tile (that claim, cyan if you can build, red if not; for a world claim
  the screen-edge frame at alpha 0.3, fainter than the 0.8 placement preview: hover is passive info),
  while its panel is open, after placing, and after a refusal.
- No transfer, rename, kick/ban list, per-tile permissions or claim history.
- No confirm dialog for removing access (instant and reversible).
