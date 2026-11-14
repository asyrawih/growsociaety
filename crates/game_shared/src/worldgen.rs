//! `generate(name)`: a pure function. Same name, same world, on any machine (docs/knowledge/02).

use crate::items::{BEDROCK, CAVE_WALL, DIRT, EMPTY, ItemId, LAVA, MAIN_DOOR, SAND, STONE};
use crate::rng::Rng;
use crate::world::{Tile, World};
use crate::{WORLD_HEIGHT, WORLD_WIDTH};

// Vertical layout, rows counted from the bottom. Everything above the surface row is sky.
pub const BEDROCK_ROWS: u32 = 5;
pub const GROUND_ROWS: u32 = 25;
/// Lowest rows of the ground band; the only place lava generates.
pub const LAVA_ROWS: u32 = 4;
pub const SURFACE_ROW: u32 = BEDROCK_ROWS + GROUND_ROWS - 1;
/// Sand generates in the top `GROUND_ROWS / SAND_BAND_DIVISOR` rows of the ground band.
pub const SAND_BAND_DIVISOR: u32 = 3;
/// The main door keeps this many tiles clear of each world edge.
pub const DOOR_EDGE_MARGIN: u32 = 2;

// Tuning.
pub const STONE_MIN: f32 = 0.03;
pub const STONE_MAX: f32 = 0.20;
pub const SAND_CHANCE: f32 = 0.04;
pub const LAVA_CHANCE: f32 = 0.10;

pub const MAX_WORLD_NAME_LEN: usize = 24;

const _: () = assert!(SURFACE_ROW + 1 < WORLD_HEIGHT && LAVA_ROWS < GROUND_ROWS);
const _: () = assert!(2 * DOOR_EDGE_MARGIN < WORLD_WIDTH);

