//! Decoration plants (docs/knowledge/02, "Foliage"). Not world state: every client derives the same
//! foliage from the world name and the current tiles. No collision, no punching, no drops; nothing
//! outside this file sees it.

use std::collections::HashMap;

use bevy::prelude::*;
use game_shared::items::{CAVE_WALL, Collision, Sprite as ItemSprite, item};
use game_shared::rules::Event;
use game_shared::world::World;
use game_shared::{TILE_SIZE, WORLD_HEIGHT, WORLD_WIDTH, tile_to_world};

use crate::state::{Game, GameEvent, Phase};
use crate::world_view::{TERRAIN_DIRT, TERRAIN_GRASS, TERRAIN_SAND, Z_FOLIAGE, drawn_terrain};

// Tuning (docs/knowledge/02).
const FOLIAGE_TREE: f32 = 0.06;
const FOLIAGE_SMALL: f32 = 0.25;
const FOLIAGE_PALM: f32 = 0.04;
const FOLIAGE_DESERT: f32 = 0.15;
const FOLIAGE_CAVE: f32 = 0.08;
const FOLIAGE_HANGING: f32 = 0.20;

const GRASS_BIG: &[&str] = &[
    "tree_pine",
    "tree_birch",
    "tree_willow",
    "tree_maple_autumn",
    "tree_apple",
    "tree_dead",
    "tree_mushroom",
    "bush_large",
    "hedge_flowers",
    "boulder_ferns",
    "log_hollow",
    "bamboo",
    "reeds_tall",
    "crystal_plant",
];
const GRASS_SMALL: &[&str] = &[
    "grass_short",
    "grass_tall",
    "grass_seeds",
    "clover",
    "dandelions",
    "tulips",
    "bluebells",
    "daisies",
    "poppies",
    "lavender",
    "pink_flowers",
    "sunflower",
    "mushrooms_brown",
    "toadstool",
    "fern_sprout",
    "bush_round",
    "berry_bush",
    "bramble",
    "reeds",
    "rock_mossy",
    "pebbles",
    "stump",
    "log_short",
    "ivy",
    "pumpkin",
];
const SAND_BIG: &[&str] = &["tree_palm"];
const SAND_SMALL: &[&str] = &["cactus", "desert_shrub"];
const CAVE: &[&str] = &["mushrooms_glow", "mushrooms_brown", "pebbles"];
const HANGING: &[&str] = &["vines_hanging", "moss_hanging"];

/// Px edge of a big plant: 2x2 tiles.
const BIG_SIZE: f32 = 2.0 * TILE_SIZE;
/// Big plants sit behind small ones, so a flower next to a trunk is drawn in front of it.
const Z_SMALL_OFFSET: f32 = 0.1;
/// A tile's foliage depends on tiles up to 3 columns to its left (two big-plant candidates, each
/// checking the column left of it) and 1 to its right; rows reach 1 up and 1 down.
const RECOMPUTE_COLUMNS: i32 = 3;
const RECOMPUTE_ROWS: i32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// 32x32, base on the tile's bottom edge.
    Small,
    /// 64x64, centred horizontally on its tile, base on the tile's bottom edge.
    Big,
    /// 32x32, top on the tile's top edge.
    Hanging,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Foliage {
    /// File stem under `assets/foliage/`.
    pub sprite: &'static str,
    pub shape: Shape,
}

/// FNV-1a over the world name, then x and y (little-endian u32), finished with SplitMix64.
fn tile_hash(name: &str, x: u32, y: u32) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes().chain(x.to_le_bytes()).chain(y.to_le_bytes()) {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h = h.wrapping_add(0x9e37_79b9_7f4a_7c15);
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

/// The independent rolls of one tile: two chances in [0, 1) and a list pick.
struct Rolls {
    big: f32,
    other: f32,
    pick: u64,
}

impl Rolls {
    fn of(world: &World, x: u32, y: u32) -> Self {
        let h = tile_hash(&world.name, x, y);
        let unit = |bits: u64| (bits & 0xff_ffff) as f32 / (1 << 24) as f32;
        Rolls {
            big: unit(h >> 40),
            other: unit(h >> 16),
            pick: h & 0xffff,
        }
    }

    fn choose(&self, list: &'static [&'static str]) -> &'static str {
        list[(self.pick % list.len() as u64) as usize]
    }
}

fn offset(x: u32, y: u32, dx: i32, dy: i32) -> Option<(u32, u32)> {
    let (nx, ny) = (x.checked_add_signed(dx)?, y.checked_add_signed(dy)?);
    (nx < WORLD_WIDTH && ny < WORLD_HEIGHT).then_some((nx, ny))
}

