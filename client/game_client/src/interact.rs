//! Mouse and keys -> `Action`. Shows which tile is hovered and whether it is in range, flashes a
//! tile when the rules reject an action on it (a blinking lock on a protected one), and opens the
//! claim panel for a claim clicked with the wrench (`claim_ui.rs`). Decides nothing itself.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use game_shared::items::{FIST, ItemId, WORKBENCH, WRENCH, item, slot_of};
use game_shared::mods::worn_mods;
use game_shared::rules::{Action, Event, PUNCH_COOLDOWN, Reject, in_reach_with};
use game_shared::world::World;
use game_shared::{TILE_SIZE, tile_to_world, world_to_tile};

use crate::hud::{
    ItemDrag, Notice, OpenPanel, Selected, WardrobeSelected, discard_target, pointer_over_ui,
    silent_reject,
};
use crate::player::Player;
use crate::state::{ActionMsg, ActionResult, Game, Phase};
use crate::world_view::{Temporary, Z_HIGHLIGHT};

const DROP_KEY: KeyCode = KeyCode::KeyQ;
const TRASH_KEY: KeyCode = KeyCode::KeyX;
/// Items per key press.
const DISCARD_COUNT: u16 = 1;
/// Seconds within which a second press of the trash key confirms.
const TRASH_CONFIRM_SECS: f64 = 2.0;

const IN_RANGE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.25);
const OUT_OF_RANGE_COLOR: Color = Color::srgba(1.0, 0.2, 0.2, 0.25);
/// The hover square, red at 40 %, on a refused tile (docs/knowledge/ux/refusals-and-big-buys.md).
const REJECT_COLOR: Color = Color::srgba(1.0, 0.0, 0.0, 0.4);
/// Shown on a tile the player may not edit: the claim sprite at half size.
const LOCK_ICON: &str = "blocks/48_padlock.png";
const LOCK_SIZE: f32 = TILE_SIZE / 2.0;
/// Seconds a rejected tile stays marked.
const REJECT_FLASH_SECS: f32 = 0.3;
const LOCK_FLASH_SECS: f32 = 0.6;
/// The lock is shown, then hidden, this long each.
const LOCK_BLINK_SECS: f32 = 0.1;

/// The tile under the cursor, if the cursor is over the world and not over the HUD.
#[derive(Resource, Default)]
pub(crate) struct HoveredTile(pub Option<(u32, u32)>);

/// The claim block the player is looking at with the wrench. View state only.
#[derive(Resource, Default)]
pub struct InspectedClaim(pub Option<(u32, u32)>);

#[derive(Component)]
struct Highlight;

/// Blinks until its `Temporary` ends.
#[derive(Component)]
struct Blink;

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
            .add_systems(Update, (show_results, blink).in_set(Phase::View));
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
    drag: Res<ItemDrag>,
    mut hovered: ResMut<HoveredTile>,
    highlight: Single<(&mut Sprite, &mut Transform, &mut Visibility), With<Highlight>>,
) {
    let (camera, camera_transform) = *camera;
    // On the HUD, or carrying an item from it, the pointer belongs to the HUD, not to the world.
    hovered.0 = window
        .cursor_position()
        .filter(|_| !pointer_over_ui(&ui) && !drag.busy())
        .and_then(|cursor| camera.viewport_to_world_2d(camera_transform, cursor).ok())
        .and_then(|point| world_to_tile(point.x, point.y));

    let (mut sprite, mut transform, mut visibility) = highlight.into_inner();
    let Some((x, y)) = hovered.0 else {
        *visibility = Visibility::Hidden;
        return;
    };
    let reachable = game
        .me()
        .is_some_and(|me| in_reach_with(&me.body, x, y, worn_mods(&me.equipped)));
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

/// Wrench + click: open the claim panel for the clicked claim block (anyone's; a stranger gets
/// the read-only view), else stop looking. A Workbench in reach opens the craft panel instead.
/// Putting the wrench away, or another panel taking over, ends the inspection.
fn inspect(
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    mut inspected: ResMut<InspectedClaim>,
    mut open: ResMut<OpenPanel>,
) {
    let holding_wrench = selected.item(&game) == Some(WRENCH);
    if inspected.0.is_some() && (!holding_wrench || *open != OpenPanel::Claim) {
        inspected.0 = None;
        if *open == OpenPanel::Claim {
            *open = OpenPanel::None;
        }
    }
    let (true, true, Some((x, y))) = (
        holding_wrench,
        mouse.just_pressed(MouseButton::Left),
        hovered.0,
    ) else {
        return;
    };
    let workbench = game.0.world.get(x, y).is_some_and(|t| t.fg == WORKBENCH);
    let near = game
        .me()
        .is_some_and(|me| in_reach_with(&me.body, x, y, worn_mods(&me.equipped)));
    if workbench && near {
        open.set_if_neq(OpenPanel::Craft);
        return;
    }
    let claim = World::tile_key(x, y).and_then(|key| game.0.world.claim(key));
    let target = claim.map(|_| (x, y));
    if inspected.0 != target {
        inspected.0 = target;
    }
    match target {
        Some(_) => open.set_if_neq(OpenPanel::Claim),
        None if *open == OpenPanel::Claim => open.set_if_neq(OpenPanel::None),
        None => false,
    };
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
            // Silent on purpose: expected while the button is held, or already visible.
            (_, Err(reject)) if silent_reject(*reject) => {}
            (&Action::Punch { x, y } | &Action::Place { x, y, .. }, Err(reject)) => {
                let (cx, cy) = tile_to_world(x, y);
                let locked = matches!(reject, Reject::NoPermission | Reject::NotOwner);
                let at = Transform::from_xyz(cx, cy, Z_HIGHLIGHT);
                if locked {
                    let sprite = Sprite {
                        image: assets.load(LOCK_ICON),
                        custom_size: Some(Vec2::splat(LOCK_SIZE)),
                        ..default()
                    };
                    let until = time.elapsed_secs() + LOCK_FLASH_SECS;
                    commands.spawn((sprite, at, Blink, Temporary { until }));
                } else {
                    let until = time.elapsed_secs() + REJECT_FLASH_SECS;
                    commands.spawn((tile_square(REJECT_COLOR), at, Temporary { until }));
                }
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

/// Shown and hidden in turns, `LOCK_BLINK_SECS` each, counted back from the end.
fn blink(time: Res<Time>, mut locks: Query<(&Temporary, &mut Visibility), With<Blink>>) {
    for (temporary, mut visibility) in &mut locks {
        let phase = ((temporary.until - time.elapsed_secs()) / LOCK_BLINK_SECS) as u32;
        let next = if phase % 2 == 0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(next);
    }
}
