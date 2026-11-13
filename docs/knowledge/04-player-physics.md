# 04 · Player Physics

## Knowledge

Research found **no public numbers** for the reference game's movement. Everything here is our own design, labelled
Tuning, chosen so the character feels quick and precise on a 32 px grid. Expect to change the numbers in playtests;
do not expect to change the structure.

### Feel targets

| Target | Value | Why |
|---|---|---|
| Jump clears | 2 tiles, not 3 | A 2-high wall is climbable, a 3-high wall is a real barrier. Building has meaning |
| Reaches top speed in | ~0.1 s | Snappy; this is a building game, not a momentum platformer |
| Horizontal jump distance | ~6 tiles | A 5-wide gap is jumpable, a 7-wide gap is not |
| Fits through | 2-tile-high gap (D2) | The sprite is 1.5 tiles tall |

### Constants (all Tuning, in tiles; multiply by `TILE_SIZE` for px)

| Constant | Value | Notes |
|---|---|---|
| `TICK_RATE` | 60 Hz | Fixed timestep. Rule, not tuning: the server runs the same step |
| `WALK_SPEED` | 6 tiles/s | 192 px/s |
| `GROUND_ACCEL` | 60 tiles/s² | Top speed in 0.1 s |
| `AIR_ACCEL` | 30 tiles/s² | Half control in the air |
| `GRAVITY` | 60 tiles/s² | |
| `JUMP_VELOCITY` | 17.5 tiles/s | Apex = v² / 2g = 2.55 tiles in continuous maths; about 2.41 tiles as stepped at 60 Hz (gravity is applied before the move) |
| `JUMP_CUT` | 0.5 | Releasing jump while rising multiplies upward velocity by this (variable height) |
| `MAX_FALL_SPEED` | 24 tiles/s | 12.8 px per tick, see the tunnelling rule below |
| `COYOTE_TIME` | 0.1 s | Can still jump this long after walking off an edge |
| `JUMP_BUFFER` | 0.1 s | A jump pressed this long before landing still fires |
| `HITBOX` | 20 × 44 px | Centered horizontally on the sprite, bottom at the feet line |

### Hitbox and the sprite

The player sheet is 48×48 cells with the feet at `y = 44` from the top of the cell (`assets/README.md`). The hitbox is
narrower than a tile (20 px) so the player drops cleanly into 1-wide holes and does not snag on corners.

D2 is open: at 44 px tall the player cannot walk through a 1-tile-high tunnel and must dig 2 high. If the owner
wants 1-tile tunnels, the hitbox height becomes ≤ 30 px and the sprite's head overlaps the tile above. Keep the
height in one constant so this is a one-line change.

## Technical

### Where it lives

Physics is a **pure function in `game_shared`**, with no Bevy types:

```rust
#[derive(Clone, Copy, Default)]
pub struct Input { pub move_x: i8, pub jump_held: bool, pub jump_pressed: bool }

#[derive(Clone, Copy)]
pub struct Body {
    pub pos: (f32, f32),     // center of the hitbox, world px
    pub vel: (f32, f32),
    pub on_ground: bool,
    pub coyote: f32,
    pub jump_buffer: f32,
    pub jump_cut_ready: bool, // set when a jump fires; makes the jump cut apply once per jump
}

pub fn step(body: &mut Body, input: Input, world: &World)   // advances exactly 1 / TICK_RATE
```

Reason: in Phase 5 the server runs `step` to validate positions and the client runs the same `step` to predict.
If physics lived in a Bevy system the server would need Bevy, which the workspace forbids (`game_shared/Cargo.toml`).

The client calls `step` from `FixedUpdate` (set to 60 Hz) and interpolates the sprite between the last two bodies
for rendering. Do not put `delta_secs` into the simulation.

### Step order (one tick)

