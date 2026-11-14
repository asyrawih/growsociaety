//! Tile, drop and damage sprites. Entities are views: they are (re)built from the `WorldState`
//! whenever a `GameEvent` says something changed, and hold no gameplay truth.

use std::collections::HashMap;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::common_conditions::on_timer;
use game_shared::farming::{GROWN_STAGE, SAPLING_HITS, growth_stage};
use game_shared::items::{Collision, ItemId, Sprite as ItemSprite, item};
use game_shared::mods::DAMAGE_PER_HIT;
use game_shared::rules::{DAMAGE_RESET, Event, Layer, drop_position};
use game_shared::world::{Drop, Tile, World};
use game_shared::{TILE_SIZE, WORLD_HEIGHT, WORLD_WIDTH, tile_to_world};

use crate::state::{Game, GameEvent, Phase, WorldReplaced};

const SKY_COLOR: Color = Color::srgb(0.47, 0.71, 0.92);

// Back to front (docs/knowledge/02). Z 0 is left for the parallax.
pub const Z_BACKGROUND: f32 = 1.0;
/// Decoration plants (`foliage`): in front of background walls, behind trees and objects.
pub const Z_FOLIAGE: f32 = 1.5;
pub const Z_OBJECT: f32 = 2.0;
pub const Z_TREE_ICON: f32 = 2.5;
pub const Z_FOREGROUND: f32 = 3.0;
pub const Z_CRACK: f32 = 3.5;
pub const Z_DROP: f32 = 4.0;
pub const Z_PLAYER: f32 = 5.0;
pub const Z_HIGHLIGHT: f32 = 6.0;

// assets/tiles/terrain.png: index = terrain * 16 + mask, cell = (index % 8, index / 8).
const TERRAIN_ATLAS: &str = "tiles/terrain.png";
const ATLAS_COLUMNS: u32 = 8;
const MASKS_PER_TERRAIN: u32 = 16;
pub const TERRAIN_GRASS: u8 = 0;
pub const TERRAIN_DIRT: u8 = 1;
pub const TERRAIN_SAND: u8 = 3;

const MASK_UP: u8 = 1;
const MASK_RIGHT: u8 = 2;
const MASK_DOWN: u8 = 4;
const MASK_LEFT: u8 = 8;
/// The single floating block; also the inventory icon of a terrain item.
const MASK_ALONE: u8 = 0;

/// Px. Dropped items are drawn smaller than a tile.
const DROP_SIZE: f32 = 16.0;

/// `assets/blocks/`: one sprite per growth stage, sapling to grown tree.
const TREE_STAGES: [&str; GROWN_STAGE as usize + 1] = [
    "52_crop_1.png",
    "53_crop_2.png",
    "54_crop_3.png",
    "55_crop_4.png",
    "56_crop_5.png",
    "57_crop_6.png",
];
/// Px. A grown tree shows what it gives as a small icon at its top.
const TREE_ICON_SIZE: f32 = 14.0;
const TREE_ICON_OFFSET_Y: f32 = 8.0;
/// Growth is recomputed from `planted_at` this often, not every frame.
const TREE_REFRESH: Duration = Duration::from_secs(1);
/// Darkening of a tile that is one punch away from breaking.
const CRACK_MAX_ALPHA: f32 = 0.6;

/// Every equipment folder has this ready-made 32x32 icon (a game_shared test checks it).
const EQUIP_ICON_FILE: &str = "icon.png";

/// The sprites of each non-empty tile, so one tile can be redrawn without touching the rest.
#[derive(Resource, Default)]
struct TileSprites(HashMap<(u32, u32), Vec<Entity>>);

#[derive(Resource, Default)]
struct DropSprites(HashMap<u32, Entity>);

/// The stage a tree's sprite currently shows. `vfx.rs` pops a new sapling's sprite.
#[derive(Component)]
pub(crate) struct TreeView {
    pub(crate) x: u32,
    pub(crate) y: u32,
    stage: u8,
}

