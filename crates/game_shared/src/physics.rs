//! Player movement as a pure function. Client and server run the same `step` (docs/knowledge/04).

use crate::items::{Collision, item};
use crate::mods::{EXTRA_JUMP_VELOCITY, GLIDE_FALL_SPEED, Mods};
use crate::world::World;
use crate::{TILE_SIZE, WORLD_HEIGHT, WORLD_WIDTH, tile_to_world};

/// Fixed timestep in Hz. Rule, not tuning: the server runs the same step.
pub const TICK_RATE: f32 = 60.0;
const DT: f32 = 1.0 / TICK_RATE;

// Tuning. Written in tiles, stored in px.
pub const WALK_SPEED: f32 = 6.0 * TILE_SIZE;
pub const GROUND_ACCEL: f32 = 60.0 * TILE_SIZE;
pub const AIR_ACCEL: f32 = 30.0 * TILE_SIZE;
pub const GRAVITY: f32 = 60.0 * TILE_SIZE;
pub const JUMP_VELOCITY: f32 = 17.5 * TILE_SIZE;
/// Releasing jump while rising multiplies the upward velocity by this, once per jump.
pub const JUMP_CUT: f32 = 0.5;
pub const MAX_FALL_SPEED: f32 = 24.0 * TILE_SIZE;
/// Ladders (docs/knowledge/ux/ladders.md).
pub const CLIMB_UP_SPEED: f32 = 4.0 * TILE_SIZE;
pub const CLIMB_DOWN_SPEED: f32 = 6.0 * TILE_SIZE;
pub const CLIMB_SIDE_SPEED: f32 = WALK_SPEED / 2.0;
/// Seconds.
pub const COYOTE_TIME: f32 = 0.1;
/// Seconds.
pub const JUMP_BUFFER: f32 = 0.1;
/// The biggest RunBoost in `mods::ITEM_MODS`, for the tunnelling check below.
const MAX_RUN_PCT: f32 = 20.0;

/// Px. Open decision D2: change the height here for 1-tile tunnels.
pub const HITBOX_WIDTH: f32 = 20.0;
pub const HITBOX_HEIGHT: f32 = 44.0;
/// Px. A box resting exactly on a tile edge does not count as overlapping that tile.
const COLLISION_EPSILON: f32 = 0.01;

const HALF_W: f32 = HITBOX_WIDTH / 2.0;
const HALF_H: f32 = HITBOX_HEIGHT / 2.0;

// Tunnelling rule: movement per tick must stay below one tile, or bodies pass through floors and walls.
const _: () = assert!(MAX_FALL_SPEED / TICK_RATE < TILE_SIZE);
const _: () = assert!(JUMP_VELOCITY / TICK_RATE < TILE_SIZE);
const _: () = assert!(WALK_SPEED / TICK_RATE < TILE_SIZE);
const _: () = assert!(WALK_SPEED * (1.0 + MAX_RUN_PCT / 100.0) / TICK_RATE < TILE_SIZE);
const _: () = assert!(EXTRA_JUMP_VELOCITY / TICK_RATE < TILE_SIZE);
const _: () = assert!(CLIMB_DOWN_SPEED / TICK_RATE < TILE_SIZE);
const _: () = assert!(CLIMB_UP_SPEED / TICK_RATE < TILE_SIZE);

#[derive(Clone, Copy, Default, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Input {
    /// Sign only: anything below zero is left, above zero is right.
    pub move_x: i8,
    pub jump_held: bool,
    pub jump_pressed: bool,
    /// Held: fall through every one-way platform; pressed on one, drop through it
    /// (docs/knowledge/ux/platforms.md).
    pub down: bool,
    /// Held: climb up a ladder (docs/knowledge/ux/ladders.md).
    pub up: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Body {
    /// Centre of the hitbox, world px.
    pub pos: (f32, f32),
    pub vel: (f32, f32),
    pub on_ground: bool,
    pub coyote: f32,
    pub jump_buffer: f32,
    /// Rising from a jump that has not been cut yet; makes the jump cut apply once per jump.
    pub jump_cut_ready: bool,
    /// The ExtraJump mod's one mid-air jump is spent; cleared on landing.
    pub extra_jump_used: bool,
    /// Ticks left during which hazards are ignored, set by every respawn
    /// (docs/knowledge/ux/respawn.md: a door next to lava cannot loop).
    pub hazard_immune: u8,
    /// Holding on to a ladder: no gravity, moved by up/down (docs/knowledge/ux/ladders.md).
    pub climbing: bool,
}

/// Hazard immunity after a respawn: 0.5 s. Tuning.
pub const HAZARD_IMMUNE_TICKS: u8 = (TICK_RATE * 0.5) as u8;

/// A body at rest on the main door tile: centred on it, hitbox bottom on the tile's bottom edge.
pub fn spawn_body(world: &World) -> Body {
    let (x, y) = tile_to_world(world.door.0, world.door.1);
    Body {
        pos: (x, y - TILE_SIZE / 2.0 + HALF_H),
        vel: (0.0, 0.0),
        on_ground: false,
        coyote: 0.0,
        jump_buffer: 0.0,
        jump_cut_ready: false,
        extra_jump_used: false,
        hazard_immune: HAZARD_IMMUNE_TICKS,
        climbing: false,
    }
}

