//! Mouse and keys -> `Action`. Shows which tile is hovered and whether it is in range, flashes a
//! tile when the rules reject an action on it (a blinking lock on a protected one), and opens the
//! claim panel for a claim clicked with the wrench (`claim_ui.rs`). Decides nothing itself.

use bevy::picking::hover::Hovered;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use game_shared::inventory::MAX_STACK;
use game_shared::items::{
    FIST, ItemId, MAIN_DOOR, PERMANENT, UNTRADEABLE, WORKBENCH, WRENCH, item, slot_of,
};
use game_shared::mods::worn_mods;
use game_shared::rules::{Action, Event, PUNCH_COOLDOWN, Reject, in_reach_with};
use game_shared::world::World;
use game_shared::{TILE_SIZE, tile_to_world, world_to_tile};

use crate::farming_ui::LONG_PRESS_SECS;
use crate::hud::{
    ItemDrag, Notice, OpenPanel, Selected, WardrobeSelected, discard_target, key_name,
    pointer_over_ui, silent_reject, title_case,
};
use crate::player::Player;
use crate::state::{ActionMsg, ActionResult, Game, Phase};
use crate::world_view::{Temporary, Z_HIGHLIGHT};

pub(crate) const DROP_KEY: KeyCode = KeyCode::KeyQ;
const TRASH_KEY: KeyCode = KeyCode::KeyX;
/// Held with Q or X: the whole stack (docs/knowledge/ux/inventory-categories.md, "Dropping").
const STACK_KEYS: [KeyCode; 2] = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
/// Gamepad: a tap drops one, held `PAD_STACK_SECS` drops the stack.
const PAD_DROP: GamepadButton = GamepadButton::West;
const PAD_STACK_SECS: f64 = 0.5;
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

/// A Drop or Trash press from the selection strip's buttons (`inventory_filter.rs`); `stack` =
/// Shift+click. Goes through the same path as Q / X.
#[derive(Message, Clone, Copy)]
pub struct DiscardPress {
    pub trash: bool,
    pub stack: bool,
}

/// Whether the rules let the item leave the inventory by Drop / Trash (not the fist or wrench).
pub(crate) fn discardable(id: ItemId) -> bool {
    item(id).is_some_and(|def| !def.has(UNTRADEABLE | PERMANENT))
}

/// Items a stack drop or trash sends: all held, within what one action may carry.
pub(crate) fn stack_count(held: u16) -> u16 {
    held.clamp(1, MAX_STACK)
}

/// A drag released here drops the stack: off every cell and off the HUD (over the world).
/// Released on a cell it moves (`hud::drop_action`); on a panel or button it cancels.
pub(crate) fn drags_out(over_cell: bool, over_ui: bool) -> bool {
    !over_cell && !over_ui
}

/// What one trash press does: arm (first press, the notice says how many) or confirm.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TrashPress {
    Arm(u16),
    Confirm(u16),
}

/// The trash confirm: a second press for the same item, the same mode (one or stack), within
/// `TRASH_CONFIRM_SECS`, confirms. `armed` is (item, stack, until).
fn trash_press(
    armed: &mut Option<(ItemId, bool, f64)>,
    target: ItemId,
    stack: bool,
    held: u16,
    now: f64,
) -> TrashPress {
    let count = if stack {
        stack_count(held)
    } else {
        DISCARD_COUNT
    };
    let confirmed =
        armed.is_some_and(|(item, mode, until)| item == target && mode == stack && now <= until);
    if confirmed {
        *armed = None;
        TrashPress::Confirm(count)
    } else {
        *armed = Some((target, stack, now + TRASH_CONFIRM_SECS));
        TrashPress::Arm(count)
    }
}

/// The toast after a stack drop or trash went through (`Dropped 37 Dirt`); single items say nothing.
fn stack_toast(action: &Action) -> Option<String> {
    let (verb, id, count) = match *action {
        Action::Drop { item, count, .. } => ("Dropped", item, count),
        Action::Trash { item, count } => ("Trashed", item, count),
        _ => return None,
    };
    let name = title_case(item(id)?.name);
    (count > DISCARD_COUNT).then(|| format!("{verb} {count} {name}"))
}