/// A drop's sprite, for `vfx.rs` (toss, wobble, glow). Spawned again when a merge re-announces it.
#[derive(Component)]
pub(crate) struct DropView {
    pub(crate) id: u32,
    pub(crate) item: ItemId,
}

/// Darkens a damaged tile.
#[derive(Component)]
struct Crack {
    x: u32,
    y: u32,
}

/// Despawned once `Time::elapsed_secs` passes `until`.
#[derive(Component)]
pub struct Temporary {
    pub until: f32,
}

pub struct WorldViewPlugin;

impl Plugin for WorldViewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(SKY_COLOR))
            .init_resource::<TileSprites>()
            .init_resource::<DropSprites>()
            .add_systems(Startup, draw_world)
            .add_systems(
                Update,
                (
                    (draw_world, reset_drops).run_if(on_message::<WorldReplaced>),
                    on_game_events,
                    grow_trees.run_if(on_timer(TREE_REFRESH)),
                    expire_temporary,
                )
                    .chain()
                    .in_set(Phase::View),
            );
    }
}

type LayerOf = fn(Tile) -> ItemId;
const FG: LayerOf = |tile| tile.fg;
const BG: LayerOf = |tile| tile.bg;

/// Which neighbours hold the same item in `layer`. World edges count as the same, so the map border has no rim.
fn autotile_mask(world: &World, x: u32, y: u32, layer: LayerOf) -> u8 {
    let Some(own) = world.get(x, y).map(layer) else {
        return 0;
    };
    let same = |dx: i32, dy: i32| {
        let neighbour = x
            .checked_add_signed(dx)
            .zip(y.checked_add_signed(dy))
            .and_then(|(nx, ny)| world.get(nx, ny));
        neighbour.is_none_or(|tile| layer(tile) == own)
    };
    [
        (MASK_UP, same(0, 1)),
        (MASK_RIGHT, same(1, 0)),
        (MASK_DOWN, same(0, -1)),
        (MASK_LEFT, same(-1, 0)),
    ]
    .into_iter()
    .filter_map(|(bit, set)| set.then_some(bit))
    .sum()
}

/// Grass is a look: dirt open to the sky (no background, nothing solid above) uses the grass terrain.
pub fn drawn_terrain(world: &World, x: u32, y: u32, terrain: u8) -> u8 {
    let open_to_sky = world.get(x, y).is_some_and(|tile| item(tile.bg).is_none());
    let covered = world
        .get(x, y + 1)
        .and_then(|above| item(above.fg))
        .is_some_and(|def| def.collision == Collision::Solid);
    if terrain == TERRAIN_DIRT && open_to_sky && !covered {
        TERRAIN_GRASS
    } else {
        terrain
    }
}

fn atlas_rect(terrain: u8, mask: u8) -> Rect {
    let index = u32::from(terrain) * MASKS_PER_TERRAIN + u32::from(mask);
    let cell = UVec2::new(index % ATLAS_COLUMNS, index / ATLAS_COLUMNS).as_vec2() * TILE_SIZE;
    Rect::from_corners(cell, cell + Vec2::splat(TILE_SIZE))
}

/// How an item looks in the inventory or as a drop (docs/knowledge/03, "Sprites and icons").
pub struct Icon {
    pub image: Handle<Image>,
    pub rect: Option<Rect>,
    /// Multiplied into the image; white leaves it as drawn.
    pub color: Color,
}

impl Icon {
    pub fn sprite(self, size: f32) -> Sprite {
        Sprite {
            image: self.image,
            rect: self.rect,
            color: self.color,
            custom_size: Some(Vec2::splat(size)),
            ..default()
        }
    }

    pub fn node(self) -> ImageNode {
        ImageNode {
            image: self.image,
            rect: self.rect,
            color: self.color,
            ..default()
        }
    }
}