1. Timers: decrease `coyote` and `jump_buffer`; set `jump_buffer` if `jump_pressed`.
2. Horizontal velocity: move `vel.x` toward `move_x * WALK_SPEED` by `accel * dt` (ground or air accel).
3. Jump: if `jump_buffer > 0` and (`on_ground` or `coyote > 0`): `vel.y = JUMP_VELOCITY`, clear both timers.
4. Jump cut: if `jump_cut_ready` and `!jump_held` and `vel.y > 0`: `vel.y *= JUMP_CUT`, clear `jump_cut_ready`.
   A buffered jump that fires after the button was already released is cut on the same tick and becomes a short
   hop. That is intended: a tap gives a hop, a hold gives a full jump.
5. Gravity: `vel.y -= GRAVITY * dt`, clamp to `-MAX_FALL_SPEED`.
6. Move X, resolve against solid tiles.
7. Move Y, resolve against solid tiles; set `on_ground` if a downward move was blocked; refresh `coyote` when grounded.
8. Clamp to world bounds (the world edges are walls; below row 0 cannot be reached because of bedrock).

### AABB vs tiles

Resolve **one axis at a time**. This is the whole trick: it removes corner cases without any sweep maths.

```
move along X by vel.x * dt
  for every solid tile overlapping the hitbox:
     moving right -> hitbox.right = tile.left;  moving left -> hitbox.left = tile.right;  vel.x = 0
move along Y by vel.y * dt
  for every solid tile overlapping the hitbox:
     moving down  -> hitbox.bottom = tile.top, on_ground = true;  moving up -> hitbox.top = tile.bottom;  vel.y = 0
```

- Only test the tiles the hitbox overlaps (at most 2 × 3 for a 20×44 box), found with `world_to_tile` on its corners.
  Never loop the whole world.
- Use a small epsilon (0.01 px) when computing the overlapped tile range, so a box resting exactly on a tile top does
  not count as overlapping it.
- **Tunnelling rule:** movement per tick must stay below `TILE_SIZE`. With the constants above the maximum is
  12.8 px. Add a `const` assertion (`MAX_FALL_SPEED * TILE_SIZE / TICK_RATE < TILE_SIZE`) so a future tuning change
  that breaks this fails to compile instead of letting players fall through floors.
- "Solid" is read from the item table (`item(tile.fg).collision`), never from an id list.

### Animation (client only)

The player is a **code-driven rig**, not a frame sheet: six 48×48 parts in `assets/player_rig/` (head, torso, two
arms, two legs) rotated around their pivots by `client/game_client/src/player/rig.rs`. Clothing from `assets/items/`
is layered on the matching parts, and skin is recoloured from a palette ramp. What a player wears is the `Look`
component (`skin`, `items`); changing it re-dresses the rig.

| Motion | Condition | Pose |
|---|---|---|
| Idle | grounded, `|vel.x|` below the idle threshold | all parts at rest |
| Walk | grounded, moving | legs swing ±14° at 1.7 cycles/s, arms counter-swing, 1 px body bob |
| Air | not grounded | a frozen mid-stride |
| Punch (overlay) | an accepted punch, for `PUNCH_COOLDOWN` | front arm sweeps 0° → 95° → 0° over the swing |

The rig faces east; it is mirrored on X for west. Facing changes with `move_x` and is exposed as
`Player.facing_west` (the Drop action uses it for its side). The feet line of the rig sits on the hitbox bottom.

The rig is a view. It reads `Body`, facing and `PUNCH_COOLDOWN`; it never decides movement, range or cooldown.
The old sheets (`assets/player/player_east*.png`) are no longer drawn.

### Tests that must exist

- A body dropped above the ground lands with `on_ground = true` and `pos.y` exactly on the surface (within epsilon).
- Walking into a wall stops at the wall with zero penetration.
- A held jump from the ground peaks between 2.2 and 2.8 tiles, clears a 2-tile wall and cannot clear a 3-tile wall.
  The wall test is the real requirement; the window only catches gross tuning mistakes.
- A body falling from the top of the world at max speed never ends a tick inside a solid tile.
- Two runs with the same inputs produce bit-identical bodies (determinism).
