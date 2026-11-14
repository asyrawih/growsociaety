//! Punch, break, farming and drop feedback (docs/knowledge/ux/vfx-farming-punch.md). View only:
//! everything here reacts to `GameEvent` / `ActionResult` and never touches the game state. Drop
//! and tree sprites are `world_view`'s; this only moves, scales and decorates them.

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;

use bevy::prelude::*;
use bevy::sprite::Text2dShadow;
use bevy::transform::TransformSystems;
use game_shared::items::{EMPTY, ItemId, ItemKind, Sprite as ItemSprite, item, seed_parent};
use game_shared::rules::{Action, Event, PlayerId};
use game_shared::world::{Tile, World};
use game_shared::{TILE_SIZE, WORLD_HEIGHT, WORLD_WIDTH, tile_to_world};

use crate::hud::{FONT, title_case};
use crate::player::{ArmContact, Player};
use crate::settings::Settings;
use crate::state::{ActionResult, Game, GameEvent, Phase, WorldReplaced};
use crate::world_view::{DropView, TreeView, Z_PLAYER, item_icon};

// Art (assets/README.md, "Farming, punch and inventory feedback"): 12 fps strips, centre pivot.
const DUST_IMAGE: &str = "fx/block_break_dust.png";
const DUST_FRAMES: u32 = 6;
const SOIL_IMAGE: &str = "fx/soil_puff.png";
const SOIL_FRAMES: u32 = 5;
const SWIRL_IMAGE: &str = "fx/splice_swirl.png";
const SWIRL_FRAMES: u32 = 8;
const FX_FRAME_SIZE: u32 = 32;
const FX_FRAME_SECS: f32 = 1.0 / 12.0;
const GLOW_IMAGE: &str = "fx/drop_glow.png";
/// The pickup's end: the ready twinkle, 4 frames of 16x16 at 10 fps (as `farming_ui`).
const SPARKLE_IMAGE: &str = "fx/ready_sparkle.png";
const SPARKLE_FRAMES: u32 = 4;
const SPARKLE_SIZE: u32 = 16;
const SPARKLE_FRAME_SECS: f32 = 0.1;

/// In front of the players, like the respawn puff.
const Z_FX: f32 = Z_PLAYER + 0.8;

// Budget.
/// Live particles in total; a burst that would pass it spawns fewer, nothing queues.
pub const MAX_PARTICLES: usize = 200;
/// Reduce effects: particle counts times this.
const REDUCED_PARTICLES: f32 = 0.25;

// Particles: code-drawn 2x2 px squares.
const PARTICLE_SIZE: f32 = 2.0;
const GRAVITY: f32 = 300.0;
const PUNCH_PARTICLES: usize = 4;
const PUNCH_LIFE_SECS: f32 = 0.25;
const BREAK_PARTICLES: usize = 12;
const BREAK_LIFE_SECS: f32 = 0.4;
const SPLICE_PARTICLES: usize = 8;
const SPLICE_LIFE_SECS: f32 = 0.5;
const PARTICLE_SPEED: (f32, f32) = (40.0, 90.0);
/// Radians either side of the spray direction.
const SPRAY_SPREAD: f32 = 0.7;
/// Until a sprite's colour is known.
const FALLBACK_COLOR: Color = Color::srgb(0.6, 0.55, 0.5);
/// Pixels at or above this alpha count towards a block's colour.
const OPAQUE_ALPHA: f32 = 0.5;

// Screen shake: on our own block break only, when the setting is on.
const SHAKE_PX: f32 = 2.0;
const SHAKE_SECS: f32 = 0.1;

// Planting and splicing.
const PLANT_POP: (f32, f32) = (0.6, 1.1);
const POP_SECS: f32 = 0.2;
/// The splice result icon: 0 -> 1.2 -> 1.0 in 0.2 s, held, fading from 0.7 s, gone at 1.0 s.
const SPLICE_ICON_POP: (f32, f32) = (0.0, 1.2);
const SPLICE_ICON_POP_SECS: f32 = 0.2;
const SPLICE_ICON_FADE_FROM_SECS: f32 = 0.7;
const SPLICE_ICON_SECS: f32 = 1.0;
const SPLICE_ICON_SIZE: f32 = 16.0;
/// Never a zero scale (a non-invertible transform).
const MIN_SCALE: f32 = 0.01;
/// Px above the tile centre.
const SPLICE_ICON_RISE: f32 = TILE_SIZE * 0.75;
const SPLICE_COLORS: [Color; 2] = [Color::srgb(0.45, 0.85, 0.35), Color::srgb(0.7, 0.45, 0.95)];