pub fn item_icon(assets: &AssetServer, sprite: ItemSprite) -> Icon {
    let block = |file: &str| assets.load(format!("blocks/{file}"));
    let (image, rect, color) = match sprite {
        ItemSprite::Terrain(terrain) => (
            assets.load(TERRAIN_ATLAS),
            Some(atlas_rect(terrain, MASK_ALONE)),
            Color::WHITE,
        ),
        ItemSprite::Block(file) => (block(file), None, Color::WHITE),
        ItemSprite::Tinted(file, [r, g, b]) => (block(file), None, Color::srgb_u8(r, g, b)),
        ItemSprite::Equip(name) => (
            assets.load(format!("items/{name}/{EQUIP_ICON_FILE}")),
            None,
            Color::WHITE,
        ),
    };
    Icon { image, rect, color }
}

/// Replaces the sprites of tile `(x, y)` with what the world holds there now. `now` is the clock
/// `apply` runs on; trees are drawn at the growth stage they have at that moment.
fn draw_tile(
    commands: &mut Commands,
    assets: &AssetServer,
    world: &World,
    sprites: &mut TileSprites,
    (x, y): (u32, u32),
    now: f64,
) {
    for entity in sprites.0.remove(&(x, y)).unwrap_or_default() {
        commands.entity(entity).despawn();
    }
    let mut spawned = Vec::new();
    let (cx, cy) = tile_to_world(x, y);
    let at = |z| Transform::from_xyz(cx, cy, z);
    for (layer, layer_z, is_foreground) in [(BG, Z_BACKGROUND, false), (FG, Z_FOREGROUND, true)] {
        let Some(def) = world.get(x, y).and_then(|tile| item(layer(tile))) else {
            continue;
        };
        // A tree's tile holds its seed id; it is drawn as the tree, not as the seed.
        if let Some(tree) = world.tree(x, y).filter(|_| is_foreground) {
            let stage = growth_stage(tree, now);
            let Some(file) = TREE_STAGES.get(usize::from(stage)) else {
                continue;
            };
            let image = assets.load(format!("blocks/{file}"));
            let view = TreeView { x, y, stage };
            spawned.push(
                commands
                    .spawn((Sprite::from_image(image), at(Z_OBJECT), view))
                    .id(),
            );
            let icon = item(tree.item).map(|def| item_icon(assets, def.sprite));
            if let Some(icon) = icon.filter(|_| stage == GROWN_STAGE) {
                let above = Transform::from_xyz(cx, cy + TREE_ICON_OFFSET_Y, Z_TREE_ICON);
                // `farming_ui.rs` adds the ready twinkle to it.
                let ready = crate::farming_ui::ReadyIcon(x, y);
                spawned.push(
                    commands
                        .spawn((icon.sprite(TREE_ICON_SIZE), above, ready))
                        .id(),
                );
            }
            continue;
        }
        let sprite = match def.sprite {
            ItemSprite::Terrain(terrain) => Sprite {
                image: assets.load(TERRAIN_ATLAS),
                rect: Some(atlas_rect(
                    drawn_terrain(world, x, y, terrain),
                    autotile_mask(world, x, y, layer),
                )),
                ..default()
            },
            ItemSprite::Block(_) | ItemSprite::Tinted(..) | ItemSprite::Equip(_) => {
                item_icon(assets, def.sprite).sprite(TILE_SIZE)
            }
        };
        let entity = commands.spawn((sprite, at(layer_z))).id();
        spawned.push(entity);
    }
    if !spawned.is_empty() {
        sprites.0.insert((x, y), spawned);
    }
}

fn draw_world(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    game: Res<Game>,
    mut sprites: ResMut<TileSprites>,
) {
    let now = time.elapsed_secs_f64();
    for y in 0..WORLD_HEIGHT {
        for x in 0..WORLD_WIDTH {
            draw_tile(
                &mut commands,
                &assets,
                &game.0.world,
                &mut sprites,
                (x, y),
                now,
            );
        }
    }
}

