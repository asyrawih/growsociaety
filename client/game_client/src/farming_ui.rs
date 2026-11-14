//! Farming feedback (docs/knowledge/ux/farming-feedback.md), all from the mirrored `Tree`: the
//! plant and splice toasts with the grow time, the tree card (hover, long-press, or standing in
//! front of a tree) and the twinkle on grown trees' icons. No messages of its own.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use game_shared::farming::{Tree, grow_seconds, is_grown};
use game_shared::items::{ItemId, item};
use game_shared::mods::worn_mods;
use game_shared::physics::HITBOX_HEIGHT;
use game_shared::rules::{Event, in_reach_with};
use game_shared::{TILE_SIZE, tile_to_world, world_to_tile};

use crate::hud::{
    HUD_MARGIN, Notice, OpenPanel, SEP, SLOT_GAP, TOOLTIP_DELAY_SECS, TOOLTIP_IMAGE, TOOLTIP_INSET,
    TOOLTIP_MAX_WIDTH, duration_text, ink, item_name, sliced, tip_part, title_case,
};
use crate::interact::HoveredTile;
use crate::player::Player;
use crate::state::{ActionResult, Game, Phase};

/// The ready twinkle: `fx/ready_sparkle.png`, 4 frames of 16x16 at 10 fps, once per period.
const SPARKLE_IMAGE: &str = "fx/ready_sparkle.png";
const SPARKLE_FRAMES: usize = 4;
const SPARKLE_SIZE: u32 = 16;
const SPARKLE_FRAME_SECS: f32 = 0.1;
const SPARKLE_PERIOD_SECS: f32 = 4.0;
/// In front of the icon it sits on.
const SPARKLE_Z: f32 = 0.01;

/// Long-press on a tree opens its card, pinned until a tap elsewhere. Shorter is a tap, which
/// punches a tree on release (`interact.rs`).
pub(crate) const LONG_PRESS_SECS: f32 = 0.4;
/// A press that moves further than this is a drag, not a long-press, logical px.
const TOUCH_SLOP: f32 = 12.0;
/// Standing still this long shows the card of the tree in front of the player.
const STILL_SECS: f32 = 0.5;
/// Moving less than this between frames counts as standing still, px.
const STILL_EPSILON: f32 = 0.5;

/// The card's progress bar, logical px.
const BAR_SIZE: Vec2 = Vec2::new(120.0, 8.0);
const BAR_GROOVE: Color = Color::srgb(0.2, 0.13, 0.08);
const BAR_FILL: Color = Color::srgb(0.3, 0.72, 0.2);
/// Between the tree and its card, logical px.
const CARD_GAP: f32 = 6.0;

/// The item icon over a grown tree at tile `(x, y)`, spawned by `world_view.rs`.
#[derive(Component)]
pub struct ReadyIcon(pub u32, pub u32);

/// A twinkle, child of a `ReadyIcon`; `offset` spreads a farm's twinkles over the period.
#[derive(Component)]
struct Sparkle {
    offset: f32,
}

#[derive(Resource)]
struct SparkleArt {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
}

#[derive(Component)]
struct TreeCard;

/// What the card says, rebuilt only when it changes.
#[derive(Clone, PartialEq, Debug)]
struct CardText {
    title: String,
    /// 0..=1, moving once per second.
    fill: f32,
    /// `NN %` while growing.
    percent: Option<u32>,
    status: String,
}

#[derive(Default)]
struct CardInput {
    /// The hovered tile and since when.
    hover: Option<((u32, u32), f32)>,
    /// A tree card pinned by a long-press.
    pinned: Option<(u32, u32)>,
    /// When each current touch started.
    touch_started: HashMap<u64, f32>,
    /// The player's last position and since when it has not moved.
    still: Option<(Vec2, f32)>,
    shown: Option<CardText>,
}

pub struct FarmingUiPlugin;

impl Plugin for FarmingUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_card).add_systems(
            Update,
            (farm_toasts, add_sparkles, twinkle, show_card).in_set(Phase::View),
        );
    }
}

fn spawn_card(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let layout = TextureAtlasLayout::from_grid(
        UVec2::splat(SPARKLE_SIZE),
        SPARKLE_FRAMES as u32,
        1,
        None,
        None,
    );
    commands.insert_resource(SparkleArt {
        image: assets.load(SPARKLE_IMAGE),
        layout: layouts.add(layout),
    });
    // The shared tooltip's look (item-tooltip.md), its own box so both can show at once.
    commands.spawn((
        TreeCard,
        Node {
            position_type: PositionType::Absolute,
            max_width: px(TOOLTIP_MAX_WIDTH),
            padding: UiRect::all(px(TOOLTIP_INSET * crate::hud::KIT_SCALE)),
            flex_direction: FlexDirection::Column,
            row_gap: px(SLOT_GAP),
            display: Display::None,
            ..default()
        },
        sliced(&assets, TOOLTIP_IMAGE, TOOLTIP_INSET),
        GlobalZIndex(i32::MAX - 2),
        tip_part(),
    ));
}