/// 1-24 ASCII letters or digits. The deny-list is the server's job; only the server's answer counts.
pub fn validate_world_name(name: &str) -> bool {
    (1..=MAX_WORLD_NAME_LEN).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// FNV-1a 64-bit.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Replaces the foreground of every tile in `rows` that passes `pick` and the roll.
/// Rolls once per tile, row-major, so the stream does not depend on tile contents.
fn scatter(
    world: &mut World,
    rng: &mut Rng,
    rows: std::ops::Range<u32>,
    fg: ItemId,
    replaces: impl Fn(ItemId) -> bool,
    chance: impl Fn(u32) -> f32,
) {
    for y in rows {
        for x in 0..WORLD_WIDTH {
            let hit = rng.chance(chance(y));
            if let Some(tile) = world.get(x, y)
                && hit
                && replaces(tile.fg)
            {
                world.set(x, y, Tile { fg, ..tile });
            }
        }
    }
}

pub fn generate(name: &str) -> World {
    let name = name.to_ascii_uppercase();
    // 1. Seed.
    let mut rng = Rng::new(fnv1a(name.as_bytes()));
    let mut world = World::empty(name);
    let ground_bottom = BEDROCK_ROWS;

    // 2-3. Bedrock, then dirt; cave wall behind everything except the surface row.
    for y in 0..=SURFACE_ROW {
        let tile = Tile {
            fg: if y < BEDROCK_ROWS { BEDROCK } else { DIRT },
            bg: if y < SURFACE_ROW { CAVE_WALL } else { EMPTY },
        };
        for x in 0..WORLD_WIDTH {
            world.set(x, y, tile);
        }
    }

    // 4. Stone, more likely with depth: 0.0 just under the surface row, 1.0 at the bottom ground row.
    let stone_span = (SURFACE_ROW - 1 - ground_bottom).max(1) as f32;
    scatter(
        &mut world,
        &mut rng,
        ground_bottom..SURFACE_ROW,
        STONE,
        |fg| fg == DIRT,
        |y| STONE_MIN + (STONE_MAX - STONE_MIN) * ((SURFACE_ROW - 1 - y) as f32 / stone_span),
    );

    // 5. Sand in the top third of the ground band, below the surface row.
    let sand_bottom = ground_bottom + GROUND_ROWS - GROUND_ROWS / SAND_BAND_DIVISOR;
    scatter(
        &mut world,
        &mut rng,
        sand_bottom..SURFACE_ROW,
        SAND,
        |fg| fg == DIRT,
        |_| SAND_CHANCE,
    );

    // 6. Lava in the lava rows only.
    scatter(
        &mut world,
        &mut rng,
        ground_bottom..ground_bottom + LAVA_ROWS,
        LAVA,
        |_| true,
        |_| LAVA_CHANCE,
    );

    // 7. Main door, with bedrock under it so the spawn cannot be dug out.
    let x = rng.range(DOOR_EDGE_MARGIN, WORLD_WIDTH - 1 - DOOR_EDGE_MARGIN);
    world.door = (x, SURFACE_ROW + 1);
    world.set(
        x,
        SURFACE_ROW + 1,
        Tile {
            fg: MAIN_DOOR,
            bg: EMPTY,
        },
    );
    world.set(
        x,
        SURFACE_ROW,
        Tile {
            fg: BEDROCK,
            bg: EMPTY,
        },
    );

    // 8. No caves, ores or trees.
    world
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiles(world: &World) -> impl Iterator<Item = (u32, u32, Tile)> + '_ {
        (0..WORLD_HEIGHT).flat_map(move |y| {
            (0..WORLD_WIDTH).map(move |x| (x, y, world.get(x, y).expect("in bounds")))
        })
    }

    #[test]
    fn layout_constants_match_the_doc() {
        assert_eq!(
            (BEDROCK_ROWS, GROUND_ROWS, LAVA_ROWS, SURFACE_ROW),
            (5, 25, 4, 29)
        );
    }

    #[test]
    fn fnv1a_matches_reference_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn generate_is_deterministic() {
        assert_eq!(generate("START"), generate("START"));
        assert_eq!(generate("start"), generate("START"));
        assert_eq!(generate("start").name, "START");
        assert_ne!(generate("START").door, generate("FARM").door);
    }

    #[test]
    fn layout_bedrock_sky_and_lava_rows() {
        let world = generate("START");
        let lava_rows = BEDROCK_ROWS..BEDROCK_ROWS + LAVA_ROWS;
        let mut lava = 0;
        for (x, y, tile) in tiles(&world) {
            if y < BEDROCK_ROWS {
                assert_eq!((tile.fg, tile.bg), (BEDROCK, CAVE_WALL), "({x},{y})");
            }
            if y > SURFACE_ROW && (x, y) != world.door {
                assert_eq!(tile, Tile::default(), "({x},{y})");
            }
            if tile.fg == LAVA {
                assert!(lava_rows.contains(&y), "lava at ({x},{y})");
                lava += 1;
            }
        }
        assert!(lava > 0);
    }

    #[test]
    fn ground_band_contents() {
        let world = generate("START");
        let sand_rows = BEDROCK_ROWS + GROUND_ROWS - GROUND_ROWS / SAND_BAND_DIVISOR..SURFACE_ROW;
        let (mut stone, mut sand) = (0, 0);
        for (x, y, tile) in tiles(&world) {
            if !(BEDROCK_ROWS..=SURFACE_ROW).contains(&y) {
                continue;
            }
            if y == SURFACE_ROW {
                // Dirt open to the sky (drawn as grass), except the bedrock under the door.
                let fg = if x == world.door.0 { BEDROCK } else { DIRT };
                assert_eq!((tile.fg, tile.bg), (fg, EMPTY), "({x},{y})");
                continue;
            }
            assert_eq!(tile.bg, CAVE_WALL, "({x},{y})");
            assert!([DIRT, STONE, SAND, LAVA].contains(&tile.fg), "({x},{y})");
            if tile.fg == SAND {
                assert!(sand_rows.contains(&y), "sand at ({x},{y})");
                sand += 1;
            }
            stone += u32::from(tile.fg == STONE);
        }
        assert!(stone > 0 && sand > 0);
    }

    #[test]
    fn door_sits_on_bedrock() {
        for name in ["START", "FARM", "A", "Z9"] {
            let world = generate(name);
            let (x, y) = world.door;
            assert_eq!(y, SURFACE_ROW + 1);
            assert!((DOOR_EDGE_MARGIN..=WORLD_WIDTH - 1 - DOOR_EDGE_MARGIN).contains(&x));
            assert_eq!(world.get(x, y).map(|t| t.fg), Some(MAIN_DOOR));
            assert_eq!(world.get(x, y - 1).map(|t| t.fg), Some(BEDROCK));
            let doors = tiles(&world).filter(|(_, _, t)| t.fg == MAIN_DOOR).count();
            assert_eq!(doors, 1);
        }
    }

    #[test]
    fn get_is_none_out_of_bounds() {
        let world = generate("START");
        assert!(world.get(WORLD_WIDTH - 1, WORLD_HEIGHT - 1).is_some());
        assert_eq!(world.get(WORLD_WIDTH, 0), None);
        assert_eq!(world.get(0, WORLD_HEIGHT), None);
        assert_eq!(world.get(u32::MAX, u32::MAX), None);
    }

    #[test]
    fn world_names() {
        for ok in ["A", "start", "World42", &"X".repeat(MAX_WORLD_NAME_LEN)] {
            assert!(validate_world_name(ok), "{ok}");
        }
        for bad in [
            "",
            "a b",
            "a-b",
            "naïve",
            &"X".repeat(MAX_WORLD_NAME_LEN + 1),
        ] {
            assert!(!validate_world_name(bad), "{bad}");
        }
    }
}

#[cfg(test)]
mod respawn_tests {
    use super::*;
    use crate::items::LAVA;

    /// The respawn point must never burn (docs/knowledge/ux/respawn.md): no lava within two tiles
    /// of the main door, in any generated world.
    #[test]
    fn lava_never_touches_the_door_or_its_spawn_tiles() {
        for n in 0..200 {
            let world = generate(&format!("W{n}"));
            let (dx, dy) = world.door;
            for y in dy.saturating_sub(2)..=dy + 2 {
                for x in dx.saturating_sub(2)..=dx + 2 {
                    let tile = world.get(x, y).map(|t| t.fg);
                    assert_ne!(
                        tile,
                        Some(LAVA),
                        "W{n}: lava at ({x}, {y}) near the door ({dx}, {dy})"
                    );
                }
            }
        }
    }
}