// Drops.
const TOSS_SECS: f32 = 0.25;
const TOSS_PEAK: f32 = 12.0;
/// A drop this close to a player (px) was tossed by them; others hop in place.
const TOSS_REACH: f32 = 2.0 * TILE_SIZE;
const HAND_OFFSET: Vec2 = Vec2::new(0.0, 6.0);
/// Peak to peak.
const BOB_PX: f32 = 3.0;
const BOB_SECS: f32 = 1.2;
const SWAY_DEGREES: f32 = 4.0;
const SWAY_SECS: f32 = 1.8;
const GLOW_ALPHA: (f32, f32) = (0.4, 0.8);

// Pickup.
const FLY_SECS: f32 = 0.2;
const FLY_ICON_SIZE: f32 = 16.0;
const CHEST_OFFSET: Vec2 = Vec2::new(0.0, 4.0);
const TEXT_HEAD_OFFSET: Vec2 = Vec2::new(0.0, 30.0);
const TEXT_RISE: f32 = 16.0;
const TEXT_SECS: f32 = 0.8;
const TEXT_MERGE_SECS: f32 = 0.5;
const TEXT_FONT_SIZE: f32 = 16.0;
/// White text, 1 px dark shadow.
const TEXT_SHADOW: Vec2 = Vec2::new(1.0, -1.0);
const TEXT_SHADOW_COLOR: Color = Color::srgb(0.1, 0.07, 0.05);

/// Seed cells: the parent item's icon in the bottom-left corner (§d).
const BADGE_SIZE: f32 = 12.0;
const BADGE_INSET: f32 = 6.0;

/// What the settings allow (Reduce effects, Screen shake).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Budget {
    particles: f32,
    shake: bool,
    wobble: bool,
    glow: bool,
    /// These carry information and always stay.
    pickup: bool,
    splice_icon: bool,
}

fn budget(reduce: bool, screen_shake: bool) -> Budget {
    Budget {
        particles: if reduce { REDUCED_PARTICLES } else { 1.0 },
        shake: screen_shake && !reduce,
        wobble: !reduce,
        glow: !reduce,
        pickup: true,
        splice_icon: true,
    }
}

/// How many of a `wanted` burst to spawn with `live` particles already up.
fn burst_count(wanted: usize, live: usize, scale: f32) -> usize {
    let scaled = (wanted as f32 * scale).round() as usize;
    scaled.min(MAX_PARTICLES.saturating_sub(live))
}

/// `+1 Dirt Seed`.
fn pickup_text(count: u32, id: ItemId) -> String {
    let name = item(id).map_or("?", |def| def.name);
    format!("+{count} {}", title_case(name))
}

/// 0 at `t = 0`, `peak` at half of `secs`, 1 from `secs` on, straight lines between.
fn pop_scale((from, peak): (f32, f32), secs: f32, t: f32) -> f32 {
    let half = secs / 2.0;
    if t < half {
        from + (peak - from) * (t / half)
    } else if t < secs {
        peak + (1.0 - peak) * ((t - half) / half)
    } else {
        1.0
    }
}

/// The splice result icon's alpha: full, then down to 0 from `SPLICE_ICON_FADE_FROM_SECS`.
fn splice_icon_alpha(t: f32) -> f32 {
    let fade = SPLICE_ICON_SECS - SPLICE_ICON_FADE_FROM_SECS;
    (1.0 - (t - SPLICE_ICON_FADE_FROM_SECS) / fade).clamp(0.0, 1.0)
}

/// A drop's own phase, so a pile never moves in sync.
fn wobble_phase(id: u32) -> f32 {
    (id.wrapping_mul(2_654_435_761) >> 16) as f32 / 65_536.0 * TAU
}

/// Bob (px, up) and sway (radians) of drop `id` at time `now`.
fn wobble(id: u32, now: f32) -> (f32, f32) {
    let phase = wobble_phase(id);
    let bob = BOB_PX / 2.0 * (TAU * now / BOB_SECS + phase).sin();
    let sway = SWAY_DEGREES.to_radians() * (TAU * now / SWAY_SECS + phase * 1.7).sin();
    (bob, sway)
}

/// The toss arc, `t` in 0..=1: a straight line from `from` to `to` lifted by a parabola.
fn toss_position(from: Vec2, to: Vec2, t: f32) -> Vec2 {
    from.lerp(to, t) + Vec2::Y * 4.0 * TOSS_PEAK * t * (1.0 - t)
}

/// The parent item's icon, small, in a seed cell's bottom-left corner. `None` for non-seeds.
pub(crate) fn seed_badge(assets: &AssetServer, id: ItemId) -> Option<impl Bundle> {
    let parent = item(seed_parent(id)?)?;
    let node = Node {
        position_type: PositionType::Absolute,
        left: px(BADGE_INSET),
        bottom: px(BADGE_INSET),
        width: px(BADGE_SIZE),
        height: px(BADGE_SIZE),
        ..default()
    };
    Some((item_icon(assets, parent.sprite).node(), node))
}