fn grow_text(id: ItemId) -> String {
    duration_text(item(id).map_or(0, |def| grow_seconds(def.rarity)))
}

fn name(id: ItemId) -> String {
    title_case(item_name(id))
}

/// The planter's own toast for a plant or splice.
fn farm_toast(event: &Event) -> Option<String> {
    match *event {
        Event::TreePlanted { tree, .. } => Some(format!(
            "Planted {} seed{SEP}ready in {}",
            name(tree.item),
            grow_text(tree.item)
        )),
        Event::TreeSpliced { tree, a, b, .. } => Some(format!(
            "{} + {} spliced into {} tree{SEP}ready in {}",
            name(a),
            name(b),
            name(tree.item),
            grow_text(tree.item)
        )),
        _ => None,
    }
}

/// Only the local player's own actions: other players' farming gets no toast.
fn farm_toasts(mut results: MessageReader<ActionResult>, mut notices: MessageWriter<Notice>) {
    for ActionResult { result, .. } in results.read() {
        for text in result.iter().flatten().filter_map(farm_toast) {
            notices.write(Notice(text));
        }
    }
}

fn add_sparkles(
    mut commands: Commands,
    art: Res<SparkleArt>,
    icons: Query<(Entity, &ReadyIcon), Added<ReadyIcon>>,
) {
    for (entity, &ReadyIcon(x, y)) in &icons {
        let atlas = TextureAtlas {
            layout: art.layout.clone(),
            index: 0,
        };
        commands.entity(entity).with_child((
            Sparkle {
                offset: sparkle_offset(x, y),
            },
            Sprite::from_atlas_image(art.image.clone(), atlas),
            Transform::from_xyz(0.0, 0.0, SPARKLE_Z),
            Visibility::Hidden,
        ));
    }
}

/// Spread over the period by tile position, so a farm doesn't blink in sync.
fn sparkle_offset(x: u32, y: u32) -> f32 {
    let steps = (SPARKLE_PERIOD_SECS / SPARKLE_FRAME_SECS) as u32;
    (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13)) % steps) as f32 * SPARKLE_FRAME_SECS
}

/// The twinkle's frame at `t` (seconds incl. offset), `None` between twinkles.
fn sparkle_frame(t: f32) -> Option<usize> {
    let frame = (t.rem_euclid(SPARKLE_PERIOD_SECS) / SPARKLE_FRAME_SECS) as usize;
    (frame < SPARKLE_FRAMES).then_some(frame)
}