fn hazard(world: &World, tx: i32, ty: i32) -> bool {
    let tile = u32::try_from(tx)
        .ok()
        .zip(u32::try_from(ty).ok())
        .and_then(|(x, y)| world.get(x, y));
    tile.and_then(|tile| item(tile.fg))
        .is_some_and(|def| def.collision == Collision::Hazard)
}

fn platform(world: &World, tx: i32, ty: i32) -> bool {
    let tile = u32::try_from(tx)
        .ok()
        .zip(u32::try_from(ty).ok())
        .and_then(|(x, y)| world.get(x, y));
    tile.and_then(|tile| item(tile.fg))
        .is_some_and(|def| def.collision == Collision::Platform)
}

fn ladder(world: &World, tx: i32, ty: i32) -> bool {
    let tile = u32::try_from(tx)
        .ok()
        .zip(u32::try_from(ty).ok())
        .and_then(|(x, y)| world.get(x, y));
    tile.and_then(|tile| item(tile.fg))
        .is_some_and(|def| def.collision == Collision::Ladder)
}

/// The topmost tile of a ladder: its top edge holds feet like a one-way platform.
fn ladder_top(world: &World, tx: i32, ty: i32) -> bool {
    ladder(world, tx, ty) && !ladder(world, tx, ty + 1)
}

fn tile_index(px: f32) -> i32 {
    (px / TILE_SIZE).floor() as i32
}

/// Whether the hitbox centre is inside a ladder tile: where a body can grab one.
pub fn on_ladder(world: &World, pos: (f32, f32)) -> bool {
    ladder(world, tile_index(pos.0), tile_index(pos.1))
}

/// Whether a ladder tile in the centre's column overlaps the hitbox: where a climber holds on.
fn holds_ladder(world: &World, pos: (f32, f32)) -> bool {
    let column = tile_index(pos.0);
    tile_span(pos.1 - HALF_H, pos.1 + HALF_H).any(|ty| ladder(world, column, ty))
}

fn solid(world: &World, tx: i32, ty: i32) -> bool {
    let tile = u32::try_from(tx)
        .ok()
        .zip(u32::try_from(ty).ok())
        .and_then(|(x, y)| world.get(x, y));
    tile.and_then(|tile| item(tile.fg))
        .is_some_and(|def| def.collision == Collision::Solid)
}

/// Tile indices overlapped by the span `lo..hi` on one axis.
fn tile_span(lo: f32, hi: f32) -> std::ops::RangeInclusive<i32> {
    let index = |px: f32| (px / TILE_SIZE).floor() as i32;
    index(lo + COLLISION_EPSILON)..=index(hi - COLLISION_EPSILON)
}

/// Solid tiles the hitbox centred on `pos` overlaps. At most 2 x 3 tiles are tested.
fn overlapped_solids(world: &World, pos: (f32, f32)) -> impl Iterator<Item = (i32, i32)> + '_ {
    let columns = tile_span(pos.0 - HALF_W, pos.0 + HALF_W);
    let rows = tile_span(pos.1 - HALF_H, pos.1 + HALF_H);
    rows.flat_map(move |ty| columns.clone().map(move |tx| (tx, ty)))
        .filter(move |&(tx, ty)| solid(world, tx, ty))
}

/// Whether the hitbox centred on `pos` overlaps tile `(x, y)`. Touching edges do not count.
pub fn hitbox_overlaps_tile(pos: (f32, f32), x: u32, y: u32) -> bool {
    let within = |span: std::ops::RangeInclusive<i32>, tile: u32| {
        i32::try_from(tile).is_ok_and(|tile| span.contains(&tile))
    };
    within(tile_span(pos.0 - HALF_W, pos.0 + HALF_W), x)
        && within(tile_span(pos.1 - HALF_H, pos.1 + HALF_H), y)
}

/// Whether the hitbox centred on `pos` contains the world point `point`.
pub fn hitbox_contains(pos: (f32, f32), point: (f32, f32)) -> bool {
    (point.0 - pos.0).abs() <= HALF_W && (point.1 - pos.1).abs() <= HALF_H
}

fn approach(value: f32, target: f32, max_delta: f32) -> f32 {
    value + (target - value).clamp(-max_delta, max_delta)
}

/// Advances `body` by exactly `1 / TICK_RATE` seconds, with no item mods. `Some(where)` if it
/// touched a hazard there and was respawned at the main door.
pub fn step(body: &mut Body, input: Input, world: &World) -> Option<(f32, f32)> {
    step_with(body, input, world, Mods::default())
}

/// `step` with the movement mods of what the player wears (`mods::worn_mods`). The client's
/// prediction and the server must pass the same mods, so reconciliation stays exact.
pub fn step_with(body: &mut Body, input: Input, world: &World, mods: Mods) -> Option<(f32, f32)> {
    // 1. Timers.
    body.coyote = (body.coyote - DT).max(0.0);
    body.jump_buffer = (body.jump_buffer - DT).max(0.0);
    if input.jump_pressed {
        body.jump_buffer = JUMP_BUFFER;
    }

    // Ladders. Grab: up with the centre in a ladder, or down with a ladder under the feet (so
    // down on the top climbs down, and down at the bottom, on the ground, does not grab). Grabbing
    // stops the body and refills the extra jump, like landing. A jump lets go with a normal jump.
    if body.climbing && !holds_ladder(world, body.pos) {
        body.climbing = false;
    }
    let below_feet = tile_index(body.pos.1 - HALF_H - COLLISION_EPSILON);
    if !body.climbing
        && ((input.up && on_ladder(world, body.pos))
            || (input.down && ladder(world, tile_index(body.pos.0), below_feet)))
    {
        body.climbing = true;
        body.vel = (0.0, 0.0);
        body.extra_jump_used = false;
        body.jump_cut_ready = false;
    }
    if body.climbing && body.jump_buffer > 0.0 {
        body.climbing = false;
        body.vel.1 = JUMP_VELOCITY;
        body.jump_buffer = 0.0;
        body.jump_cut_ready = true;
    }
    if body.climbing {
        body.vel.0 = f32::from(input.move_x.signum()) * CLIMB_SIDE_SPEED;
        body.vel.1 = match (input.up, input.down) {
            (true, false) => CLIMB_UP_SPEED,
            (false, true) => -CLIMB_DOWN_SPEED,
            _ => 0.0,
        };
    } else {
        fall(body, input, mods);
    }
    move_and_collide(body, input, world)
}

