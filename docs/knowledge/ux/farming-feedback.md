# UX · Farming feedback

Spec for the implementer. Rules don't change. Everything is computed on the client from the mirrored `Tree`
(`item`, `planted_at` on the local clock, `fruit` rolled at planting, `spliced`) plus `farming::grow_seconds`,
`growth_stage` and `is_grown`. No new messages. All durations use the one `duration_text` in `hud.rs` (the same as
the U3 seed tooltip, so `3m 12s`, `3m`, `2h` read identically everywhere).

## 1. Plant and splice messages

| Event (local player only) | Toast |
|---|---|
| `TreePlanted` | `Planted <Item> seed · ready in <t>` (e.g. `Planted Planks seed · ready in 3m 12s`) |
| `TreeSpliced` | `<A> + <B> spliced into <Result> tree · ready in <t>` (replaces today's text, which has no time) |
| Planting fails (`NeedsGround`, claims) | the existing refusal copy (U2 / claims.md), unchanged |
| Harvest | no toast: the drops pop out of the tree and are visible |

Other players' plants and splices: no toast.

## 2. Tree card (hover)

Hovering any tree or sapling for 300 ms shows the shared tooltip box (`ui/tooltip.png`, item-tooltip.md), anchored
above the tree and kept on screen. Not limited by reach: looking is free.

```
┌─────────────────────────┐          ┌─────────────────────────┐
│ Planks tree             │          │ Planks tree             │
│ [██████░░░░░░]  58 %    │          │ [████████████]          │
│ 1m 20s left             │          │ Ready! · 3 Planks       │
└─────────────────────────┘          └─────────────────────────┘
       growing                              grown
```

| Line | Content |
|---|---|
| 1 | `<Item> tree`, plus ` · spliced` when `spliced` (it can't be spliced again) |
| 2 | Progress bar 120×8 px inside the box: dark groove, green fill, the fill updates once per second; `NN %` after it while growing |
| 3 | Growing: `<t> left`. Grown: `Ready! · <fruit> <Item>`. `fruit` is already known, so show the exact count, not a range |

- The card updates live while shown (time left counts down; at 0 it switches to the grown look).
- Hides when the pointer leaves, the tree is harvested or broken, or a panel opens.
- Card position never covers the tree itself.

## 3. At a glance in the world

- **Grown trees already show their item icon** above them (`world_view.rs`). Add a small twinkle on that icon:
  `fx/ready_sparkle.png` (4 frames, 16×16), played once every 4 s per tree, offset by tile position so a whole farm
  doesn't blink in sync. That's the "ready" signal: nothing else in the world.
- **No in-world progress bars.** Growth stages (six sprites) already show progress; the exact number is one hover
  away. A field of bars would clutter the screen.

## 4. Seed tooltip (U3) consistency

The seed tooltip line `Plant on soil · grows in <t>` uses the same `duration_text` and the same
`grow_seconds(rarity)` as the plant toast, so a seed that says `grows in 3m 12s` is `ready in 3m 12s` once planted.
A test asserts both strings come from the same function for one seed.

## 5. Input

- **Mouse:** hover as above.
- **Touch:** long-press a tree 400 ms opens the same card, pinned until tapping elsewhere. On tree and sapling tiles
  only, the punch fires on the release of a short tap (< 400 ms) instead of on press, so a long-press never damages
  the tree. Every other tile keeps punch-on-press (and hold-to-repeat).
- The percentage caps at 99 % while growing; grown shows `Ready!` instead of 100 %.
- **Keyboard / gamepad (no cursor):** the card appears for the tree directly in front of the player while standing
  within reach of it and not moving for 0.5 s; it hides on the first movement. Waits for the gamepad pass if the
  client has no cursorless mode yet.

## Art

| Asset | Spec |
|---|---|
| `fx/ready_sparkle.png` + `.json` | 4 × 16×16, a warm white/gold glint, ~10 fps, no loop, centre pivot; same json shape as `respawn_puff` (done) |
| Progress bar | drawn in code (two coloured rects), no art |

## Do NOT build

- No notifications when offline or away, no "your trees are ready" mail, no farm overview screen.
- No auto-harvest, no fertiliser/speed-ups, no per-tree ownership display.
- No toasts for other players' farming.