fn solid(world: &World, x: u32, y: u32) -> bool {
    world
        .get(x, y)
        .and_then(|tile| item(tile.fg))
        .is_some_and(|def| def.collision == Collision::Solid)
}

/// Empty foreground (which also rules out the door and tree/seed tiles), checked explicitly too.
fn free(world: &World, x: u32, y: u32) -> bool {
    world.get(x, y).is_some_and(|tile| item(tile.fg).is_none())
        && world.door != (x, y)
        && world.tree(x, y).is_none()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ground {
    Grass,
    Sand,
}

/// Grass (dirt drawn as grass) or sand open to the sky under `(x, y)`.
fn surface_ground(world: &World, x: u32, y: u32) -> Option<Ground> {
    let (bx, by) = offset(x, y, 0, -1)?;
    let below = world.get(bx, by)?;
    let ItemSprite::Terrain(terrain) = item(below.fg)?.sprite else {
        return None;
    };
    if terrain == TERRAIN_DIRT && drawn_terrain(world, bx, by, terrain) == TERRAIN_GRASS {
        Some(Ground::Grass)
    } else if terrain == TERRAIN_SAND && item(below.bg).is_none() {
        Some(Ground::Sand)
    } else {
        None
    }
}

/// Columns x-1..=x+1 of this row and the row above are empty (and inside the world).
fn has_room(world: &World, x: u32, y: u32) -> bool {
    (-1..=1).all(|dx| {
        (0..=1).all(|dy| offset(x, y, dx, dy).is_some_and(|(nx, ny)| free(world, nx, ny)))
    })
}

/// A big plant would go here if no neighbour wanted one: the tile is free, on grass or sand,
/// rolled under the big chance and has room.
fn big_candidate(world: &World, x: u32, y: u32) -> Option<&'static str> {
    if !free(world, x, y) {
        return None;
    }
    let (chance, list) = match surface_ground(world, x, y)? {
        Ground::Grass => (FOLIAGE_TREE, GRASS_BIG),
        Ground::Sand => (FOLIAGE_PALM, SAND_BIG),
    };
    let rolls = Rolls::of(world, x, y);
    (rolls.big < chance && has_room(world, x, y)).then(|| rolls.choose(list))
}

/// Left to right: a candidate wins unless one of the two columns left of it is a candidate too.
/// Depends on candidates only, never on other winners, so it is local and order-free; two winners
/// can never be within 2 columns of each other.
fn big_plant(world: &World, x: u32, y: u32) -> Option<&'static str> {
    let sprite = big_candidate(world, x, y)?;
    let left_wants = (1..=2).any(|dx| x >= dx && big_candidate(world, x - dx, y).is_some());
    (!left_wants).then_some(sprite)
}

/// What decoration tile `(x, y)` shows, if any. Pure: the world name and current tiles decide.
pub fn foliage_at(world: &World, x: u32, y: u32) -> Option<Foliage> {
    if !free(world, x, y) {
        return None;
    }
    let rolls = Rolls::of(world, x, y);
    let small = |chance: f32, list| {
        (rolls.other < chance).then(|| Foliage {
            sprite: rolls.choose(list),
            shape: Shape::Small,
        })
    };
    if let Some(ground) = surface_ground(world, x, y) {
        if let Some(sprite) = big_plant(world, x, y) {
            return Some(Foliage {
                sprite,
                shape: Shape::Big,
            });
        }
        return match ground {
            Ground::Grass => small(FOLIAGE_SMALL, GRASS_SMALL),
            Ground::Sand => small(FOLIAGE_DESERT, SAND_SMALL),
        };
    }
    let solid_below = offset(x, y, 0, -1).is_some_and(|(bx, by)| solid(world, bx, by));
    let cave = world.get(x, y).is_some_and(|tile| tile.bg == CAVE_WALL);
    if cave && solid_below {
        return small(FOLIAGE_CAVE, CAVE);
    }
    let solid_above = offset(x, y, 0, 1).is_some_and(|(ax, ay)| solid(world, ax, ay));
    if solid_above && !solid_below {
        return (rolls.other < FOLIAGE_HANGING).then(|| Foliage {
            sprite: rolls.choose(HANGING),
            shape: Shape::Hanging,
        });
    }
    None
}

/// The foliage entity of each tile that has one, and what it shows.
#[derive(Resource, Default)]
struct FoliageSprites(HashMap<(u32, u32), (Foliage, Entity)>);

pub struct FoliagePlugin;

impl Plugin for FoliagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FoliageSprites>()
            .add_systems(Startup, draw_all)
            .add_systems(
                Update,
                (
                    // A whole new world: `redraw` compares every tile, so this replaces what changed.
                    draw_all.run_if(on_message::<crate::state::WorldReplaced>),
                    on_tile_changed,
                )
                    .chain()
                    .in_set(Phase::View),
            );
    }
}