#[derive(Resource)]
struct FxArt {
    dust: (Handle<Image>, Handle<TextureAtlasLayout>),
    soil: (Handle<Image>, Handle<TextureAtlasLayout>),
    swirl: (Handle<Image>, Handle<TextureAtlasLayout>),
    sparkle: (Handle<Image>, Handle<TextureAtlasLayout>),
    glow: Handle<Image>,
    font: Handle<Font>,
}

/// What the effects remember between frames.
#[derive(Resource, Default)]
struct FxMemory {
    /// The world's tiles as last seen, to tell a break and what broke (as `audio.rs`).
    tiles: Option<(String, Vec<Tile>)>,
    /// Every known drop: item and position, for the pickup fly after it is gone.
    drops: HashMap<u32, (ItemId, Vec2)>,
    /// Drops re-announced by a merge: no second toss.
    merged: HashSet<u32>,
    /// Drops out of a broken block or a harvest: they hop in place, nobody tossed them.
    hops: HashSet<u32>,
    /// Trees planted or spliced this frame whose new sprite should pop.
    pops: HashSet<(u32, u32)>,
    /// Average colour per item sprite.
    colours: HashMap<ItemId, Color>,
    shake_until: f32,
    seed: u32,
}

impl FxMemory {
    /// xorshift: 0..1.
    fn random(&mut self) -> f32 {
        let mut x = self.seed.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

#[derive(Component)]
struct Particle {
    vel: Vec2,
    born: f32,
    life: f32,
}

/// A sprite strip playing once.
#[derive(Component)]
struct Strip {
    started: f32,
    frames: usize,
    frame_secs: f32,
}

#[derive(Component)]
struct Pop {
    started: f32,
}

#[derive(Component)]
struct SpliceIcon {
    started: f32,
}

#[derive(Component)]
struct Wobble {
    id: u32,
    base: Vec3,
    from: Vec2,
    born: f32,
}

#[derive(Component)]
struct Glow;

#[derive(Component)]
struct Fly {
    from: Vec2,
    to: PlayerId,
    started: f32,
}

#[derive(Component)]
struct FloatText {
    item: ItemId,
    count: u32,
    started: f32,
    base: Vec2,
}

pub struct VfxPlugin;

impl Plugin for VfxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FxMemory>()
            .add_systems(Startup, load_art)
            .add_systems(
                Update,
                (on_events, animate_particles, animate_sprites, animate_drops)
                    .chain()
                    .in_set(Phase::View),
            )
            .add_systems(
                PostUpdate,
                (attach_pops, attach_drops, shake_camera).before(TransformSystems::Propagate),
            );
    }
}

fn load_art(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let mut strip = |file: &'static str, size: u32, frames: u32| {
        let layout = TextureAtlasLayout::from_grid(UVec2::splat(size), frames, 1, None, None);
        (assets.load(file), layouts.add(layout))
    };
    let art = FxArt {
        dust: strip(DUST_IMAGE, FX_FRAME_SIZE, DUST_FRAMES),
        soil: strip(SOIL_IMAGE, FX_FRAME_SIZE, SOIL_FRAMES),
        swirl: strip(SWIRL_IMAGE, FX_FRAME_SIZE, SWIRL_FRAMES),
        sparkle: strip(SPARKLE_IMAGE, SPARKLE_SIZE, SPARKLE_FRAMES),
        glow: assets.load(GLOW_IMAGE),
        font: assets.load(FONT),
    };
    commands.insert_resource(art);
}

fn strip_bundle(
    (image, layout): &(Handle<Image>, Handle<TextureAtlasLayout>),
    frames: u32,
    frame_secs: f32,
    at: Vec2,
    color: Color,
    now: f32,
) -> impl Bundle {
    let mut sprite = Sprite::from_atlas_image(
        image.clone(),
        TextureAtlas {
            layout: layout.clone(),
            index: 0,
        },
    );
    sprite.color = color;
    (
        sprite,
        Transform::from_translation(at.extend(Z_FX)),
        Strip {
            started: now,
            frames: frames as usize,
            frame_secs,
        },
    )
}

