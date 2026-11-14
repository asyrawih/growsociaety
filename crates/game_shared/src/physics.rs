//! Player movement as a pure function. Client and server run the same `step` (docs/knowledge/04).

use crate::items::{Collision, item};
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
/// Seconds.
pub const COYOTE_TIME: f32 = 0.1;
/// Seconds.
pub const JUMP_BUFFER: f32 = 0.1;
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

#[derive(Clone, Copy, Default, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Input {
    /// Sign only: anything below zero is left, above zero is right.
    pub move_x: i8,
    pub jump_held: bool,
    pub jump_pressed: bool,
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
}

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
    }
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

/// Advances `body` by exactly `1 / TICK_RATE` seconds.
pub fn step(body: &mut Body, input: Input, world: &World) {
    // 1. Timers.
    body.coyote = (body.coyote - DT).max(0.0);
    body.jump_buffer = (body.jump_buffer - DT).max(0.0);
    if input.jump_pressed {
        body.jump_buffer = JUMP_BUFFER;
    }

    // 2. Horizontal velocity.
    let accel = if body.on_ground {
        GROUND_ACCEL
    } else {
        AIR_ACCEL
    };
    let target = f32::from(input.move_x.signum()) * WALK_SPEED;
    body.vel.0 = approach(body.vel.0, target, accel * DT);

    // 3. Jump.
    if body.jump_buffer > 0.0 && (body.on_ground || body.coyote > 0.0) {
        body.vel.1 = JUMP_VELOCITY;
        body.coyote = 0.0;
        body.jump_buffer = 0.0;
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
    body.vel.1 = (body.vel.1 - GRAVITY * DT).max(-MAX_FALL_SPEED);

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

    // 7. Move Y, resolve.
    body.pos.1 += body.vel.1 * DT;
    body.on_ground = false;
    if body.vel.1 < 0.0 {
        let floor = overlapped_solids(world, body.pos)
            .map(|(_, ty)| (ty + 1) as f32 * TILE_SIZE + HALF_H)
            .reduce(f32::max);
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

    if body.on_ground {
        body.coyote = COYOTE_TIME;
    }
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
    };
    const HOLD_JUMP: Input = Input {
        move_x: 0,
        jump_held: true,
        jump_pressed: false,
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