/// Walking, jumping and gravity: steps 2 to 5 of `step_with`, skipped while climbing.
fn fall(body: &mut Body, input: Input, mods: Mods) {
    // 2. Horizontal velocity.
    let accel = if body.on_ground {
        GROUND_ACCEL
    } else {
        AIR_ACCEL
    };
    let speed = WALK_SPEED * (1.0 + f32::from(mods.run_pct) / 100.0);
    let target = f32::from(input.move_x.signum()) * speed;
    body.vel.0 = approach(body.vel.0, target, accel * DT);

    // 3. Jump.
    if body.jump_buffer > 0.0 && (body.on_ground || body.coyote > 0.0) {
        body.vel.1 = JUMP_VELOCITY;
        body.coyote = 0.0;
        body.jump_buffer = 0.0;
        body.jump_cut_ready = true;
    } else if input.jump_pressed && mods.extra_jump && !body.extra_jump_used {
        body.vel.1 = EXTRA_JUMP_VELOCITY;
        body.jump_buffer = 0.0;
        body.extra_jump_used = true;
        body.jump_cut_ready = true;
    }

    // 4. Jump cut.
    if body.vel.1 <= 0.0 {
        body.jump_cut_ready = false;
    } else if body.jump_cut_ready && !input.jump_held {
        body.vel.1 *= JUMP_CUT;
        body.jump_cut_ready = false;
    }

    // 5. Gravity.
    let max_fall = if mods.glide && input.jump_held {
        GLIDE_FALL_SPEED
    } else {
        MAX_FALL_SPEED
    };
    body.vel.1 = (body.vel.1 - GRAVITY * DT).max(-max_fall);
}