/// Blinks until its `Temporary` ends.
#[derive(Component)]
struct Blink;

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DiscardPress>()
            .init_resource::<HoveredTile>()
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
/// A touch does the same, except on a tree or sapling: there only a short tap acts, on release,
/// so a long-press (the tree card, `farming_ui.rs`) never damages or harvests it.
#[allow(clippy::too_many_arguments)] // a Bevy system: two pointers, the world and the selection
fn click(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    camera: Single<(&Camera, &GlobalTransform)>,
    ui: Query<&Hovered>,
    game: Res<Game>,
    selected: Res<Selected>,
    hovered: Res<HoveredTile>,
    mut next_at: Local<f64>,
    mut touch_started: Local<HashMap<u64, f32>>,
    mut open: ResMut<OpenPanel>,
    mut actions: MessageWriter<ActionMsg>,
) {
    let now = time.elapsed_secs_f64();
    let Some(item) = selected.item(&game).filter(|&item| item != WRENCH) else {
        return;
    };
    let act = |x, y| {
        ActionMsg(if item == FIST || slot_of(item).is_some() {
            Action::Punch { x, y }
        } else {
            Action::Place { x, y, item }
        })
    };
    let cadence_ready = now >= *next_at;
    // The main door opens the World menu (docs/knowledge/ux/world-select.md); it is never punched.
    let on_door = |x: u32, y: u32| game.0.world.get(x, y).is_some_and(|t| t.fg == MAIN_DOOR);

    let touching = touches.iter().next().is_some() || touches.any_just_released();
    if touching {
        let (camera, camera_transform) = *camera;
        let over_ui = pointer_over_ui(&ui);
        let tile_at = |screen: Vec2| {
            let point = camera.viewport_to_world_2d(camera_transform, screen).ok()?;
            world_to_tile(point.x, point.y)
        };
        let secs = time.elapsed_secs();
        for touch in touches.iter_just_pressed() {
            touch_started.insert(touch.id(), secs);
        }
        let mut fired = false;
        let pressed = touches.iter().map(|touch| (touch, false));
        let released = touches.iter_just_released().map(|touch| (touch, true));
        for (touch, released) in pressed.chain(released) {
            let Some((x, y)) = tile_at(touch.position()).filter(|_| !over_ui) else {
                continue;
            };
            let held = touch_started
                .get(&touch.id())
                .map_or(0.0, |&from| secs - from);
            if on_door(x, y) {
                if released {
                    open.set_if_neq(OpenPanel::WorldSelect);
                }
                continue;
            }
            let on_tree = game.0.world.tree(x, y).is_some();
            if !fired && touch_fires(on_tree, held, released, cadence_ready) {
                actions.write(act(x, y));
                fired = true;
            }
        }
        for touch in touches
            .iter_just_released()
            .chain(touches.iter_just_canceled())
        {
            touch_started.remove(&touch.id());
        }
        if fired {
            *next_at = now + PUNCH_COOLDOWN;
        }
        return;
    }

    if let Some((x, y)) = hovered.0
        && on_door(x, y)
    {
        if mouse.just_pressed(MouseButton::Left) {
            open.set_if_neq(OpenPanel::WorldSelect);
        }
        return;
    }
    let (true, Some((x, y))) = (mouse.pressed(MouseButton::Left) && cadence_ready, hovered.0)
    else {
        return;
    };
    *next_at = now + PUNCH_COOLDOWN;
    actions.write(act(x, y));
}