/// The average opaque colour of an item's sprite (seeds: their tint), cached once the image is in.
fn block_colour(
    id: ItemId,
    assets: &AssetServer,
    images: &Assets<Image>,
    cache: &mut HashMap<ItemId, Color>,
) -> Color {
    let Some(def) = item(id) else {
        return FALLBACK_COLOR;
    };
    if let ItemSprite::Tinted(_, [r, g, b]) = def.sprite {
        return Color::srgb_u8(r, g, b);
    }
    if let Some(&colour) = cache.get(&id) {
        return colour;
    }
    let icon = item_icon(assets, def.sprite);
    let Some(image) = images.get(&icon.image) else {
        return FALLBACK_COLOR;
    };
    let size = image.size().as_vec2();
    let rect = icon.rect.unwrap_or(Rect::from_corners(Vec2::ZERO, size));
    let (mut sum, mut n) = (Vec3::ZERO, 0.0);
    for y in rect.min.y as u32..rect.max.y as u32 {
        for x in rect.min.x as u32..rect.max.x as u32 {
            let Ok(pixel) = image.get_color_at(x, y) else {
                continue;
            };
            let pixel = pixel.to_srgba();
            if pixel.alpha >= OPAQUE_ALPHA {
                sum += Vec3::new(pixel.red, pixel.green, pixel.blue);
                n += 1.0;
            }
        }
    }
    let colour = if n > 0.0 {
        let avg = sum / n;
        Color::srgb(avg.x, avg.y, avg.z)
    } else {
        FALLBACK_COLOR
    };
    cache.insert(id, colour);
    colour
}

/// Whether `pos` is in (or a tile beside) the camera's view.
fn on_screen(camera: (&GlobalTransform, &Projection), pos: Vec2) -> bool {
    let (transform, projection) = camera;
    let Projection::Orthographic(ortho) = projection else {
        return true;
    };
    let half = ortho.area.half_size() + Vec2::splat(TILE_SIZE);
    let d = (pos - transform.translation().truncate()).abs();
    d.x <= half.x && d.y <= half.y
}

/// Where player `id` is: the local one interpolated, others from the state.
fn player_pos(game: &Game, local: &Player, id: PlayerId) -> Option<Vec2> {
    if id == game.1 {
        return Some(local.position);
    }
    game.0.players.get(&id).map(|p| Vec2::from(p.body.pos))
}

fn snapshot(world: &World) -> Vec<Tile> {
    let keys = 0..WORLD_WIDTH * WORLD_HEIGHT;
    let tile = |key| world.get(key % WORLD_WIDTH, key / WORLD_WIDTH);
    keys.map(|key| tile(key).unwrap_or_default()).collect()
}