/// Redraws the trees whose growth stage moved on since they were last drawn.
fn grow_trees(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    game: Res<Game>,
    mut sprites: ResMut<TileSprites>,
    trees: Query<&TreeView>,
) {
    let now = time.elapsed_secs_f64();
    let world = &game.0.world;
    for view in &trees {
        let stage = world
            .tree(view.x, view.y)
            .map(|tree| growth_stage(tree, now));
        if stage.is_some_and(|stage| stage != view.stage) {
            draw_tile(
                &mut commands,
                &assets,
                world,
                &mut sprites,
                (view.x, view.y),
                now,
            );
        }
    }
}

/// The tile and its four neighbours: a change can alter their autotile mask and grass look.
fn with_neighbours(x: u32, y: u32) -> impl Iterator<Item = (u32, u32)> {
    [(0, 0), (0, 1), (1, 0), (0, -1), (-1, 0)]
        .into_iter()
        .filter_map(move |(dx, dy)| x.checked_add_signed(dx).zip(y.checked_add_signed(dy)))
        .filter(|&(x, y)| x < WORLD_WIDTH && y < WORLD_HEIGHT)
}

/// How broken a tile looks, 0..=1: `damage` is in points (`DAMAGE_PER_HIT` per hit), the tile
/// takes `hits` hits.
fn crack_fraction(damage: u8, hits: u8) -> f32 {
    let health = u16::from(hits.max(1)) * DAMAGE_PER_HIT;
    (f32::from(damage) / f32::from(health)).min(1.0)
}

#[allow(clippy::too_many_arguments)] // one system keeps the events in their original order
fn on_game_events(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut tiles: ResMut<TileSprites>,
    mut drops: ResMut<DropSprites>,
    cracks: Query<(Entity, &Crack)>,
) {
    let world = &game.0.world;
    let now = time.elapsed_secs_f64();
    let clear_crack = |commands: &mut Commands, x: u32, y: u32| {
        for (entity, crack) in &cracks {
            if (crack.x, crack.y) == (x, y) {
                commands.entity(entity).try_despawn();
            }
        }
    };
    for GameEvent(event) in events.read() {
        match *event {
            Event::TileChanged { x, y, .. } => {
                for neighbour in with_neighbours(x, y) {
                    draw_tile(&mut commands, &assets, world, &mut tiles, neighbour, now);
                }
                clear_crack(&mut commands, x, y);
            }
            Event::TreePlanted { x, y, .. }
            | Event::TreeSpliced { x, y, .. }
            | Event::TreeHarvested { x, y } => {
                draw_tile(&mut commands, &assets, world, &mut tiles, (x, y), now);
                clear_crack(&mut commands, x, y);
            }
            Event::TileDamaged {
                x,
                y,
                layer,
                damage,
            } => {
                clear_crack(&mut commands, x, y);
                let def = world.get(x, y).and_then(|tile| {
                    item(match layer {
                        Layer::Foreground => tile.fg,
                        Layer::Background => tile.bg,
                    })
                });
                let is_sapling = layer == Layer::Foreground && world.tree(x, y).is_some();
                let hits = match def {
                    _ if is_sapling => SAPLING_HITS,
                    Some(def) => def.hits,
                    None => 1,
                };
                let alpha = CRACK_MAX_ALPHA * crack_fraction(damage, hits);
                let (cx, cy) = tile_to_world(x, y);
                commands.spawn((
                    Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, alpha), Vec2::splat(TILE_SIZE)),
                    Transform::from_xyz(cx, cy, Z_CRACK),
                    Crack { x, y },
                    // The rules forget the damage after DAMAGE_RESET; so does the picture.
                    Temporary {
                        until: time.elapsed_secs() + DAMAGE_RESET as f32,
                    },
                ));
            }
            Event::ItemDropped { drop } => {
                let Some(entity) = spawn_drop(&mut commands, &assets, &drop) else {
                    continue;
                };
                // A merge re-announces an existing drop: replace its sprite.
                if let Some(old) = drops.0.insert(drop.id, entity) {
                    commands.entity(old).despawn();
                }
            }
            Event::DropTaken { drop_id, .. } => {
                let remains = world.drops.iter().any(|drop| drop.id == drop_id);
                if !remains && let Some(entity) = drops.0.remove(&drop_id) {
                    commands.entity(entity).despawn();
                }
            }
            // A claim block arrives and leaves as a `TileChanged`; its outline is `interact`'s business.
            Event::InventoryChanged { .. }
            | Event::GemsChanged { .. }
            | Event::BackpackUpgraded { .. }
            | Event::InventoryMoved { .. }
            | Event::ClaimPlaced { .. }
            | Event::ClaimRemoved { .. }
            | Event::ClaimUpdated { .. }
            | Event::LeftWorld { .. }
            | Event::Equipped { .. }
            | Event::TradeRequested { .. }
            | Event::TradeUpdated { .. }
            | Event::TradeCompleted { .. }
            | Event::TradeCancelled { .. }
            | Event::XpChanged { .. }
            | Event::LevelUp { .. }
            | Event::Crafted { .. }
            | Event::Respawned { .. } => {}
        }
    }
}

