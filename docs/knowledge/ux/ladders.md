# UX · Ladders

Spec for the implementer: climbing in `game_shared::physics` (client prediction and server identical). The item
exists (`LADDER`, id 33, `Collision::Ladder`, Workbench recipe Planks ×6 → ×4); only the movement is missing.

## Input

`physics::Input` gets `up: bool` next to platforms' `down: bool`.

| Device | Up / Down |
|---|---|
| Keyboard | `W` / `↑` = up, `S` / `↓` = down. **Space** = jump. Off a ladder, W and ↑ still jump as today |
| Gamepad | Left stick or d-pad up/down past 60 %; A = jump |
| Touch | `↑` and `↓` buttons by the walk pad, shown only while overlapping a ladder (same context button as platforms' `↓`) |

## Rules

| Situation | Result |
|---|---|
| Overlapping a ladder (hitbox centre inside a ladder tile) and pressing up or down | **Grab**: velocity set to zero, gravity off. Walking past a ladder without pressing up/down does not grab |
| Falling past a ladder and pressing up or down | Grab mid-fall (same rule; this is how you catch yourself) |
| Climbing | Up 4 tiles/s, down 6 tiles/s, no input = hang still. Left/right moves at half walk speed |
| Moving sideways off the ladder column | Let go: normal gravity again |
| Jump (Space / A) while climbing | Normal jump from the ladder; lets go |
| Top of a ladder | The top edge of the topmost ladder tile works like a one-way platform: climbing up past it lands you standing on top. Down while standing there grabs and climbs down |
| Bottom | Climbing down into solid ground stops you; you stand normally |
| Platforms | Ignored while climbing, so a ladder can run through a platform floor (platforms.md) |
| Wings: extra jump | Grabbing a ladder refills the extra jump, like landing |
| Glide | Not while climbing; available again after letting go |
| Hazards | Unchanged: touching one while climbing respawns (respawn.md) |

All speeds are Tuning constants next to `WALK_SPEED` and stay under one tile per tick (the existing const asserts).

## Feel and visuals

- No new art. `player/rig.rs` gets a `Motion::Climb` pose: both arms up, hands alternating at 6 Hz while moving,
  still while hanging. The body faces the screen while climbing.
- Ladder hover tooltip (U3) line 3: `Ladder · climb with W / S`.
- The first time the player overlaps a ladder: one-time hint above the item strip, `W / S to climb · Space to jump off`
  (stored in settings like the other hints).

## Tests that must exist

- Pressing up on a ladder rises at the climb speed; no input keeps the height.
- Walking through a ladder tile without up/down does not grab.
- Grabbing mid-fall stops the fall in the same tick.
- Climbing up past the top lands standing on the top edge; Down from there climbs back down.
- Jump from a ladder gives a normal jump; the extra jump is available after a grab.
- Client and server steps with `up`/`down` stay bit-identical.

## Do NOT build

- No ropes, vines or chains, no sideways ladders, no auto-climb, no climbing on walls.