/// Makes tile `(x, y)`'s sprite match `foliage_at`; untouched if it already does.
fn redraw(
    commands: &mut Commands,
    assets: &AssetServer,
    world: &World,
    sprites: &mut FoliageSprites,
    (x, y): (u32, u32),
) {
    let want = foliage_at(world, x, y);
    if sprites.0.get(&(x, y)).map(|(have, _)| *have) == want {
        return;
    }
    if let Some((_, entity)) = sprites.0.remove(&(x, y)) {
        commands.entity(entity).try_despawn();
    }
    let Some(foliage) = want else { return };
    let (cx, cy) = tile_to_world(x, y);
    let (y_offset, z) = match foliage.shape {
        Shape::Big => ((BIG_SIZE - TILE_SIZE) / 2.0, Z_FOLIAGE),
        Shape::Small | Shape::Hanging => (0.0, Z_FOLIAGE + Z_SMALL_OFFSET),
    };
    let image = assets.load(format!("foliage/{}.png", foliage.sprite));
    let entity = commands
        .spawn((
            Sprite::from_image(image),
            Transform::from_xyz(cx, cy + y_offset, z),
        ))
        .id();
    sprites.0.insert((x, y), (foliage, entity));
}

fn draw_all(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    mut sprites: ResMut<FoliageSprites>,
) {
    for y in 0..WORLD_HEIGHT {
        for x in 0..WORLD_WIDTH {
            redraw(&mut commands, &assets, &game.0.world, &mut sprites, (x, y));
        }
    }
}

/// The tiles whose foliage a change at `(x, y)` can alter.
fn affected(x: u32, y: u32) -> impl Iterator<Item = (u32, u32)> {
    (-RECOMPUTE_ROWS..=RECOMPUTE_ROWS).flat_map(move |dy| {
        (-RECOMPUTE_COLUMNS..=RECOMPUTE_COLUMNS).filter_map(move |dx| offset(x, y, dx, dy))
    })
}

