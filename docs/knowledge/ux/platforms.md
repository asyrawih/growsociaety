# UX · One-way platforms

Spec for the implementer (physics in `game_shared::physics`, so client prediction and server stay identical).
Uses the existing `Collision::Platform` (unused today). Claims, drops and inventory rules do not change.

## Feel

A platform is a thin plank you can jump up through and stand on, and drop through on purpose.

| Situation | Result |
|---|---|
| Moving up (jumping) into it from below | Pass through. No head bump |
| Walking into it from the side | Pass through |
| Falling onto it, feet were at or above its top edge on the previous tick | Land on its top edge, exactly like ground (coyote time, jump buffer and landing all work) |
| Standing on it, press **Down** | Drop through (below) |
| Glide (holding jump while falling) | Lands on platforms normally; gliding does not pass through |
| Extra jump (wings) | Passes up through platforms like a normal jump |
| Ladders | Climbing isn't in physics yet. When it is: while climbing, platforms are ignored, so a ladder can run through a platform floor |
| A platform is placed where a player stands | Nothing happens. It only catches feet coming from above, so it can never trap or push anyone |

**Drop-through, tap vs hold:**
- **Tap Down** (pressed while standing on a platform): pass through the platform you stand on only. Implementation
  hint: remember the top-edge y you dropped from and ignore platforms with that top until the feet are below it.
- **Hold Down**: ignore every platform while held, so you fall through a whole stack of floors.
- Down on solid ground or in the air does nothing.

**Input:** add `down: bool` to `physics::Input` (one more field; sent with every input like `jump_held`).

Gamepad and touch rows wait for the client's gamepad-movement and touch-control layers (not built yet); ship
keyboard first.

| Device | Down |
|---|---|
| Keyboard | `S` or `↓` (both free today; keep them in one `DOWN_KEYS` constant next to `JUMP_KEYS`) |
| Gamepad | D-pad down, or the left stick pushed down past 60 % |
| Touch | A `↓` button under the walk pad, shown only while standing on a platform (it doubles as the hint that you can drop) |

## Placing and breaking

- Placed like any block, in mid-air too (doc 05: placement needs no neighbour).
- Claim permission unchanged: `can_edit` decides, as for every block.
- Breaking: punches below; drops itself like other crafted blocks; `NO_SEED` (crafted only, like the Workbench).
- Platforms are not solid for seeds: a seed on a platform is `NeedsGround` (trees need real soil).

## Items and recipes

Two original items, both crafted at the Workbench (doc 18 table style). Ids: the next free block ids.

| Item | Collision | Fist hits | Recipe (Workbench) | Output |
|---|---|---|---|---|
| Wood Platform | `Platform` | 2 | Planks ×2 | ×4 |
| Stone Platform | `Platform` | 5 | Stone Bricks ×2 + Planks ×1 | ×4 |

Wood is the cheap early one; stone costs a little more and takes longer to break, which matters in claimed builds.
Four per craft so a floor is affordable.

## Visual cue

A platform must never be mistaken for a solid block:

- Sprite: 32×32 tile, only the **top 8 px** drawn (the plank or slab), two small brackets underneath at the ends,
  the rest transparent, so the background and the player behind it show through.
- A 1 px lighter highlight along the top edge, so the walkable surface reads at a glance.
- Item icon: the same sprite (it already reads as "thin").
- Art is done: `assets/blocks/wood_platform.png`, `assets/blocks/stone_platform.png`. When the item ids are assigned,
  rename them to the `NN_name.png` pattern (e.g. `68_wood_platform.png`) in the same change.
- Optional, cheap: while standing on one, the touch `↓` button is the only extra UI. No floating hints.

## Tests that must exist

- Jumping up from below through a platform ends above it; falling back lands on its top.
- Walking sideways through a platform tile does not stop the player.
- Tap Down drops through exactly one of two stacked platforms; hold Down drops through both.
- Down on solid ground changes nothing. A platform placed inside a player does not move them.
- Client and server steps with `down` stay bit-identical (existing determinism test extended).

## Do NOT build

- No moving or crumbling platforms, no slopes, no "down + jump" combo (Down alone is enough).
- No platform walls or ceilings: a platform is only ever a floor.
- No special collision for dropped items yet (drops already ignore physics).