/// Whether a touch acts this frame. On a tree or sapling: only when a short tap (under
/// `LONG_PRESS_SECS`) is released. Elsewhere: while pressed, at the punch cadence, like the mouse.
fn touch_fires(on_tree: bool, held: f32, released: bool, cadence_ready: bool) -> bool {
    if on_tree {
        released && held < LONG_PRESS_SECS
    } else {
        !released && cadence_ready
    }
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

/// Q drops one of the selected item on the side the player faces, Shift+Q the whole stack. X
/// trashes one after a second press within `TRASH_CONFIRM_SECS`, Shift+X the whole stack the same
/// way. The strip's Drop / Trash buttons (`DiscardPress`) and the gamepad's X (tap one, hold the
/// stack) do the same. While the wardrobe is open they act on the item selected in its grid
/// (`hud::discard_target`). The fist and the wrench never leave.
#[allow(clippy::too_many_arguments)] // input, two selections, facing, confirmation state and two outputs
fn discard(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    game: Res<Game>,
    selected: Res<Selected>,
    wardrobe: Res<WardrobeSelected>,
    open: Res<OpenPanel>,
    player: Single<&Player>,
    mut presses: MessageReader<DiscardPress>,
    mut armed: Local<Option<(ItemId, bool, f64)>>,
    mut pad_since: Local<Option<f64>>,
    mut actions: MessageWriter<ActionMsg>,
    mut notices: MessageWriter<Notice>,
) {
    let now = time.elapsed_secs_f64();
    let presses: Vec<DiscardPress> = presses.read().copied().collect();
    let Some(selected_item) =
        discard_target(&game, *open, &selected, &wardrobe).filter(|&id| discardable(id))
    else {
        *pad_since = None;
        return;
    };
    let held = game.me().map_or(0, |me| me.inventory.count(selected_item));
    let shift = keys.any_pressed(STACK_KEYS);
    let mut drops: Vec<bool> = Vec::new();
    let mut trashes: Vec<(bool, bool)> = Vec::new();
    if keys.just_pressed(DROP_KEY) {
        drops.push(shift);
    }
    if keys.just_pressed(TRASH_KEY) {
        trashes.push((shift, false));
    }
    for press in presses {
        if press.trash {
            trashes.push((press.stack, true));
        } else {
            drops.push(press.stack);
        }
    }
    let pad_down = pads.iter().any(|pad| pad.pressed(PAD_DROP));
    match (*pad_since, pad_down) {
        (None, true) => *pad_since = Some(now),
        (Some(from), true) if from >= 0.0 && now - from >= PAD_STACK_SECS => {
            drops.push(true);
            *pad_since = Some(f64::NEG_INFINITY); // fired; wait for the release
        }
        (Some(from), false) => {
            if from >= 0.0 {
                drops.push(false);
            }
            *pad_since = None;
        }
        _ => {}
    }
    for stack in drops {
        let count = if stack {
            stack_count(held)
        } else {
            DISCARD_COUNT
        };
        actions.write(ActionMsg(Action::Drop {
            item: selected_item,
            count,
            left: player.facing_west,
        }));
    }
    for (stack, clicked) in trashes {
        match trash_press(&mut armed, selected_item, stack, held, now) {
            TrashPress::Confirm(count) => {
                actions.write(ActionMsg(Action::Trash {
                    item: selected_item,
                    count,
                }));
            }
            TrashPress::Arm(count) => {
                let name = title_case(item(selected_item).map_or("item", |def| def.name));
                let again = match (clicked, stack) {
                    (true, _) => "Click Trash".to_string(),
                    (false, true) => format!("Press Shift+{}", key_name(TRASH_KEY)),
                    (false, false) => format!("Press {}", key_name(TRASH_KEY)),
                };
                notices.write(Notice(format!("{again} again to destroy {count} {name}")));
            }
        }
    }
}

fn show_results(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    mut results: MessageReader<ActionResult>,
    mut notices: MessageWriter<Notice>,
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
            (action, Ok(events)) => {
                notices.write_batch(stack_toast(action).map(Notice));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_punched_on_a_short_taps_release_only() {
        // Tree or sapling: nothing while held, a short tap acts on release, a long-press never.
        assert!(!touch_fires(true, 0.1, false, true));
        assert!(touch_fires(true, 0.1, true, true));
        assert!(touch_fires(true, LONG_PRESS_SECS - 0.01, true, false));
        assert!(!touch_fires(true, LONG_PRESS_SECS, true, true));
        assert!(!touch_fires(true, 2.0, false, true));
        // Any other tile: on press and while held at the cadence; the release does nothing.
        assert!(touch_fires(false, 0.0, false, true));
        assert!(touch_fires(false, 2.0, false, true));
        assert!(!touch_fires(false, 0.5, false, false));
        assert!(!touch_fires(false, 0.1, true, true));
    }

    #[test]
    fn shift_x_twice_trashes_the_whole_stack_capped() {
        let dirt = ItemId(2);
        let mut armed = None;
        // Shift+X twice within the window: the held count, capped at one action's limit.
        assert_eq!(
            trash_press(&mut armed, dirt, true, 37, 0.0),
            TrashPress::Arm(37)
        );
        assert_eq!(
            trash_press(&mut armed, dirt, true, 37, 1.9),
            TrashPress::Confirm(37)
        );
        assert_eq!(armed, None);
        assert_eq!(
            trash_press(&mut armed, dirt, true, 900, 0.0),
            TrashPress::Arm(MAX_STACK)
        );
        assert_eq!(
            trash_press(&mut armed, dirt, true, 900, 0.5),
            TrashPress::Confirm(MAX_STACK)
        );
        // Too late: the second press arms again.
        trash_press(&mut armed, dirt, true, 5, 0.0);
        assert_eq!(
            trash_press(&mut armed, dirt, true, 5, TRASH_CONFIRM_SECS + 0.1),
            TrashPress::Arm(5)
        );
        // X then Shift+X (or another item) is not a confirm; plain X twice trashes one.
        let mut armed = None;
        trash_press(&mut armed, dirt, false, 5, 0.0);
        assert_eq!(
            trash_press(&mut armed, dirt, true, 5, 0.1),
            TrashPress::Arm(5)
        );
        assert_eq!(
            trash_press(&mut armed, ItemId(3), true, 5, 0.2),
            TrashPress::Arm(5)
        );
        trash_press(&mut armed, dirt, false, 5, 1.0);
        assert_eq!(
            trash_press(&mut armed, dirt, false, 5, 1.5),
            TrashPress::Confirm(1)
        );
    }

    #[test]
    fn shift_q_drops_the_stack_capped_and_only_stacks_toast() {
        assert_eq!(stack_count(37), 37);
        assert_eq!(stack_count(MAX_STACK + 50), MAX_STACK);
        assert_eq!(stack_count(0), 1);
        let dirt = game_shared::items::DIRT;
        let name = title_case(item(dirt).unwrap().name);
        let drop = |count| Action::Drop {
            item: dirt,
            count,
            left: false,
        };
        assert_eq!(stack_toast(&drop(37)), Some(format!("Dropped 37 {name}")));
        assert_eq!(stack_toast(&drop(1)), None);
        let trash = Action::Trash {
            item: dirt,
            count: 37,
        };
        assert_eq!(stack_toast(&trash), Some(format!("Trashed 37 {name}")));
        assert!(!discardable(FIST) && !discardable(WRENCH) && discardable(dirt));
    }

    #[test]
    fn a_drag_drops_only_when_released_over_the_world() {
        assert!(drags_out(false, false));
        assert!(!drags_out(true, true), "on a cell: a move");
        assert!(
            !drags_out(false, true),
            "on the panel or a button: cancelled"
        );
    }
}