/// Steps 6 to 9 of `step_with`: move by the velocity, collide, bounds, hazards.
fn move_and_collide(body: &mut Body, input: Input, world: &World) -> Option<(f32, f32)> {
    // 6. Move X, resolve.
    body.pos.0 += body.vel.0 * DT;
    let hit = if body.vel.0 > 0.0 {
        overlapped_solids(world, body.pos)
            .map(|(tx, _)| tx as f32 * TILE_SIZE - HALF_W)
            .reduce(f32::min)
    } else if body.vel.0 < 0.0 {
        overlapped_solids(world, body.pos)
            .map(|(tx, _)| (tx + 1) as f32 * TILE_SIZE + HALF_W)
            .reduce(f32::max)
    } else {
        None
    };
    if let Some(x) = hit {
        body.pos.0 = x;
        body.vel.0 = 0.0;
    }

    // 7. Move Y, resolve. One-way platforms only catch feet that were at or above their top edge
    // a tick ago, so jumping up through them and walking through them just works. Down ignores
    // them for that tick: a tap drops through the one stood on (a tick later the feet are below
    // it, so it no longer catches), holding falls through a whole stack.
    let feet_before = body.pos.1 - HALF_H;
    body.pos.1 += body.vel.1 * DT;
    body.on_ground = false;
    let feet = body.pos.1 - HALF_H;
    if body.vel.1 < 0.0 {
        let solid_floor = overlapped_solids(world, body.pos)
            .map(|(_, ty)| (ty + 1) as f32 * TILE_SIZE + HALF_H)
            .reduce(f32::max);
        // Climbers pass through platforms and ladder tops (a ladder can run through a floor).
        let platform_floor = if input.down || body.climbing {
            None
        } else {
            let columns = tile_span(body.pos.0 - HALF_W, body.pos.0 + HALF_W);
            tile_span(feet, body.pos.1 + HALF_H)
                .filter(|&ty| {
                    columns
                        .clone()
                        .any(|tx| platform(world, tx, ty) || ladder_top(world, tx, ty))
                })
                .map(|ty| (ty + 1) as f32 * TILE_SIZE)
                .filter(|&top| feet_before >= top - COLLISION_EPSILON && feet < top)
                .map(|top| top + HALF_H)
                .reduce(f32::max)
        };
        let floor = match (solid_floor, platform_floor) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if let Some(y) = floor {
            body.pos.1 = y;
            body.vel.1 = 0.0;
            body.on_ground = true;
        }
    } else if body.vel.1 > 0.0 {
        let ceiling = overlapped_solids(world, body.pos)
            .map(|(_, ty)| ty as f32 * TILE_SIZE - HALF_H)
            .reduce(f32::min);
        if let Some(y) = ceiling {
            body.pos.1 = y;
            body.vel.1 = 0.0;
        }
    }

    // 8. World bounds: the edges are walls, the bottom is a floor.
    let (max_x, max_y) = (
        WORLD_WIDTH as f32 * TILE_SIZE - HALF_W,
        WORLD_HEIGHT as f32 * TILE_SIZE - HALF_H,
    );
    if body.pos.0 < HALF_W || body.pos.0 > max_x {
        body.pos.0 = body.pos.0.clamp(HALF_W, max_x);
        body.vel.0 = 0.0;
    }
    if body.pos.1 < HALF_H || body.pos.1 > max_y {
        body.on_ground |= body.pos.1 < HALF_H;
        body.pos.1 = body.pos.1.clamp(HALF_H, max_y);
        body.vel.1 = 0.0;
    }

    // Climbing down onto the ground ends the climb: the body stands.
    if body.on_ground && !input.up {
        body.climbing = false;
    }
    if body.on_ground {
        body.coyote = COYOTE_TIME;
        body.extra_jump_used = false;
    }

    // 9. Hazards: touching one sends the body back to the main door (docs/knowledge/ux/respawn.md).
    if body.hazard_immune > 0 {
        body.hazard_immune -= 1;
        return None;
    }
    let columns = tile_span(body.pos.0 - HALF_W, body.pos.0 + HALF_W);
    let touched = tile_span(body.pos.1 - HALF_H, body.pos.1 + HALF_H)
        .any(|ty| columns.clone().any(|tx| hazard(world, tx, ty)));
    if !touched {
        return None;
    }
    let burned_at = body.pos;
    *body = spawn_body(world);
    Some(burned_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{BEDROCK, EMPTY};
    use crate::rng::Rng;
    use crate::world::Tile;
    use crate::worldgen::{SURFACE_ROW, generate};

    const FLOOR_TOP: f32 = TILE_SIZE;
    const HOLD_RIGHT: Input = Input {
        move_x: 1,
        jump_held: false,
        jump_pressed: false,
        down: false,
        up: false,
    };
    const HOLD_JUMP: Input = Input {
        move_x: 0,
        jump_held: true,
        jump_pressed: false,
        down: false,
        up: false,
    };

    fn put(world: &mut World, x: u32, y: u32) {
        assert!(world.set(
            x,
            y,
            Tile {
                fg: BEDROCK,
                bg: EMPTY
            }
        ));
    }

    /// Empty world with a one-tile floor on row 0.
    fn flat_world() -> World {
        let mut world = World::empty("T".into());
        for x in 0..WORLD_WIDTH {
            put(&mut world, x, 0);
        }
        world
    }

    fn body_at(x: f32, feet: f32) -> Body {
        Body {
            pos: (x, feet + HALF_H),
            ..spawn_body(&World::empty("T".into()))
        }
    }

    /// A body that has settled on the flat floor at `x`.
    fn standing(world: &World, x: f32) -> Body {
        let mut body = body_at(x, FLOOR_TOP);
        step(&mut body, Input::default(), world);
        assert!(body.on_ground);
        body
    }

    fn feet(body: &Body) -> f32 {
        body.pos.1 - HALF_H
    }

    fn inside_solid(world: &World, body: &Body) -> bool {
        overlapped_solids(world, body.pos).next().is_some()
    }

    #[test]
    fn dropped_body_lands_exactly_on_the_surface() {
        let world = flat_world();
        let mut body = body_at(100.0, FLOOR_TOP + 150.3);
        for _ in 0..120 {
            step(&mut body, Input::default(), &world);
        }
        assert!(body.on_ground);
        assert!(
            (feet(&body) - FLOOR_TOP).abs() < COLLISION_EPSILON,
            "{body:?}"
        );
        assert_eq!(body.vel, (0.0, 0.0));
    }

    #[test]
    fn walking_into_a_wall_stops_at_the_wall() {
        let mut world = flat_world();
        for y in 1..=3 {
            put(&mut world, 10, y);
            put(&mut world, 3, y);
        }
        let (right_wall, left_wall) = (10.0 * TILE_SIZE, 4.0 * TILE_SIZE);

        let mut body = standing(&world, 6.5 * TILE_SIZE);
        for _ in 0..120 {
            step(&mut body, HOLD_RIGHT, &world);
            assert!(body.pos.0 + HALF_W <= right_wall, "{body:?}");
        }
        assert!(right_wall - (body.pos.0 + HALF_W) < COLLISION_EPSILON);

        let hold_left = Input {
            move_x: -1,
            ..HOLD_RIGHT
        };
        for _ in 0..120 {
            step(&mut body, hold_left, &world);
            assert!(body.pos.0 - HALF_W >= left_wall, "{body:?}");
        }
        assert!(body.pos.0 - HALF_W - left_wall < COLLISION_EPSILON);
        assert!(body.on_ground);
    }

    #[test]
    fn walk_reaches_top_speed_and_out_of_range_move_x_is_clamped() {
        let world = flat_world();
        let mut body = standing(&world, 5.0 * TILE_SIZE);
        let sprint_hack = Input {
            move_x: i8::MAX,
            ..HOLD_RIGHT
        };
        for _ in 0..30 {
            step(&mut body, sprint_hack, &world);
            assert!(body.vel.0 <= WALK_SPEED);
        }
        assert_eq!(body.vel.0, WALK_SPEED);
    }

    /// Peak height of the feet above the floor, in tiles, for a jump with the button held throughout.
    fn jump_peak(world: &World, x: f32, held: Input) -> (f32, Body) {
        let mut body = standing(world, x);
        step(
            &mut body,
            Input {
                jump_pressed: true,
                ..held
            },
            world,
        );
        let mut peak = feet(&body);
        for _ in 0..120 {
            step(&mut body, held, world);
            peak = peak.max(feet(&body));
        }
        ((peak - FLOOR_TOP) / TILE_SIZE, body)
    }

    #[test]
    fn jump_peaks_between_2_2_and_2_8_tiles() {
        let (peak, body) = jump_peak(&flat_world(), 5.0 * TILE_SIZE, HOLD_JUMP);
        assert!((2.2..=2.8).contains(&peak), "peak {peak}");
        assert!(body.on_ground);
    }

    #[test]
    fn releasing_jump_cuts_the_height_once() {
        let world = flat_world();
        let (full, _) = jump_peak(&world, 5.0 * TILE_SIZE, HOLD_JUMP);
        let (cut, _) = jump_peak(&world, 5.0 * TILE_SIZE, Input::default());
        // Halving the launch speed once quarters the height; halving every tick would leave almost none.
        assert!(
            cut > full * 0.2 && cut < full * 0.3,
            "full {full} cut {cut}"
        );
    }

    #[test]
    fn jump_clears_a_2_tile_wall_but_not_a_3_tile_wall() {
        let wall_x = 10;
        let jump_right = Input {
            move_x: 1,
            ..HOLD_JUMP
        };
        for (height, clears) in [(2, true), (3, false)] {
            let mut world = flat_world();
            for y in 1..=height {
                put(&mut world, wall_x, y);
            }
            let start = wall_x as f32 * TILE_SIZE - HALF_W;
            let (_, body) = jump_peak(&world, start, jump_right);
            assert_eq!(body.pos.0 > start, clears, "wall {height}: {body:?}");
            assert!(!inside_solid(&world, &body));
        }
    }

    #[test]
    fn coyote_jump_works_once_and_buffered_jump_fires_on_landing() {
        let mut world = World::empty("T".into());
        for x in 0..5 {
            put(&mut world, x, 10);
        }
        // Walk off the right end of a ledge, then jump a few ticks later.
        let mut body = body_at(5.0 * TILE_SIZE - 1.0, 11.0 * TILE_SIZE);
        step(&mut body, HOLD_RIGHT, &world);
        assert!(body.on_ground);
        while body.on_ground {
            step(&mut body, HOLD_RIGHT, &world);
        }
        step(&mut body, HOLD_RIGHT, &world);
        let press = Input {
            jump_pressed: true,
            ..HOLD_JUMP
        };
        step(&mut body, press, &world);
        assert!(body.vel.1 > 0.0, "coyote jump: {body:?}");
        // No second jump in the air.
        step(&mut body, press, &world);
        step(&mut body, HOLD_JUMP, &world);
        assert!(body.vel.1 < JUMP_VELOCITY - 2.0 * GRAVITY * DT);

        // Buffered: pressed just before touching down, fires on the landing tick or the one after.
        let world = flat_world();
        let mut body = body_at(100.0, FLOOR_TOP + 4.0);
        step(&mut body, press, &world);
        let mut jumped = false;
        for _ in 0..4 {
            step(&mut body, HOLD_JUMP, &world);
            jumped |= body.vel.1 > 0.0;
        }
        assert!(jumped, "{body:?}");
    }

    #[test]
    fn falling_at_max_speed_never_ends_a_tick_inside_a_solid() {
        let world = generate("START");
        let top = WORLD_HEIGHT as f32 * TILE_SIZE;
        // Several sub-pixel alignments, so the landing tick hits the floor at different offsets.
        for offset in [0.0, 0.3, 5.7, 12.79] {
            let x = (world.door.0 as f32 + 3.5) * TILE_SIZE;
            let mut body = body_at(x, top - HITBOX_HEIGHT - offset);
            body.vel.1 = -MAX_FALL_SPEED;
            let mut ticks = 0;
            while !body.on_ground {
                step(&mut body, Input::default(), &world);
                assert_eq!(body.vel.1.min(-MAX_FALL_SPEED), -MAX_FALL_SPEED);
                assert!(!inside_solid(&world, &body), "{body:?}");
                ticks += 1;
                assert!(ticks < 600, "never landed: {body:?}");
            }
            let surface = (SURFACE_ROW + 1) as f32 * TILE_SIZE;
            assert!(
                (feet(&body) - surface).abs() < COLLISION_EPSILON,
                "{body:?}"
            );
        }
    }

    #[test]
    fn body_cannot_leave_the_world() {
        let world = generate("START");
        let surface = (SURFACE_ROW + 1) as f32 * TILE_SIZE;
        for (move_x, start_x) in [
            (-1, 3.0 * TILE_SIZE),
            (1, (WORLD_WIDTH - 3) as f32 * TILE_SIZE),
        ] {
            let mut body = body_at(start_x, surface);
            let input = Input {
                move_x,
                jump_held: true,
                jump_pressed: true,
                down: false,
                up: false,
            };
            for _ in 0..300 {
                step(&mut body, input, &world);
                assert!(
                    body.pos.0 >= HALF_W && body.pos.0 <= WORLD_WIDTH as f32 * TILE_SIZE - HALF_W
                );
            }
            let edge = if move_x < 0 {
                HALF_W
            } else {
                WORLD_WIDTH as f32 * TILE_SIZE - HALF_W
            };
            assert_eq!(body.pos.0, edge);
        }
        // The top of the world is a ceiling.
        let mut body = body_at(100.0, WORLD_HEIGHT as f32 * TILE_SIZE - HITBOX_HEIGHT - 1.0);
        body.vel.1 = JUMP_VELOCITY;
        step(&mut body, HOLD_JUMP, &world);
        assert_eq!(body.pos.1, WORLD_HEIGHT as f32 * TILE_SIZE - HALF_H);
    }

    #[test]
    fn spawn_stands_on_the_door_tile() {
        let world = generate("START");
        let mut body = spawn_body(&world);
        let (door_x, _) = tile_to_world(world.door.0, world.door.1);
        assert_eq!(body.pos.0, door_x);
        assert_eq!(feet(&body), world.door.1 as f32 * TILE_SIZE);
        assert!(!inside_solid(&world, &body));
        let spawn = body.pos;
        step(&mut body, Input::default(), &world);
        assert!(body.on_ground);
        assert_eq!(body.pos, spawn);
    }

    #[test]
    fn same_inputs_give_bit_identical_bodies() {
        let world = generate("START");
        let run = || {
            let mut rng = Rng::new(42);
            let mut body = spawn_body(&world);
            let mut trace = Vec::new();
            for _ in 0..2000 {
                let input = Input {
                    move_x: rng.range(0, 2) as i8 - 1,
                    jump_held: rng.chance(0.5),
                    jump_pressed: rng.chance(0.05),
                    down: rng.chance(0.1),
                    up: false,
                };
                step(&mut body, input, &world);
                trace.push((
                    [body.pos.0, body.pos.1, body.vel.0, body.vel.1].map(f32::to_bits),
                    [body.coyote, body.jump_buffer].map(f32::to_bits),
                    (body.on_ground, body.jump_cut_ready),
                ));
            }
            trace
        };
        let first = run();
        assert!(first.iter().any(|t| t.0 != first[0].0), "body never moved");
        assert_eq!(first, run());
    }
}

#[cfg(test)]
mod mod_tests {
    use super::*;
    use crate::items::{DIRT, EMPTY};
    use crate::world::Tile;
    use crate::worldgen::generate;

    fn mods(f: impl FnOnce(&mut Mods)) -> Mods {
        let mut m = Mods::default();
        f(&mut m);
        m
    }

    /// Flat floor at row 1, a wall of `height` tiles at column `wall`.
    fn wall_world(wall: u32, height: u32) -> World {
        let mut world = generate("MODS");
        for y in 0..crate::WORLD_HEIGHT {
            for x in 0..crate::WORLD_WIDTH {
                let fg = if y == 0 || (x == wall && (1..=height).contains(&y)) {
                    DIRT
                } else {
                    EMPTY
                };
                world.set(x, y, Tile { fg, bg: EMPTY });
            }
        }
        world.door = (wall - 3, 1);
        world
    }

    /// Walks right, jumps, jumps again at the top: does the body end up past the wall?
    fn clears(height: u32, m: Mods) -> bool {
        let world = wall_world(20, height);
        let mut body = spawn_body(&world);
        for _ in 0..30 {
            step_with(&mut body, Input::default(), &world, m);
        }
        let mut pressed = false;
        for tick in 0..240 {
            let jump = tick == 0 || (!pressed && body.vel.1 <= 0.0 && tick > 1);
            pressed |= jump && tick > 0;
            let input = Input {
                move_x: 1,
                jump_pressed: jump,
                down: false,
                up: false,
                jump_held: true,
            };
            step_with(&mut body, input, &world, m);
        }
        body.pos.0 > 21.0 * TILE_SIZE
    }

    #[test]
    fn extra_jump_clears_three_tiles_not_five_and_only_once() {
        let jumper = mods(|m| m.extra_jump = true);
        assert!(clears(3, jumper));
        assert!(!clears(5, jumper));
        assert!(
            !clears(3, Mods::default()),
            "without the mod a 3-tile wall stops you"
        );
        // A second press in the same airtime does nothing.
        let world = wall_world(60, 1);
        let mut body = spawn_body(&world);
        for _ in 0..30 {
            step_with(&mut body, Input::default(), &world, jumper);
        }
        let press = Input {
            jump_pressed: true,
            jump_held: true,
            ..Input::default()
        };
        step_with(&mut body, press, &world, jumper);
        while body.vel.1 > 0.0 {
            step_with(
                &mut body,
                Input {
                    jump_held: true,
                    ..Input::default()
                },
                &world,
                jumper,
            );
        }
        step_with(&mut body, press, &world, jumper);
        assert!(body.extra_jump_used && body.vel.1 > 0.0);
        while body.vel.1 > 0.0 {
            step_with(&mut body, Input::default(), &world, jumper);
        }
        step_with(&mut body, press, &world, jumper);
        assert!(body.vel.1 <= 0.0, "no third jump");
    }

    #[test]
    fn glide_caps_the_fall_only_while_jump_is_held() {
        let world = wall_world(60, 1);
        let glider = mods(|m| m.glide = true);
        let mut body = spawn_body(&world);
        body.pos.1 = 40.0 * TILE_SIZE;
        for _ in 0..60 {
            step_with(
                &mut body,
                Input {
                    jump_held: true,
                    ..Input::default()
                },
                &world,
                glider,
            );
        }
        assert!(body.vel.1 >= -GLIDE_FALL_SPEED);
        for _ in 0..30 {
            step_with(&mut body, Input::default(), &world, glider);
        }
        assert!(body.vel.1 < -GLIDE_FALL_SPEED, "released: falls fast again");
    }

    #[test]
    fn run_boost_twenty_walks_at_seven_point_two() {
        // The wall is 3 tiles right of the spawn: run left, into the open.
        let world = wall_world(90, 1);
        let mut body = spawn_body(&world);
        let m = mods(|m| m.run_pct = 20);
        let left = Input {
            move_x: -1,
            ..Input::default()
        };
        for _ in 0..60 {
            step_with(&mut body, left, &world, m);
        }
        let speed = -body.vel.0;
        assert!(
            (speed - 7.2 * TILE_SIZE).abs() < 0.01,
            "{}",
            speed / TILE_SIZE
        );
    }

    #[test]
    fn same_mods_same_steps_bit_for_bit() {
        let world = generate("MODS");
        let m = mods(|m| {
            m.extra_jump = true;
            m.run_pct = 10;
        });
        let (mut a, mut b) = (spawn_body(&world), spawn_body(&world));
        for tick in 0..300u32 {
            let input = Input {
                move_x: if tick % 90 < 45 { 1 } else { -1 },
                jump_pressed: tick % 37 == 0,
                down: tick % 53 < 3,
                up: tick % 41 < 5,
                jump_held: tick % 37 < 20,
            };
            step_with(&mut a, input, &world, m);
            step_with(&mut b, input, &world, m);
        }
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod platform_tests {
    use super::*;
    use crate::items::{DIRT, EMPTY, STONE_PLATFORM, WOOD_PLATFORM};
    use crate::world::Tile;
    use crate::worldgen::generate;

    /// Flat ground at row 0, platforms at the given rows across columns 40..60, spawn at column 50.
    fn world_with(rows: &[u32]) -> World {
        let mut world = generate("PLATFORMS");
        for y in 0..crate::WORLD_HEIGHT {
            for x in 0..crate::WORLD_WIDTH {
                let fg = if y == 0 {
                    DIRT
                } else if (40..60).contains(&x) && rows.contains(&y) {
                    if y % 2 == 0 {
                        WOOD_PLATFORM
                    } else {
                        STONE_PLATFORM
                    }
                } else {
                    EMPTY
                };
                world.set(x, y, Tile { fg, bg: EMPTY });
            }
        }
        world.door = (50, 1);
        world
    }

    fn settle(body: &mut Body, world: &World, input: Input, ticks: u32) {
        for _ in 0..ticks {
            step(body, input, world);
        }
    }

    fn feet_row(body: &Body) -> f32 {
        (body.pos.1 - HALF_H) / TILE_SIZE
    }

    #[test]
    fn jump_up_through_and_land_on_top() {
        let world = world_with(&[2]);
        let mut body = spawn_body(&world);
        settle(&mut body, &world, Input::default(), 30);
        assert!(body.on_ground && feet_row(&body) == 1.0);
        let jump = Input {
            jump_pressed: true,
            jump_held: true,
            ..Input::default()
        };
        step(&mut body, jump, &world);
        settle(
            &mut body,
            &world,
            Input {
                jump_held: true,
                ..Input::default()
            },
            120,
        );
        assert!(body.on_ground, "landed");
        assert_eq!(feet_row(&body), 3.0, "on top of the platform at row 2");
    }

    #[test]
    fn walking_sideways_passes_through() {
        // A platform at row 1 is level with the feet: walking into it from the side does not stop.
        let world = world_with(&[1, 2]);
        let mut body = spawn_body(&world);
        body.pos.0 = 30.0 * TILE_SIZE;
        settle(&mut body, &world, Input::default(), 30);
        let start = body.pos.0;
        settle(
            &mut body,
            &world,
            Input {
                move_x: 1,
                ..Input::default()
            },
            120,
        );
        assert!(body.pos.0 > start + 10.0 * TILE_SIZE, "walked through");
    }

    #[test]
    fn tap_down_drops_one_platform_hold_drops_all() {
        let world = world_with(&[6, 3]);
        let start = |world: &World| {
            let mut body = spawn_body(world);
            body.pos.1 = 7.0 * TILE_SIZE + HALF_H;
            body
        };
        let mut body = start(&world);
        settle(&mut body, &world, Input::default(), 30);
        assert_eq!(feet_row(&body), 7.0, "standing on the top platform");
        step(
            &mut body,
            Input {
                down: true,
                ..Input::default()
            },
            &world,
        );
        settle(&mut body, &world, Input::default(), 60);
        assert_eq!(feet_row(&body), 4.0, "a tap drops through one");

        let mut body = start(&world);
        settle(&mut body, &world, Input::default(), 30);
        settle(
            &mut body,
            &world,
            Input {
                down: true,
                ..Input::default()
            },
            90,
        );
        assert_eq!(
            feet_row(&body),
            1.0,
            "holding drops through both, onto the ground"
        );
    }

    #[test]
    fn down_on_ground_does_nothing_and_a_platform_never_moves_a_player() {
        let world = world_with(&[]);
        let mut body = spawn_body(&world);
        settle(&mut body, &world, Input::default(), 30);
        let before = body;
        settle(
            &mut body,
            &world,
            Input {
                down: true,
                ..Input::default()
            },
            30,
        );
        assert_eq!(body, before);
        // A platform where the player stands (rows 1 and 2 overlap the hitbox).
        let mut inside = world_with(&[1, 2]);
        inside.door = (50, 1);
        let mut body = spawn_body(&inside);
        settle(&mut body, &world, Input::default(), 30);
        let still = body;
        settle(&mut body, &inside, Input::default(), 30);
        assert_eq!(body.pos, still.pos, "not pushed");
    }
}

#[cfg(test)]
mod ladder_tests {
    use super::*;
    use crate::items::{DIRT, EMPTY, LADDER};
    use crate::world::Tile;
    use crate::worldgen::generate;

    const COLUMN: u32 = 52;
    /// Ladder rows 1..=TOP; its top edge is at row TOP + 1.
    const TOP: u32 = 6;
    const UP: Input = Input {
        move_x: 0,
        jump_held: false,
        jump_pressed: false,
        down: false,
        up: true,
    };
    const DOWN: Input = Input {
        up: false,
        down: true,
        ..UP
    };

    /// Flat ground at row 0, one ladder in `COLUMN`, rows 1..=TOP.
    fn ladder_world() -> World {
        let mut world = generate("LADDERS");
        for y in 0..crate::WORLD_HEIGHT {
            for x in 0..crate::WORLD_WIDTH {
                let fg = match (x, y) {
                    (_, 0) => DIRT,
                    (COLUMN, 1..=TOP) => LADDER,
                    _ => EMPTY,
                };
                world.set(x, y, Tile { fg, bg: EMPTY });
            }
        }
        world.door = (50, 1);
        world
    }

    /// Standing on the ground at the foot of the ladder.
    fn at_ladder(world: &World) -> Body {
        let mut body = spawn_body(world);
        body.pos.0 = (COLUMN as f32 + 0.5) * TILE_SIZE;
        settle(&mut body, world, Input::default(), 30);
        assert!(body.on_ground && !body.climbing);
        body
    }

    fn settle(body: &mut Body, world: &World, input: Input, ticks: u32) {
        for _ in 0..ticks {
            step(body, input, world);
        }
    }

    fn feet(body: &Body) -> f32 {
        body.pos.1 - HALF_H
    }

    #[test]
    fn up_climbs_at_the_climb_speed_and_no_input_hangs_still() {
        let world = ladder_world();
        let mut body = at_ladder(&world);
        let start = body.pos.1;
        settle(&mut body, &world, UP, 10);
        assert!(body.climbing);
        assert!(
            (body.pos.1 - start - 10.0 * CLIMB_UP_SPEED * DT).abs() < 0.01,
            "{body:?}"
        );
        let hanging = body.pos.1;
        settle(&mut body, &world, Input::default(), 60);
        assert!(body.climbing && body.pos.1 == hanging, "{body:?}");
        // Down to the bottom: the climb ends standing on the ground.
        settle(&mut body, &world, DOWN, 60);
        assert!(
            body.on_ground && !body.climbing && feet(&body) == TILE_SIZE,
            "{body:?}"
        );
    }

    #[test]
    fn walking_through_a_ladder_does_not_grab() {
        let world = ladder_world();
        let mut body = spawn_body(&world);
        settle(&mut body, &world, Input::default(), 30);
        let right = Input {
            move_x: 1,
            ..Input::default()
        };
        for _ in 0..40 {
            step(&mut body, right, &world);
            assert!(!body.climbing, "{body:?}");
        }
        assert!(body.pos.0 > (COLUMN + 1) as f32 * TILE_SIZE, "walked past");
    }

    #[test]
    fn grabbing_mid_fall_stops_the_fall_in_the_same_tick() {
        let world = ladder_world();
        let mut body = at_ladder(&world);
        body.pos.1 = 4.5 * TILE_SIZE;
        body.vel.1 = -MAX_FALL_SPEED;
        body.on_ground = false;
        body.extra_jump_used = true;
        let before = body.pos.1;
        step(&mut body, UP, &world);
        assert!(body.climbing && body.vel.1 == CLIMB_UP_SPEED && body.pos.1 > before);
        assert!(!body.extra_jump_used, "a grab refills the extra jump");
    }

    #[test]
    fn climbing_past_the_top_lands_on_it_and_down_climbs_back() {
        let world = ladder_world();
        let mut body = at_ladder(&world);
        let top = (TOP + 1) as f32 * TILE_SIZE;
        settle(&mut body, &world, UP, 180);
        assert!(
            body.on_ground && !body.climbing && feet(&body) == top,
            "{body:?}"
        );
        settle(&mut body, &world, Input::default(), 30);
        assert_eq!(feet(&body), top, "the top holds like a platform");
        settle(&mut body, &world, DOWN, 10);
        assert!(body.climbing && feet(&body) < top, "{body:?}");
    }

    #[test]
    fn jump_lets_go_with_a_normal_jump() {
        let world = ladder_world();
        let mut body = at_ladder(&world);
        settle(&mut body, &world, UP, 20);
        let jump = Input {
            jump_pressed: true,
            jump_held: true,
            ..Input::default()
        };
        step(&mut body, jump, &world);
        assert!(!body.climbing);
        assert_eq!(body.vel.1, JUMP_VELOCITY - GRAVITY * DT);
    }

    #[test]
    fn climbing_steps_are_deterministic() {
        let world = ladder_world();
        let script = [UP, UP, Input::default(), DOWN, UP, DOWN, DOWN];
        let run = || {
            let mut body = at_ladder(&world);
            for &input in script.iter().cycle().take(300) {
                step(&mut body, input, &world);
            }
            body
        };
        let (a, b) = (run(), run());
        assert_eq!(a, b);
        assert_eq!(a.pos.0.to_bits(), b.pos.0.to_bits());
        assert_eq!(a.pos.1.to_bits(), b.pos.1.to_bits());
    }
}
