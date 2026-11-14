//! Mouse and keys -> `Action`. Shows which tile is hovered and whether it is in range, flashes a
//! tile when the rules reject an action on it, and outlines a claim inspected with the wrench.
//! Decides nothing itself.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use game_shared::items::{FIST, ItemId, WRENCH, item, slot_of};
use game_shared::rules::{Action, Event, PUNCH_COOLDOWN, Reject, in_reach};
use game_shared::world::World;
use game_shared::{TILE_SIZE, tile_to_world, world_to_tile};

use crate::hud::{Notice, OpenPanel, Selected, WardrobeSelected, discard_target, pointer_over_ui};
use crate::player::Player;
use crate::state::{ActionMsg, ActionResult, Game, GameEvent, Phase};
use crate::world_view::{Temporary, Z_HIGHLIGHT};

const DROP_KEY: KeyCode = KeyCode::KeyQ;
const TRASH_KEY: KeyCode = KeyCode::KeyX;
/// Items per key press.
const DISCARD_COUNT: u16 = 1;
/// Seconds within which a second press of the trash key confirms.
const TRASH_CONFIRM_SECS: f64 = 2.0;

const IN_RANGE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.25);
const OUT_OF_RANGE_COLOR: Color = Color::srgba(1.0, 0.2, 0.2, 0.25);
const REJECT_COLOR: Color = Color::srgba(1.0, 0.0, 0.0, 0.5);
const CLAIM_OUTLINE_COLOR: Color = Color::srgba(0.3, 0.8, 1.0, 0.3);
/// Shown on a tile the player may not edit.
const LOCK_ICON: &str = "blocks/48_padlock.png";
/// Seconds a rejected tile stays marked.
const REJECT_FLASH_SECS: f32 = 0.25;
const LOCK_FLASH_SECS: f32 = 1.0;

/// The tile under the cursor, if the cursor is over the world and not over the HUD.
#[derive(Resource, Default)]
struct HoveredTile(Option<(u32, u32)>);

/// The claim block the player is looking at with the wrench. View state only.
#[derive(Resource, Default)]
pub struct InspectedClaim(pub Option<(u32, u32)>);

#[derive(Component)]
struct Highlight;

#[derive(Component)]
struct ClaimOutline;

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HoveredTile>()
            .init_resource::<InspectedClaim>()
            .add_systems(Startup, spawn_highlight)
            .add_systems(
                Update,
                (hover, (click, inspect, discard))
                    .chain()
                    .in_set(Phase::Input),
            )
            .add_systems(Update, (show_results, outline_claim).in_set(Phase::View));
    }
}

fn tile_square(color: Color) -> Sprite {
    Sprite::from_color(color, Vec2::splat(TILE_SIZE))
}

fn spawn_highlight(mut commands: Commands) {
    commands.spawn((Highlight, tile_square(IN_RANGE_COLOR), Visibility::Hidden));
}

fn hover(
    game: Res<Game>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    ui: Query<&Hovered>,
    mut hovered: ResMut<HoveredTile>,
    highlight: Single<(&mut Sprite, &mut Transform, &mut Visibility), With<Highlight>>,
) {
    let (camera, camera_transform) = *camera;
    // On the HUD the pointer belongs to the HUD, not to the world tile behind it.
    hovered.0 = window
        .cursor_position()
        .filter(|_| !pointer_over_ui(&ui))
        .and_then(|cursor| camera.viewport_to_world_2d(camera_transform, cursor).ok())
        .and_then(|point| world_to_tile(point.x, point.y));

    let (mut sprite, mut transform, mut visibility) = highlight.into_inner();
    let Some((x, y)) = hovered.0 else {
        *visibility = Visibility::Hidden;
        return;
    };
    let reachable = game.me().is_some_and(|me| in_reach(&me.body, x, y));
    let (cx, cy) = tile_to_world(x, y);
    *visibility = Visibility::Visible;
    transform.translation = Vec3::new(cx, cy, Z_HIGHLIGHT);
    sprite.color = if reachable {
        IN_RANGE_COLOR
    } else {
        OUT_OF_RANGE_COLOR
    };
}

/// Left mouse held: punch with the fist or with equipment selected (equipment is never placed; the
/// worn pickaxe sets the punch power), otherwise place the selected item. Sent at the punch
/// cadence; the rules still decide whether each one counts. The wrench sends nothing (see `inspect`).
fn click(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    mut next_at: Local<f64>,
    mut actions: MessageWriter<ActionMsg>,
) {
    let now = time.elapsed_secs_f64();
    let (true, Some((x, y)), Some(item)) = (
        mouse.pressed(MouseButton::Left) && now >= *next_at,
        hovered.0,
        selected.item(&game).filter(|&item| item != WRENCH),
    ) else {
        return;
    };
    *next_at = now + PUNCH_COOLDOWN;
    actions.write(ActionMsg(if item == FIST || slot_of(item).is_some() {
        Action::Punch { x, y }
    } else {
        Action::Place { x, y, item }
    }));
}

