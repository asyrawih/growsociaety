# UX · First-run hints (U15)

Spec for f0, in a new `client/game_client/src/hints.rs`. Progress is the existing `Settings::hints` (0..=3, the number
of steps finished), saved with the settings, so a returning player never sees a finished step again.

## Steps

One hint at a time, in order. A step is done when the player does the thing, not when they read it.

| `hints` | Hint text (desktop) | Touch text | Done when |
|---|---|---|---|
| 0 | `A / D to move · Space to jump` | `Use the stick to move · tap ↑ to jump` | The local player has moved ≥ 3 tiles from spawn **and** jumped once |
| 1 | `Hold left mouse on a block to punch it` | `Hold a block to punch it` | First successful `TileDamaged` or `TileBroken` by the local player |
| 2 | `Pick a seed in the hotbar and click soil to plant it` | `Pick a seed, then tap soil` | First `TreePlanted` by the local player |
| 3 | (none: finished) | | |

Key names come from the binding constants (`JUMP_KEYS`, `LEFT_KEYS`, `RIGHT_KEYS` in `player.rs`), first entry of
each, so the text follows any future rebinding.

## Look and place

```
                         ┌──────────────────────────────────────┐
                         │  A / D to move · Space to jump    1/3 │   ← centred, 24 px above the item strip
                         └──────────────────────────────────────┘
                [ hotbar .................................................. ]
```

- `ui/tooltip.png` 9-slice (inset 6), `INK_COLOR` text, 16 px,
  step counter `1/3` right-aligned in the same box.
- Appears 1 s after the world is shown (not over the loading/join moment). Fades in 0.3 s.
- On done: the box gets a short green check (`✓` replaces the counter for 0.6 s), fades out, next hint appears 1.5 s
  later. Play `ui_click` once on done; no other sound.
- Hidden while any panel (inventory, store, settings, chat input) is open; comes back when it closes.
- Never covers the hotbar, the item strip or toasts: toasts sit above it.

## Rules

- Only in the first session where `hints < 3`; works the same offline and online.
- Steps can complete out of order in practice (someone punches before walking 3 tiles): when a later step's condition
  happens early, remember it, and skip that step instantly when it comes up (it shows only its check, 0.6 s).
- `Settings > Controls` gets one button `Show tips again` that sets `hints = 0`.
- Players who never touch anything still see only one box; no stacking, no timers that nag.

## Do NOT build

- No tutorial world, arrows pointing at things, forced pauses or "Next" buttons.
- No hints for claims, crafting or trading here (claims has its own one-time tip in [claims.md](claims.md) §3).
- No analytics of hint completion for now.