fn on_tile_changed(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut sprites: ResMut<FoliageSprites>,
) {
    for GameEvent(event) in events.read() {
        if let Event::TileChanged { x, y, .. } = *event {
            for tile in affected(x, y) {
                redraw(&mut commands, &assets, &game.0.world, &mut sprites, tile);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{BEDROCK, DIRT, EMPTY, SAND};
    use game_shared::world::Tile;
    use game_shared::worldgen::{SURFACE_ROW, generate};

    const NAMES: [&str; 6] = ["START", "BUILD", "JUNGLE", "A", "PARK42", "ZZZ"];
    const SKY: u32 = SURFACE_ROW + 1;

    fn all(world: &World) -> Vec<((u32, u32), Option<Foliage>)> {
        (0..WORLD_HEIGHT)
            .flat_map(|y| (0..WORLD_WIDTH).map(move |x| (x, y)))
            .map(|(x, y)| ((x, y), foliage_at(world, x, y)))
            .collect()
    }

    fn block(fg: game_shared::items::ItemId) -> Tile {
        Tile { fg, bg: EMPTY }
    }

    #[test]
    fn deterministic_per_name_and_different_across_names() {
        let a = generate("START");
        assert_eq!(all(&a), all(&generate("START")));
        assert_ne!(all(&a), all(&generate("BUILD")));
    }

    #[test]
    fn never_on_foreground_door_or_tree_tiles() {
        for name in NAMES {
            let world = generate(name);
            for ((x, y), foliage) in all(&world) {
                if foliage.is_some() {
                    assert!(free(&world, x, y), "{name} ({x}, {y})");
                }
            }
            let (dx, dy) = world.door;
            assert_eq!(foliage_at(&world, dx, dy), None);
        }
        // A tree tile (its seed in the foreground) has none, whatever the roll.
        let mut world = generate("START");
        let seed = game_shared::items::seed_of(DIRT).unwrap();
        for x in 0..WORLD_WIDTH {
            if world.door.0 != x {
                world.set(x, SKY, block(seed));
            }
        }
        assert!((0..WORLD_WIDTH).all(|x| foliage_at(&world, x, SKY).is_none()));
    }

    /// Every foliage found in `world` restricted to `rows` comes from `lists`, and some exist.
    fn only_from(world: &World, rows: std::ops::RangeInclusive<u32>, lists: &[&[&str]]) {
        let mut seen = 0;
        for ((x, y), foliage) in all(world) {
            if let Some(foliage) = foliage.filter(|_| rows.contains(&y)) {
                assert!(
                    lists.iter().any(|list| list.contains(&foliage.sprite)),
                    "{} at ({x}, {y})",
                    foliage.sprite
                );
                seen += 1;
            }
        }
        assert!(seen > 0);
    }

    #[test]
    fn each_rule_picks_only_from_its_own_list() {
        // Grass: the plain generated surface.
        only_from(&generate("START"), SKY..=SKY, &[GRASS_BIG, GRASS_SMALL]);

        // Sand open to the sky: the surface row turned to sand.
        let mut world = generate("START");
        for x in 0..WORLD_WIDTH {
            if world.door.0 != x {
                world.set(x, SURFACE_ROW, block(SAND));
            }
        }
        only_from(&world, SKY..=SKY, &[SAND_BIG, SAND_SMALL]);

        // Cave: a dug-out tunnel keeps its cave wall; its floor is solid.
        let mut world = generate("START");
        let tunnel = SURFACE_ROW - 10;
        for x in 0..WORLD_WIDTH {
            world.set(x, tunnel - 1, block(BEDROCK));
            world.set(
                x,
                tunnel,
                Tile {
                    fg: EMPTY,
                    bg: CAVE_WALL,
                },
            );
        }
        only_from(&world, tunnel..=tunnel, &[CAVE]);

        // Hanging: under a floating roof, with nothing solid below.
        let mut world = generate("START");
        let roof = SKY + 6;
        for x in 0..WORLD_WIDTH {
            world.set(x, roof, block(DIRT));
        }
        only_from(&world, roof - 1..=roof - 1, &[HANGING]);
        assert!(all(&world).iter().all(|(_, f)| {
            f.is_none_or(|f| (f.shape == Shape::Hanging) == HANGING.contains(&f.sprite))
        }));
    }

    fn big_plants(world: &World) -> Vec<(u32, u32)> {
        all(world)
            .into_iter()
            .filter(|(_, f)| f.is_some_and(|f| f.shape == Shape::Big))
            .map(|(at, _)| at)
            .collect()
    }

    #[test]
    fn big_plants_only_with_room_and_never_two_within_two_columns() {
        let mut total = 0;
        for name in NAMES {
            let world = generate(name);
            let big = big_plants(&world);
            total += big.len();
            for &(x, y) in &big {
                assert!(has_room(&world, x, y), "{name} ({x}, {y})");
                assert!(
                    !big.iter()
                        .any(|&(ox, oy)| oy == y && ox != x && ox.abs_diff(x) <= 2),
                    "{name} ({x}, {y})"
                );
            }
        }
        assert!(total > 0, "no big plant in {} worlds", NAMES.len());
    }

    #[test]
    fn building_over_a_big_plant_or_digging_under_a_small_one_removes_it() {
        for name in NAMES {
            let mut world = generate(name);
            if let Some(&(x, y)) = big_plants(&world).first() {
                // No room any more: the big plant is gone (a small one may take its place).
                world.set(x, y + 1, block(DIRT));
                let now = foliage_at(&world, x, y);
                assert!(now.is_none_or(|f| f.shape != Shape::Big), "{name}");
            }
            let world = generate(name);
            let small = all(&world)
                .into_iter()
                .find(|(_, f)| f.is_some_and(|f| f.shape == Shape::Small));
            if let Some(((x, y), _)) = small {
                let mut world = world;
                world.set(x, y - 1, Tile::default());
                assert_eq!(foliage_at(&world, x, y), None, "{name}");
            }
        }
    }

    #[test]
    fn surface_share_is_roughly_the_configured_chances() {
        let (mut tiles, mut planted) = (0, 0);
        for name in NAMES {
            let world = generate(name);
            for x in 0..WORLD_WIDTH {
                if surface_ground(&world, x, SKY) == Some(Ground::Grass) {
                    tiles += 1;
                    planted += usize::from(foliage_at(&world, x, SKY).is_some());
                }
            }
        }
        let share = planted as f32 / tiles as f32;
        // About SMALL plus some of TREE; wide tolerance for 600 tiles.
        assert!(
            (FOLIAGE_SMALL * 0.6..FOLIAGE_SMALL + FOLIAGE_TREE * 2.0).contains(&share),
            "{share}"
        );
    }

    #[test]
    fn a_change_recomputes_its_neighbourhood_inside_the_world() {
        assert_eq!(affected(50, 20).count(), 21);
        assert_eq!(affected(0, 0).count(), 8);
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(tile_hash("START", 3, 4), tile_hash("START", 3, 4));
        assert_ne!(tile_hash("START", 3, 4), tile_hash("START", 4, 3));
        assert_ne!(tile_hash("START", 3, 4), tile_hash("STARU", 3, 4));
    }
}