fn twinkle(time: Res<Time>, mut sparkles: Query<(&Sparkle, &mut Sprite, &mut Visibility)>) {
    let now = time.elapsed_secs();
    for (sparkle, mut sprite, mut visibility) in &mut sparkles {
        match sparkle_frame(now + sparkle.offset) {
            Some(frame) => {
                visibility.set_if_neq(Visibility::Inherited);
                if let Some(atlas) = &mut sprite.texture_atlas {
                    atlas.index = frame;
                }
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

fn card_text(tree: &Tree, now: f64) -> CardText {
    let mut title = format!("{} tree", name(tree.item));
    if tree.spliced {
        title.push_str(&format!("{SEP}spliced"));
    }
    if is_grown(tree, now) {
        return CardText {
            title,
            fill: 1.0,
            percent: None,
            status: format!("Ready!{SEP}{} {}", tree.fruit, name(tree.item)),
        };
    }
    let needed = item(tree.item)
        .map_or(1, |def| grow_seconds(def.rarity))
        .max(1);
    let elapsed = ((now - tree.planted_at).max(0.0) as u64).min(needed);
    let fill = elapsed as f32 / needed as f32;
    CardText {
        title,
        fill,
        percent: Some(((fill * 100.0) as u32).min(99)),
        status: format!("{} left", duration_text(needed - elapsed)),
    }
}

/// Top-left of a card `size` big over the tree whose top and bottom centres are at `top` and
/// `bottom` (screen px): above it, or below when there is no room, kept on screen.
fn card_position(top: Vec2, bottom: Vec2, size: Vec2, window: Vec2) -> Vec2 {
    let x = (top.x - size.x / 2.0).clamp(0.0, (window.x - size.x).max(0.0));
    let above = top.y - CARD_GAP - size.y;
    let y = if above >= 0.0 {
        above
    } else {
        (bottom.y + CARD_GAP).min((window.y - size.y).max(0.0))
    };
    Vec2::new(x, y)
}

fn spawn_card_lines(card: &mut ChildSpawnerCommands, text: &CardText) {
    card.spawn((ink(text.title.clone()), tip_part()));
    let row = Node {
        column_gap: px(HUD_MARGIN),
        align_items: AlignItems::Center,
        ..default()
    };
    card.spawn((row, tip_part())).with_children(|row| {
        let groove = Node {
            width: px(BAR_SIZE.x),
            height: px(BAR_SIZE.y),
            ..default()
        };
        let fill = Node {
            width: px(BAR_SIZE.x * text.fill),
            height: percent(100),
            ..default()
        };
        row.spawn((groove, BackgroundColor(BAR_GROOVE), tip_part()))
            .with_child((fill, BackgroundColor(BAR_FILL), tip_part()));
        if let Some(pct) = text.percent {
            row.spawn((ink(format!("{pct} %")), tip_part()));
        }
    });
    card.spawn((ink(text.status.clone()), tip_part()));
}

/// The card: pinned by a long-press, else after `TOOLTIP_DELAY_SECS` of hovering, else for the
/// tree in front of a player standing still. Hidden while a panel is open.
#[allow(clippy::too_many_arguments)] // a Bevy system: three inputs, the world and the box
fn show_card(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    open: Res<OpenPanel>,
    hovered: Res<HoveredTile>,
    touches: Res<Touches>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    player: Single<&Player>,
    card: Single<(Entity, &mut Node, &ComputedNode), With<TreeCard>>,
    mut input: Local<CardInput>,
) {
    let now = time.elapsed_secs();
    let world = &game.0.world;
    let (camera, camera_transform) = *camera;
    let tile_at = |screen: Vec2| {
        let point = camera.viewport_to_world_2d(camera_transform, screen).ok()?;
        world_to_tile(point.x, point.y)
    };

    // Touch: long-press pins, a tap elsewhere unpins.
    for touch in touches.iter_just_pressed() {
        input.touch_started.insert(touch.id(), now);
        if input.pinned.is_some() && tile_at(touch.position()) != input.pinned {
            input.pinned = None;
        }
    }
    for touch in touches.iter() {
        let held = input
            .touch_started
            .get(&touch.id())
            .is_some_and(|&from| now - from >= LONG_PRESS_SECS);
        if held && touch.distance().length() <= TOUCH_SLOP {
            let tile = tile_at(touch.position()).filter(|&(x, y)| world.tree(x, y).is_some());
            if tile.is_some() {
                input.pinned = tile;
            }
        }
    }
    for touch in touches
        .iter_just_released()
        .chain(touches.iter_just_canceled())
    {
        input.touch_started.remove(&touch.id());
    }

    // Mouse.
    if input.hover.map(|(tile, _)| tile) != hovered.0 {
        input.hover = hovered.0.map(|tile| (tile, now));
    }
    let hover = input
        .hover
        .filter(|&(_, from)| now - from >= TOOLTIP_DELAY_SECS)
        .map(|(tile, _)| tile);

    // Keyboard: standing still in front of a tree in reach.
    let pos = player.position;
    match input.still {
        Some((at, _)) if at.distance(pos) <= STILL_EPSILON => {}
        _ => input.still = Some((pos, now)),
    }
    let in_front = input
        .still
        .filter(|&(_, from)| now - from >= STILL_SECS)
        .and_then(|_| {
            let side = if player.facing_west { -1.0 } else { 1.0 };
            let feet = pos.y - HITBOX_HEIGHT / 2.0 + TILE_SIZE / 2.0;
            let (x, y) = world_to_tile(pos.x + side * TILE_SIZE, feet)?;
            let me = game.me()?;
            in_reach_with(&me.body, x, y, worn_mods(&me.equipped)).then_some((x, y))
        });

    let target = [input.pinned, hover, in_front]
        .into_iter()
        .flatten()
        .find_map(|(x, y)| world.tree(x, y).map(|tree| ((x, y), *tree)))
        .filter(|_| *open == OpenPanel::None);
    if target.is_none()
        && input
            .pinned
            .is_some_and(|(x, y)| world.tree(x, y).is_none())
    {
        input.pinned = None;
    }

    let (card_entity, mut node, computed) = card.into_inner();
    let Some(((x, y), tree)) = target else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        input.shown = None;
        return;
    };
    let text = card_text(&tree, time.elapsed_secs_f64());
    if input.shown.as_ref() != Some(&text) {
        commands
            .entity(card_entity)
            .despawn_children()
            .with_children(|card| spawn_card_lines(card, &text));
        input.shown = Some(text);
    }
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    let (cx, cy) = tile_to_world(x, y);
    let screen = |dy: f32| {
        camera
            .world_to_viewport(camera_transform, Vec3::new(cx, cy + dy, 0.0))
            .ok()
    };
    let (Some(top), Some(bottom)) = (screen(TILE_SIZE / 2.0), screen(-TILE_SIZE / 2.0)) else {
        return;
    };
    // The box's own size is known a frame after it is first shown; it settles then.
    let size = computed.size * computed.inverse_scale_factor();
    let at = card_position(top, bottom, size, window.size());
    if node.left != px(at.x) || node.top != px(at.y) {
        node.left = px(at.x);
        node.top = px(at.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{DIRT, GRAVEL, ItemKind, PLANKS, STONE, seed_of};

    fn tree(item: ItemId, spliced: bool) -> Tree {
        Tree {
            item,
            planted_at: 100.0,
            fruit: 3,
            spliced,
        }
    }

    fn needed(id: ItemId) -> u64 {
        grow_seconds(item(id).unwrap().rarity)
    }

    #[test]
    fn plant_and_splice_toasts_carry_the_grow_time() {
        let planted = Event::TreePlanted {
            x: 1,
            y: 1,
            tree: tree(PLANKS, false),
        };
        assert_eq!(
            farm_toast(&planted),
            Some(format!(
                "Planted Planks seed · ready in {}",
                duration_text(needed(PLANKS))
            ))
        );
        let spliced = Event::TreeSpliced {
            x: 1,
            y: 1,
            tree: tree(GRAVEL, true),
            a: DIRT,
            b: STONE,
        };
        assert_eq!(
            farm_toast(&spliced),
            Some(format!(
                "Dirt + Stone spliced into Gravel tree · ready in {}",
                duration_text(needed(GRAVEL))
            ))
        );
        assert_eq!(farm_toast(&Event::TreeHarvested { x: 1, y: 1 }), None);
    }

    #[test]
    fn the_seed_tooltip_and_the_plant_toast_say_the_same_time() {
        let seed = item(seed_of(PLANKS).unwrap()).unwrap();
        assert_eq!(seed.kind, ItemKind::Seed);
        let time = duration_text(grow_seconds(seed.rarity));
        let tip = crate::hud::kind_line(seed, false).unwrap();
        assert!(tip.ends_with(&format!(" · {time}")), "{tip}");
        let toast = farm_toast(&Event::TreePlanted {
            x: 0,
            y: 0,
            tree: tree(PLANKS, false),
        })
        .unwrap();
        assert!(toast.ends_with(&format!("ready in {time}")), "{toast}");
    }

    #[test]
    fn the_card_counts_down_then_says_ready() {
        let t = tree(PLANKS, false);
        let total = needed(PLANKS);
        let start = card_text(&t, 100.0);
        assert_eq!(start.title, "Planks tree");
        assert_eq!((start.fill, start.percent), (0.0, Some(0)));
        assert_eq!(start.status, format!("{} left", duration_text(total)));
        // Half way, mid-second: the fill moves in whole seconds.
        let half = 100.0 + (total / 2) as f64 + 0.4;
        let mid = card_text(&t, half);
        assert_eq!(mid.percent, Some(50));
        assert_eq!(mid.fill, card_text(&t, half - 0.3).fill);
        assert_eq!(
            mid.status,
            format!("{} left", duration_text(total - total / 2))
        );
        let grown = card_text(&t, 100.0 + total as f64);
        assert_eq!((grown.fill, grown.percent), (1.0, None));
        assert_eq!(grown.status, "Ready! · 3 Planks");
        assert_eq!(
            card_text(&tree(PLANKS, true), 0.0).title,
            "Planks tree · spliced"
        );
    }

    #[test]
    fn the_card_sits_above_the_tree_or_below_it_on_screen() {
        let window = Vec2::new(800.0, 600.0);
        let size = Vec2::new(140.0, 60.0);
        let at = card_position(
            Vec2::new(400.0, 300.0),
            Vec2::new(400.0, 332.0),
            size,
            window,
        );
        assert_eq!(at, Vec2::new(330.0, 300.0 - CARD_GAP - 60.0));
        // No room above: below the tree, never over it.
        let at = card_position(Vec2::new(10.0, 20.0), Vec2::new(10.0, 52.0), size, window);
        assert_eq!(at, Vec2::new(0.0, 52.0 + CARD_GAP));
    }

    #[test]
    fn twinkles_play_once_per_period_and_are_spread_out() {
        assert_eq!(sparkle_frame(0.0), Some(0));
        assert_eq!(sparkle_frame(0.35), Some(3));
        assert_eq!(sparkle_frame(0.45), None);
        assert_eq!(sparkle_frame(SPARKLE_PERIOD_SECS + 0.15), Some(1));
        assert_ne!(sparkle_offset(3, 10), sparkle_offset(4, 10));
        assert!(sparkle_offset(999, 49) < SPARKLE_PERIOD_SECS);
    }
}