/// Wrench + click: look at the clicked claim block if it is the player's own, else stop looking.
fn inspect(
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    mut inspected: ResMut<InspectedClaim>,
) {
    let holding_wrench = selected.item(&game) == Some(WRENCH);
    if !holding_wrench && inspected.0.is_some() {
        inspected.0 = None;
    }
    let (true, true, Some((x, y))) = (
        holding_wrench,
        mouse.just_pressed(MouseButton::Left),
        hovered.0,
    ) else {
        return;
    };
    let claim = World::tile_key(x, y).and_then(|key| game.0.world.claim(key));
    let own = claim.is_some_and(|claim| claim.owner == game.1);
    let target = own.then_some((x, y));
    if inspected.0 != target {
        inspected.0 = target;
    }
}

/// Redraws the outline of the inspected claim's tiles when the inspection or the claim changes.
fn outline_claim(
    mut commands: Commands,
    game: Res<Game>,
    mut inspected: ResMut<InspectedClaim>,
    mut events: MessageReader<GameEvent>,
    outline: Query<Entity, With<ClaimOutline>>,
) {
    let removed = events
        .read()
        .any(|GameEvent(event)| matches!(event, Event::ClaimRemoved { .. }));
    if !removed && !inspected.is_changed() {
        return;
    }
    for entity in &outline {
        commands.entity(entity).despawn();
    }
    let claim = inspected
        .0
        .and_then(|(x, y)| game.0.world.claim(World::tile_key(x, y)?));
    let Some(claim) = claim else {
        if inspected.0.is_some() {
            inspected.0 = None; // the claim is gone
        }
        return;
    };
    // A world claim has no tile list: there is nothing smaller than the world to outline.
    for (x, y) in claim.tiles.iter().filter_map(|&key| World::tile_xy(key)) {
        let (cx, cy) = tile_to_world(x, y);
        commands.spawn((
            ClaimOutline,
            tile_square(CLAIM_OUTLINE_COLOR),
            Transform::from_xyz(cx, cy, Z_HIGHLIGHT),
        ));
    }
}

/// Q drops one of the selected item on the side the player faces. X trashes one, after a second
/// press within `TRASH_CONFIRM_SECS`. While the wardrobe is open they act on the item selected in
/// its grid (`hud::discard_target`).
#[allow(clippy::too_many_arguments)] // input, two selections, facing, confirmation state and two outputs
fn discard(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    game: Res<Game>,
    selected: Res<Selected>,
    wardrobe: Res<WardrobeSelected>,
    open: Res<OpenPanel>,
    player: Single<&Player>,
    mut armed: Local<Option<(ItemId, f64)>>,
    mut actions: MessageWriter<ActionMsg>,
    mut notices: MessageWriter<Notice>,
) {
    let Some(selected_item) = discard_target(&game, *open, &selected, &wardrobe) else {
        return;
    };
    let count = DISCARD_COUNT;
    if keys.just_pressed(DROP_KEY) {
        let left = player.facing_west;
        actions.write(ActionMsg(Action::Drop {
            item: selected_item,
            count,
            left,
        }));
    }
    if keys.just_pressed(TRASH_KEY) {
        let now = time.elapsed_secs_f64();
        let confirmed = armed.is_some_and(|(item, until)| item == selected_item && now <= until);
        if confirmed {
            *armed = None;
            actions.write(ActionMsg(Action::Trash {
                item: selected_item,
                count,
            }));
        } else {
            *armed = Some((selected_item, now + TRASH_CONFIRM_SECS));
            let name = item(selected_item).map_or("item", |def| def.name);
            notices.write(Notice(format!("Press X again to destroy {count} {name}")));
        }
    }
}

fn show_results(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    mut results: MessageReader<ActionResult>,
) {
    for ActionResult { action, result } in results.read() {
        match (action, result) {
            // Expected while the button is held.
            (_, Err(Reject::Cooldown)) => {}
            (&Action::Punch { x, y } | &Action::Place { x, y, .. }, Err(reject)) => {
                let (cx, cy) = tile_to_world(x, y);
                let locked = matches!(reject, Reject::NoPermission | Reject::NotOwner);
                let (sprite, seconds) = if locked {
                    (Sprite::from_image(assets.load(LOCK_ICON)), LOCK_FLASH_SECS)
                } else {
                    (tile_square(REJECT_COLOR), REJECT_FLASH_SECS)
                };
                commands.spawn((
                    sprite,
                    Transform::from_xyz(cx, cy, Z_HIGHLIGHT),
                    Temporary {
                        until: time.elapsed_secs() + seconds,
                    },
                ));
            }
            // The HUD's message line explains the ones a player cannot guess.
            (action, Err(reject)) => info!("{action:?} rejected: {reject:?}"),
            (_, Ok(events)) => {
                let left = |event: &Event| matches!(event, Event::LeftWorld { .. });
                if events.iter().any(left) {
                    // ponytail: there is no other world to go to yet; world switching comes with M7.
                    info!("punched the main door: leaving the world");
                }
            }
        }
    }
}