/// A drop's sprite, marked for `vfx.rs`.
fn spawn_drop(commands: &mut Commands, assets: &AssetServer, drop: &Drop) -> Option<Entity> {
    let def = item(drop.item)?;
    let (cx, cy) = drop_position(drop);
    let sprite = item_icon(assets, def.sprite).sprite(DROP_SIZE);
    let view = DropView {
        id: drop.id,
        item: drop.item,
    };
    Some(
        commands
            .spawn((sprite, Transform::from_xyz(cx, cy, Z_DROP), view))
            .id(),
    )
}

/// A whole new world (`WorldReplaced`): the old drops and cracks go, the new world's drops come.
/// The tiles are `draw_world`'s, which replaces each tile's sprites.
fn reset_drops(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    mut drops: ResMut<DropSprites>,
    cracks: Query<Entity, With<Crack>>,
) {
    for (_, entity) in drops.0.drain() {
        commands.entity(entity).try_despawn();
    }
    for entity in &cracks {
        commands.entity(entity).try_despawn();
    }
    for drop in &game.0.world.drops {
        if let Some(entity) = spawn_drop(&mut commands, &assets, drop) {
            drops.0.insert(drop.id, entity);
        }
    }
}

fn expire_temporary(
    mut commands: Commands,
    time: Res<Time>,
    temporary: Query<(Entity, &Temporary)>,
) {
    for (entity, temporary) in &temporary {
        if time.elapsed_secs() >= temporary.until {
            commands.entity(entity).try_despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{DIRT, EMPTY};
    use game_shared::worldgen::{BEDROCK_ROWS, SURFACE_ROW, generate};

    const START_WORLD: &str = "START";

    /// A surface x that is at least two tiles away from the door and the world edges.
    fn plain_x(world: &World) -> u32 {
        if world.door.0 > WORLD_WIDTH / 2 {
            10
        } else {
            WORLD_WIDTH - 10
        }
    }

    fn dirt() -> Tile {
        Tile {
            fg: DIRT,
            bg: EMPTY,
        }
    }

    #[test]
    fn mask_interior_surface_isolated_and_edges() {
        let mut world = generate(START_WORLD);
        let x = plain_x(&world);

        // Bedrock is uniform: interior everywhere, and world edges count as the same material.
        assert_eq!(autotile_mask(&world, x, 2, FG), 15);
        assert_eq!(autotile_mask(&world, 0, 0, FG), 15);
        assert_eq!(autotile_mask(&world, WORLD_WIDTH - 1, 0, FG), 15);
        // Top bedrock row: whatever is above (dirt, stone or lava) is a different material.
        assert_eq!(autotile_mask(&world, x, BEDROCK_ROWS - 1, FG), 14);

        // Surface dirt on a dirt column: open above only.
        world.set(x, SURFACE_ROW - 1, dirt());
        assert_eq!(autotile_mask(&world, x, SURFACE_ROW, FG), 14);
        // Next to the door's bedrock: that side is open too.
        let door_x = world.door.0;
        world.set(door_x + 1, SURFACE_ROW - 1, dirt());
        assert_eq!(
            autotile_mask(&world, door_x + 1, SURFACE_ROW, FG),
            14 & !MASK_LEFT
        );

        // A block floating in the sky.
        world.set(x, SURFACE_ROW + 5, dirt());
        assert_eq!(autotile_mask(&world, x, SURFACE_ROW + 5, FG), 0);

        assert_eq!(autotile_mask(&world, WORLD_WIDTH, 0, FG), 0);
    }

    #[test]
    fn grass_only_on_dirt_open_to_the_sky() {
        let mut world = generate(START_WORLD);
        let x = plain_x(&world);
        assert_eq!(
            drawn_terrain(&world, x, SURFACE_ROW, TERRAIN_DIRT),
            TERRAIN_GRASS
        );
        // Other terrains never turn into grass.
        assert_eq!(drawn_terrain(&world, x, SURFACE_ROW, 2), 2);

        // Underground dirt has a cave wall behind it: digging above it grows no grass.
        world.set(x, SURFACE_ROW, Tile::default());
        assert_eq!(
            drawn_terrain(&world, x, SURFACE_ROW - 1, TERRAIN_DIRT),
            TERRAIN_DIRT
        );

        // Dirt placed in the open air gets a grass top, until something solid sits on it.
        world.set(x, SURFACE_ROW + 3, dirt());
        assert_eq!(
            drawn_terrain(&world, x, SURFACE_ROW + 3, TERRAIN_DIRT),
            TERRAIN_GRASS
        );
        world.set(x, SURFACE_ROW + 4, dirt());
        assert_eq!(
            drawn_terrain(&world, x, SURFACE_ROW + 3, TERRAIN_DIRT),
            TERRAIN_DIRT
        );
    }

    #[test]
    fn atlas_cells() {
        // index 14 -> cell (6, 1); index 16 + 15 = 31 -> cell (7, 3).
        assert_eq!(
            atlas_rect(TERRAIN_GRASS, 14).min,
            Vec2::new(6.0, 1.0) * TILE_SIZE
        );
        assert_eq!(
            atlas_rect(TERRAIN_DIRT, 15).min,
            Vec2::new(7.0, 3.0) * TILE_SIZE
        );
        assert_eq!(atlas_rect(3, 0).size(), Vec2::splat(TILE_SIZE));
    }

    #[test]
    fn a_change_redraws_the_tile_and_its_in_world_neighbours() {
        assert_eq!(with_neighbours(5, 5).count(), 5);
        let corner: Vec<_> = with_neighbours(0, 0).collect();
        assert_eq!(corner, vec![(0, 0), (0, 1), (1, 0)]);
        let far = with_neighbours(WORLD_WIDTH - 1, WORLD_HEIGHT - 1);
        assert_eq!(far.count(), 3);
    }

    #[test]
    fn the_crack_counts_quarter_points_against_hits() {
        // Stone: 6 hits = 24 points. A fist punch (4) is a sixth, a tier-2 pickaxe (5) a bit more.
        assert_eq!(crack_fraction(4, 6), 4.0 / 24.0);
        assert_eq!(crack_fraction(5, 6), 5.0 / 24.0);
        assert_eq!(crack_fraction(24, 6), 1.0);
        assert_eq!(crack_fraction(30, 6), 1.0, "clamped");
        assert_eq!(crack_fraction(4, SAPLING_HITS), 4.0 / 12.0);
        assert_eq!(crack_fraction(2, 0), 0.5, "no hits counts as one");
    }
}