/// The item a tile change removed, if it was a break (a layer went empty).
fn broken_item(before: Tile, after: Tile) -> Option<ItemId> {
    if before.fg != EMPTY && after.fg == EMPTY {
        Some(before.fg)
    } else if before.bg != EMPTY && after.bg == EMPTY {
        Some(before.bg)
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)] // where, which way, how many, what colour, when
fn spawn_burst(
    commands: &mut Commands,
    memory: &mut FxMemory,
    live: &mut usize,
    scale: f32,
    (wanted, life): (usize, f32),
    at: Vec2,
    dir: Vec2,
    colours: &[Color],
    now: f32,
) {
    let n = burst_count(wanted, *live, scale);
    *live += n;
    for i in 0..n {
        let angle = (memory.random() * 2.0 - 1.0) * SPRAY_SPREAD;
        let speed = PARTICLE_SPEED.0 + memory.random() * (PARTICLE_SPEED.1 - PARTICLE_SPEED.0);
        let vel = Vec2::from_angle(angle).rotate(dir) * speed;
        commands.spawn((
            Sprite::from_color(colours[i % colours.len()], Vec2::splat(PARTICLE_SIZE)),
            Transform::from_translation(at.extend(Z_FX)),
            Particle {
                vel,
                born: now,
                life,
            },
        ));
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per thing an effect needs
fn on_events(
    mut commands: Commands,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    time: Res<Time>,
    game: Res<Game>,
    settings: Res<Settings>,
    art: Res<FxArt>,
    local: Single<&Player>,
    camera: Single<(&GlobalTransform, &Projection), With<Camera2d>>,
    particles: Query<(), With<Particle>>,
    mut texts: Query<(&mut FloatText, &mut Text2d)>,
    mut events: MessageReader<GameEvent>,
    mut results: MessageReader<ActionResult>,
    mut replaced: MessageReader<WorldReplaced>,
    mut contacts: MessageReader<ArmContact>,
    mut memory: ResMut<FxMemory>,
) {
    let now = time.elapsed_secs();
    let fx = budget(settings.reduce_effects, settings.screen_shake);
    let camera = *camera;
    let world = &game.0.world;
    let memory = &mut *memory;
    memory.seed = memory.seed.wrapping_add(time.elapsed().subsec_nanos() | 1);
    // A whole new world (same name): forget the old tiles and drops; the redrawn drops don't toss.
    if replaced.read().count() > 0 {
        memory.tiles = None;
        memory.drops.clear();
        memory.merged = world.drops.iter().map(|drop| drop.id).collect();
    }
    let mut live = particles.iter().count();
    let mut tiles = match memory.tiles.take() {
        Some((name, tiles)) if name == world.name => tiles,
        _ => snapshot(world),
    };
    let events: Vec<&Event> = events.read().map(|GameEvent(event)| event).collect();
    // A harvest clears the tile too; its drops are its effect.
    let harvested: HashSet<(u32, u32)> = events
        .iter()
        .filter_map(|event| match **event {
            Event::TreeHarvested { x, y } => Some((x, y)),
            _ => None,
        })
        .collect();
    // A drop on a tile that changed in the same batch came out of it (break, harvest).
    let changed: HashSet<(u32, u32)> = events
        .iter()
        .filter_map(|event| match **event {
            Event::TileChanged { x, y, .. } => Some((x, y)),
            _ => None,
        })
        .collect();
    for event in events {
        match *event {
            Event::TileChanged { x, y, tile } => {
                let Some(seen) = World::tile_key(x, y).and_then(|key| tiles.get_mut(key as usize))
                else {
                    continue;
                };
                let before = std::mem::replace(seen, tile);
                let (cx, cy) = tile_to_world(x, y);
                let centre = Vec2::new(cx, cy);
                let Some(id) = broken_item(before, tile)
                    .filter(|_| !harvested.contains(&(x, y)) && on_screen(camera, centre))
                else {
                    continue;
                };
                let colour = block_colour(id, &assets, &images, &mut memory.colours);
                let burst = (BREAK_PARTICLES, BREAK_LIFE_SECS);
                spawn_burst(
                    &mut commands,
                    memory,
                    &mut live,
                    fx.particles,
                    burst,
                    centre,
                    Vec2::Y,
                    &[colour],
                    now,
                );
                commands.spawn(strip_bundle(
                    &art.dust,
                    DUST_FRAMES,
                    FX_FRAME_SECS,
                    centre,
                    colour,
                    now,
                ));
            }
            Event::TreePlanted { x, y, .. } => {
                let (cx, cy) = tile_to_world(x, y);
                memory.pops.insert((x, y));
                if on_screen(camera, Vec2::new(cx, cy)) {
                    // The puff's ground line is its frame's bottom edge: on the tile's base.
                    let at = Vec2::new(cx, cy);
                    let soil =
                        strip_bundle(&art.soil, SOIL_FRAMES, FX_FRAME_SECS, at, Color::WHITE, now);
                    commands.spawn(soil);
                }
            }
            Event::TreeSpliced { x, y, ref tree, .. } => {
                let (cx, cy) = tile_to_world(x, y);
                let centre = Vec2::new(cx, cy);
                memory.pops.insert((x, y));
                if !on_screen(camera, centre) {
                    continue;
                }
                let swirl = strip_bundle(
                    &art.swirl,
                    SWIRL_FRAMES,
                    FX_FRAME_SECS,
                    centre,
                    Color::WHITE,
                    now,
                );
                commands.spawn(swirl);
                let burst = (SPLICE_PARTICLES, SPLICE_LIFE_SECS);
                spawn_burst(
                    &mut commands,
                    memory,
                    &mut live,
                    fx.particles,
                    burst,
                    centre,
                    Vec2::Y,
                    &SPLICE_COLORS,
                    now,
                );
                if let Some(def) = item(tree.item).filter(|_| fx.splice_icon) {
                    let above = centre + Vec2::Y * SPLICE_ICON_RISE;
                    commands.spawn((
                        item_icon(&assets, def.sprite).sprite(SPLICE_ICON_SIZE),
                        Transform::from_translation(above.extend(Z_FX))
                            .with_scale(Vec3::splat(MIN_SCALE)),
                        SpliceIcon { started: now },
                    ));
                }
            }
            Event::ItemDropped { ref drop } => {
                let (cx, cy) = game_shared::rules::drop_position(&drop);
                if memory
                    .drops
                    .insert(drop.id, (drop.item, Vec2::new(cx, cy)))
                    .is_some()
                {
                    memory.merged.insert(drop.id);
                }
                if changed.contains(&(drop.x, drop.y)) {
                    memory.hops.insert(drop.id);
                }
            }
            Event::DropTaken { drop_id, by, count } => {
                let Some(&(id, at)) = memory.drops.get(&drop_id) else {
                    continue;
                };
                if !world.drops.iter().any(|drop| drop.id == drop_id) {
                    memory.drops.remove(&drop_id);
                }
                if !fx.pickup || !on_screen(camera, at) {
                    continue;
                }
                if let Some(def) = item(id) {
                    commands.spawn((
                        item_icon(&assets, def.sprite).sprite(FLY_ICON_SIZE),
                        Transform::from_translation(at.extend(Z_FX)),
                        Fly {
                            from: at,
                            to: by,
                            started: now,
                        },
                    ));
                }
                if by != game.1 {
                    continue;
                }
                let count = u32::from(count);
                let merge = texts
                    .iter_mut()
                    .find(|(text, _)| text.item == id && now - text.started < TEXT_MERGE_SECS);
                if let Some((mut text, mut label)) = merge {
                    text.count += count;
                    text.started = now;
                    label.0 = pickup_text(text.count, id);
                    continue;
                }
                let base = local.position + TEXT_HEAD_OFFSET;
                let mut font = TextFont::from_font_size(TEXT_FONT_SIZE);
                font.font = FontSource::Handle(art.font.clone());
                commands.spawn((
                    Text2d::new(pickup_text(count, id)),
                    font,
                    TextColor(Color::WHITE),
                    Text2dShadow {
                        offset: TEXT_SHADOW,
                        color: TEXT_SHADOW_COLOR,
                    },
                    Transform::from_translation(base.extend(Z_FX)),
                    FloatText {
                        item: id,
                        count,
                        started: now,
                        base,
                    },
                ));
            }
            _ => {}
        }
    }
    memory.tiles = Some((world.name.clone(), tiles));
    // Hit particles at the moment a fist reaches its tile (`player::ArmContact`), sprayed away
    // from that player. A block that broke on this punch is already gone: its break burst shows.
    for contact in contacts.read().filter(|contact| !contact.place) {
        let (cx, cy) = tile_to_world(contact.x, contact.y);
        let centre = Vec2::new(cx, cy);
        let Some(tile) = world.get(contact.x, contact.y) else {
            continue;
        };
        let id = if tile.fg != EMPTY { tile.fg } else { tile.bg };
        if id == EMPTY || !on_screen(camera, centre) {
            continue;
        }
        let colour = block_colour(id, &assets, &images, &mut memory.colours);
        let from = player_pos(&game, *local, contact.player).unwrap_or(centre);
        let away = (centre - from).normalize_or(Vec2::Y);
        let hit = centre - away * TILE_SIZE / 2.0;
        let burst = (PUNCH_PARTICLES, PUNCH_LIFE_SECS);
        spawn_burst(
            &mut commands,
            memory,
            &mut live,
            fx.particles,
            burst,
            hit,
            away,
            &[colour],
            now,
        );
    }
    // Drops that were there before we came (no `ItemDropped` seen; after the events, so a new
    // drop's own `ItemDropped` is not taken for a merge), for the pickup fly.
    for drop in &world.drops {
        let at = game_shared::rules::drop_position(drop);
        memory
            .drops
            .entry(drop.id)
            .or_insert((drop.item, Vec2::from(at)));
    }
    // Shake on our own break: a punch whose answer changed a tile.
    for ActionResult { action, result } in results.read() {
        let broke = matches!(action, Action::Punch { .. })
            && result.as_ref().is_ok_and(|events| {
                events
                    .iter()
                    .any(|event| matches!(event, Event::TileChanged { .. }))
            });
        if broke && fx.shake {
            memory.shake_until = now + SHAKE_SECS;
        }
    }
}

fn animate_particles(
    mut commands: Commands,
    time: Res<Time>,
    mut particles: Query<(Entity, &Particle, &mut Transform, &mut Sprite)>,
) {
    let (now, dt) = (time.elapsed_secs(), time.delta_secs());
    for (entity, particle, mut transform, mut sprite) in &mut particles {
        let age = now - particle.born;
        if age >= particle.life {
            commands.entity(entity).despawn();
            continue;
        }
        let vel = particle.vel - Vec2::Y * GRAVITY * age;
        transform.translation += (vel * dt).extend(0.0);
        sprite.color.set_alpha(1.0 - age / particle.life);
    }
}

/// Strips, pops, the splice icon, pickup flies and floating texts.
#[allow(clippy::too_many_arguments)] // a Bevy system: one query per kind of effect
fn animate_sprites(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    art: Res<FxArt>,
    local: Single<&Player>,
    mut strips: Query<(Entity, &Strip, &mut Sprite), Without<SpliceIcon>>,
    mut pops: Query<
        (Entity, &Pop, &mut Transform),
        (Without<SpliceIcon>, Without<Fly>, Without<FloatText>),
    >,
    mut icons: Query<
        (Entity, &SpliceIcon, &mut Transform, &mut Sprite),
        (Without<Fly>, Without<FloatText>),
    >,
    mut flies: Query<(Entity, &Fly, &mut Transform), Without<FloatText>>,
    mut texts: Query<(
        Entity,
        &FloatText,
        &mut Transform,
        &mut TextColor,
        &mut Text2dShadow,
    )>,
) {
    let now = time.elapsed_secs();
    for (entity, strip, mut sprite) in &mut strips {
        let frame = ((now - strip.started) / strip.frame_secs) as usize;
        if frame >= strip.frames {
            commands.entity(entity).despawn();
        } else if let Some(atlas) = sprite.texture_atlas.as_mut() {
            atlas.index = frame;
        }
    }
    for (entity, pop, mut transform) in &mut pops {
        let t = now - pop.started;
        transform.scale = Vec3::splat(pop_scale(PLANT_POP, POP_SECS, t));
        if t >= POP_SECS {
            commands.entity(entity).remove::<Pop>();
        }
    }
    for (entity, icon, mut transform, mut sprite) in &mut icons {
        let t = now - icon.started;
        if t >= SPLICE_ICON_SECS {
            commands.entity(entity).despawn();
            continue;
        }
        let scale = pop_scale(SPLICE_ICON_POP, SPLICE_ICON_POP_SECS, t).max(MIN_SCALE);
        transform.scale = Vec3::splat(scale);
        sprite.color.set_alpha(splice_icon_alpha(t));
    }
    for (entity, fly, mut transform) in &mut flies {
        let t = (now - fly.started) / FLY_SECS;
        let target = player_pos(&game, *local, fly.to).unwrap_or(fly.from) + CHEST_OFFSET;
        if t >= 1.0 {
            commands.entity(entity).despawn();
            commands.spawn(strip_bundle(
                &art.sparkle,
                SPARKLE_FRAMES,
                SPARKLE_FRAME_SECS,
                target,
                Color::WHITE,
                now,
            ));
            continue;
        }
        let at = fly.from.lerp(target, t * t);
        transform.translation = at.extend(transform.translation.z);
    }
    for (entity, text, mut transform, mut colour, mut shadow) in &mut texts {
        let t = (now - text.started) / TEXT_SECS;
        if t >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let at = text.base + Vec2::Y * TEXT_RISE * t;
        transform.translation = at.extend(transform.translation.z);
        colour.0.set_alpha(1.0 - t);
        shadow.color.set_alpha(1.0 - t);
    }
}

/// Toss, then bob and sway (Reduce effects: still); the seed glow pulses under it.
fn animate_drops(
    time: Res<Time>,
    settings: Res<Settings>,
    mut drops: Query<(&Wobble, &mut Transform)>,
    mut glows: Query<(&mut Sprite, &mut Visibility), With<Glow>>,
) {
    let now = time.elapsed_secs();
    let fx = budget(settings.reduce_effects, settings.screen_shake);
    for (wobble_of, mut transform) in &mut drops {
        let t = (now - wobble_of.born) / TOSS_SECS;
        let base = wobble_of.base.truncate();
        let (at, angle) = if t < 1.0 {
            (toss_position(wobble_of.from, base, t), 0.0)
        } else if fx.wobble {
            let (bob, sway) = wobble(wobble_of.id, now);
            (base + Vec2::Y * bob, sway)
        } else {
            (base, 0.0)
        };
        transform.translation = at.extend(wobble_of.base.z);
        transform.rotation = Quat::from_rotation_z(angle);
    }
    let pulse = (0.5 + 0.5 * (TAU * now / BOB_SECS).sin()) * (GLOW_ALPHA.1 - GLOW_ALPHA.0);
    let shown = if fx.glow {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (mut sprite, mut visibility) in &mut glows {
        sprite.color.set_alpha(GLOW_ALPHA.0 + pulse);
        visibility.set_if_neq(shown);
    }
}

/// A sapling sprite that was just planted or spliced pops.
fn attach_pops(
    mut commands: Commands,
    time: Res<Time>,
    mut memory: ResMut<FxMemory>,
    trees: Query<(Entity, &TreeView), Added<TreeView>>,
) {
    for (entity, tree) in &trees {
        if memory.pops.remove(&(tree.x, tree.y)) {
            let started = time.elapsed_secs();
            commands.entity(entity).insert(Pop { started });
        }
    }
    memory.pops.clear();
}

/// A new drop sprite: tossed from the hand of the player beside it (or hopping in place), then
/// wobbling; a seed gets the glow. A merge's replacement sprite skips the toss.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn attach_drops(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    art: Res<FxArt>,
    local: Single<&Player>,
    mut memory: ResMut<FxMemory>,
    mut drops: Query<(Entity, &DropView, &mut Transform), Added<DropView>>,
) {
    let now = time.elapsed_secs();
    for (entity, view, mut transform) in &mut drops {
        let base = transform.translation;
        let landing = base.truncate();
        let merged = memory.merged.remove(&view.id);
        let hop = memory.hops.remove(&view.id);
        let thrower = std::iter::once(local.position)
            .chain(
                game.0
                    .players
                    .iter()
                    .filter(|(id, _)| **id != game.1)
                    .map(|(_, p)| Vec2::from(p.body.pos)),
            )
            .filter(|pos| !hop && pos.distance(landing) <= TOSS_REACH)
            .min_by(|a, b| a.distance(landing).total_cmp(&b.distance(landing)));
        let from = thrower.map_or(landing, |pos| pos + HAND_OFFSET);
        let born = if merged { now - TOSS_SECS } else { now };
        if !merged {
            transform.translation = from.extend(base.z);
        }
        let mut drop = commands.entity(entity);
        drop.insert(Wobble {
            id: view.id,
            base,
            from,
            born,
        });
        if item(view.item).is_some_and(|def| def.kind == ItemKind::Seed) {
            drop.with_child((
                Sprite::from_image(art.glow.clone()),
                Transform::from_xyz(0.0, 0.0, -0.05),
                Glow,
            ));
        }
    }
}

/// A visual offset on the camera after `camera.rs` has placed it; `follow_player` sets the
/// position from scratch every frame, so the offset never feeds back into its logic.
fn shake_camera(
    time: Res<Time>,
    mut memory: ResMut<FxMemory>,
    camera: Single<(&mut Transform, &Projection), With<Camera2d>>,
) {
    if time.elapsed_secs() >= memory.shake_until {
        return;
    }
    let (mut transform, projection) = camera.into_inner();
    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    let offset = Vec2::new(memory.random() * 2.0 - 1.0, memory.random() * 2.0 - 1.0);
    transform.translation += (offset * SHAKE_PX * scale).extend(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{DIRT, seed_of};

    #[test]
    fn bursts_respect_the_global_cap_and_reduce_effects() {
        let full = budget(false, false);
        let reduced = budget(true, false);
        assert_eq!(burst_count(BREAK_PARTICLES, 0, full.particles), 12);
        assert_eq!(burst_count(BREAK_PARTICLES, 0, reduced.particles), 3);
        assert_eq!(burst_count(PUNCH_PARTICLES, 0, reduced.particles), 1);
        // Near the cap a burst spawns fewer; at the cap none, nothing queues.
        assert_eq!(burst_count(BREAK_PARTICLES, MAX_PARTICLES - 5, 1.0), 5);
        assert_eq!(burst_count(BREAK_PARTICLES, MAX_PARTICLES, 1.0), 0);
        assert_eq!(burst_count(BREAK_PARTICLES, MAX_PARTICLES + 9, 1.0), 0);
    }

    #[test]
    fn reduce_effects_keeps_what_carries_information() {
        let reduced = budget(true, true);
        assert!(reduced.splice_icon, "the splice result icon always pops");
        assert!(reduced.pickup, "the pickup fly and text always show");
        assert!(!reduced.shake && !reduced.wobble && !reduced.glow);
        let full = budget(false, false);
        assert!(!full.shake, "shake is off unless the setting is on");
        assert!(budget(false, true).shake);
        assert!(full.wobble && full.glow && full.splice_icon);
    }

    #[test]
    fn the_pickup_text_names_the_item_in_title_case() {
        let seed = seed_of(DIRT).unwrap();
        let name = title_case(item(seed).unwrap().name);
        assert_eq!(pickup_text(1, seed), format!("+1 {name}"));
        assert_eq!(pickup_text(3, DIRT), "+3 Dirt");
    }

    #[test]
    fn pops_go_over_the_peak_and_settle_at_one() {
        assert_eq!(pop_scale(PLANT_POP, POP_SECS, 0.0), 0.6);
        assert!((pop_scale(PLANT_POP, POP_SECS, POP_SECS / 2.0) - 1.1).abs() < 1e-5);
        assert_eq!(pop_scale(PLANT_POP, POP_SECS, POP_SECS), 1.0);
        assert_eq!(pop_scale(SPLICE_ICON_POP, SPLICE_ICON_POP_SECS, 0.0), 0.0);
        let peak = pop_scale(SPLICE_ICON_POP, SPLICE_ICON_POP_SECS, 0.1);
        assert!((peak - 1.2).abs() < 1e-5);
        // Held at full alpha, gone by 1 s.
        assert_eq!(splice_icon_alpha(0.5), 1.0);
        assert!(splice_icon_alpha(0.85) < 1.0);
        assert_eq!(splice_icon_alpha(SPLICE_ICON_SECS), 0.0);
    }

    #[test]
    fn drops_wobble_out_of_sync_and_toss_in_an_arc() {
        assert_ne!(wobble_phase(1), wobble_phase(2));
        assert_ne!(wobble(1, 0.3), wobble(2, 0.3));
        let (bob, sway) = wobble(7, 0.9);
        assert!(bob.abs() <= BOB_PX / 2.0 + 1e-5);
        assert!(sway.abs() <= SWAY_DEGREES.to_radians() + 1e-5);
        let (from, to) = (Vec2::ZERO, Vec2::new(20.0, 0.0));
        assert_eq!(toss_position(from, to, 0.0), from);
        assert_eq!(toss_position(from, to, 1.0), to);
        assert_eq!(toss_position(from, to, 0.5), Vec2::new(10.0, TOSS_PEAK));
    }

    #[test]
    fn a_break_is_a_layer_going_empty() {
        let dirt = Tile {
            fg: DIRT,
            bg: EMPTY,
        };
        assert_eq!(broken_item(dirt, Tile::default()), Some(DIRT));
        assert_eq!(broken_item(Tile::default(), dirt), None);
    }
}
