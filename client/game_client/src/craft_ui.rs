//! The craft panel (docs/knowledge/18, "Client"). Opens from a wrench click on a Workbench in
//! reach (`interact.rs`) or the inventory's Craft tab. Every recipe is listed, the ones the player
//! can craft right now first, the rest greyed. It decides nothing: the Craft button sends
//! `Action::Craft` and the rules answer; the greyed button and the red counts are only a hint.
//! The panel's entities are spawned once; a change (inventory, selection, Workbench in reach)
//! only rewrites icons, counts, tints and the button, so hover and press state survive.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_shared::crafting::{CRAFT_STATION_REACH, RECIPES, Recipe, recipe};
use game_shared::items::{ItemId, WORKBENCH, item};
use game_shared::physics::Body;
use game_shared::rules::{Action, Event};
use game_shared::world::World;
use game_shared::world_to_tile;

use crate::audio::Sfx;
use crate::hud::{
    FIT_PANEL_TOP, FitWindow, HUD_MARGIN, ICON_SIZE, INK_COLOR, ItemTip, KIT_SCALE, OpenPanel,
    PANEL_PADDING, SCROLLBAR_SPACE, SLOT_GAP, SLOT_IMAGE, SLOT_SELECTED_IMAGE, SLOT_SIZE, bag_tabs,
    count_label, grid_width, icon_node, ink, key_name, label, scroll_grid, sliced, slot_cell,
    spawn_centre_panel, spawn_mods,
};
use crate::state::{ActionMsg, Game, GameEvent, Phase};
use crate::world_view::item_icon;

const ARROW_IMAGE: &str = "ui/craft/arrow.png";
const ARROW_SIZE: f32 = 32.0;
const BUTTON_IMAGE: &str = "ui/lobby/button_primary.png";
const BUTTON_HOVER_IMAGE: &str = "ui/lobby/button_primary_hover.png";
const BUTTON_PRESSED_IMAGE: &str = "ui/lobby/button_primary_pressed.png";
const BUTTON_DISABLED_IMAGE: &str = "ui/craft/button_craft_disabled.png";
const BUTTON_INSET: f32 = 10.0;
/// The 151x45 art, a little larger; a touch target. Grows with a long reason label.
const BUTTON_SIZE: Vec2 = Vec2::new(200.0, 56.0);
const _: () = assert!(BUTTON_SIZE.y >= 44.0);
/// Recipe list columns, left of the selected recipe.
const LIST_COLUMNS: usize = 3;
const LIST_WIDTH: f32 = grid_width(LIST_COLUMNS as f32) + SCROLLBAR_SPACE;
/// Room for the most inputs a recipe has, the arrow and the output in one row.
const MAX_INPUTS: usize = 4;
const DETAIL_WIDTH: f32 = (MAX_INPUTS as f32 + 1.0) * (SLOT_SIZE + SLOT_GAP) + ARROW_SIZE;
const SPLIT: f32 = 8.0 * KIT_SCALE;
const PANEL_WIDTH: f32 = 2.0 * PANEL_PADDING + LIST_WIDTH + SPLIT + DETAIL_WIDTH;
/// An input count the player is short of, on parchment.
const SHORT_COLOR: Color = Color::srgb(0.7, 0.1, 0.08);
/// Multiplied into the icon of a recipe the player cannot craft right now.
const UNAVAILABLE_ICON_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.45);
/// The output slot after a craft: this tint, fading back to white.
const PULSE_COLOR: Color = Color::srgb(1.0, 0.85, 0.3);
const PULSE_SECS: f32 = 0.4;
/// Crafts the selected recipe while the panel is open; the button names it. Enter crafts too: the
/// chat line leaves it alone while the panel is open (`hud::TextFocus`, set in `claim_ui.rs`).
const CRAFT_KEY: KeyCode = KeyCode::KeyC;
const CRAFT_ENTER: KeyCode = KeyCode::Enter;
const NAV_KEYS: [KeyCode; 4] = [
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
];

/// The recipe on show and whether a Workbench is within `CRAFT_STATION_REACH`.
#[derive(Resource, Default)]
struct CraftView {
    selected: u16,
    workbench_near: bool,
    /// The list order on show (`ordered_recipes`).
    order: Vec<u16>,
}

#[derive(Component, Clone, Copy)]
enum CraftButton {
    Recipe(u16),
    Craft,
}

/// Place `n` of the recipe list, its icon and its count.
#[derive(Component)]
struct ListCell(usize);
#[derive(Component)]
struct ListIcon(usize);
#[derive(Component)]
struct ListCount(usize);

#[derive(Component)]
struct DetailName;
#[derive(Component)]
struct DetailMods;
#[derive(Component)]
struct StationLine;
#[derive(Component)]
struct InputColumn(usize);
#[derive(Component)]
struct InputIcon(usize);
#[derive(Component)]
struct InputCount(usize);
#[derive(Component)]
struct OutputSlot;
#[derive(Component)]
struct OutputIcon;
#[derive(Component)]
struct OutputCount;
#[derive(Component)]
struct ButtonLabel;

/// Whether the Craft button is shown enabled; its art follows hover and press only then.
#[derive(Component)]
struct CraftEnabled(bool);

/// The output slot is pulsing until this `Time::elapsed_secs`.
#[derive(Component, Default)]
struct Pulse(f32);

/// Keys and buttons for the panel, read before the player's movement sees them.
#[derive(Resource, Default)]
struct CraftKeys {
    step: Option<(i32, i32)>,
    craft: bool,
    close: bool,
}

pub struct CraftUiPlugin;

impl Plugin for CraftUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CraftView>()
            .init_resource::<CraftKeys>()
            .add_systems(Startup, spawn_panel)
            // After `PreUpdate`, where the chat line and screenshot runs clear the keys, and before
            // `FixedUpdate`, where the player would walk on the arrows.
            .add_systems(
                RunFixedMainLoop,
                take_keys.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(Update, (click_craft, use_keys).chain().in_set(Phase::Input))
            .add_systems(
                Update,
                (
                    (track_workbench, refresh_craft).chain(),
                    skin_button,
                    pulse_output,
                )
                    .in_set(Phase::View),
            );
    }
}

/// Mirrors `rules::craft`: some tile within `CRAFT_STATION_REACH` of the player's tile holds a
/// Workbench. ponytail: the Workbench is the only station; check `Recipe::station` when there are more.
fn workbench_near(world: &World, body: &Body) -> bool {
    let Some((px, py)) = world_to_tile(body.pos.0, body.pos.1) else {
        return false;
    };
    let r = CRAFT_STATION_REACH;
    (py.saturating_sub(r)..=py + r).any(|y| {
        (px.saturating_sub(r)..=px + r)
            .any(|x| world.get(x, y).is_some_and(|tile| tile.fg == WORKBENCH))
    })
}

/// Every recipe id: the ones `can_craft` now first, then the rest, each in table order.
fn ordered_recipes(have: impl Fn(ItemId) -> u16, workbench_near: bool) -> Vec<u16> {
    let ids = (0..RECIPES.len()).filter_map(|i| u16::try_from(i).ok());
    let (mut ready, rest): (Vec<u16>, Vec<u16>) =
        ids.partition(|&id| recipe(id).is_some_and(|r| can_craft(r, &have, workbench_near)));
    ready.extend(rest);
    ready
}

/// Whether the Craft button is shown enabled: every input held in full and the station near.
fn can_craft(recipe: &Recipe, have: impl Fn(ItemId) -> u16, workbench_near: bool) -> bool {
    disabled_reason(recipe, have, workbench_near).is_none()
}

/// Why the Craft button is greyed, as its label: the first input short, else the missing station.
fn disabled_reason(
    recipe: &Recipe,
    have: impl Fn(ItemId) -> u16,
    workbench_near: bool,
) -> Option<String> {
    let short = recipe.inputs.iter().find(|&&(id, need)| have(id) < need);
    if let Some(&(id, need)) = short {
        let name = item(id).map_or("?", |def| def.name);
        return Some(format!("Need {} more {name}", need - have(id)));
    }
    (recipe.station.is_some() && !workbench_near).then(|| "Find a Workbench".to_string())
}

/// The list place after moving `(dx, dy)` from `index` in a grid of `columns` holding `len`
/// entries: left and right wrap around the whole list, up and down stop at its ends.
pub(crate) fn step_index(index: usize, len: usize, (dx, dy): (i32, i32), columns: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let index = index.min(len - 1) as i64;
    let len = len as i64;
    let moved = index + i64::from(dx) + i64::from(dy) * columns as i64;
    let next = if dy == 0 {
        moved.rem_euclid(len)
    } else if (0..len).contains(&moved) {
        moved
    } else {
        index
    };
    next as usize
}

fn spawn_panel(mut commands: Commands, assets: Res<AssetServer>) {
    let slot = || (slot_cell(&assets, SLOT_IMAGE), ItemTip(None));
    let icon = || (ImageNode::default(), icon_node(ICON_SIZE));
    let panel = spawn_centre_panel(
        &mut commands,
        &assets,
        OpenPanel::Craft,
        PANEL_WIDTH,
        |panel| {
            bag_tabs(panel, &assets, OpenPanel::Craft);
            let body = Node {
                column_gap: px(SPLIT),
                flex_grow: 1.0,
                min_height: px(0),
                ..default()
            };
            let list = Node {
                width: px(LIST_WIDTH),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                min_height: px(0),
                ..default()
            };
            let detail = Node {
                width: px(DETAIL_WIDTH),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                row_gap: px(HUD_MARGIN),
                ..default()
            };
            panel
                .spawn((body, Hovered::default()))
                .with_children(|body| {
                    body.spawn((list, Hovered::default()))
                        .with_children(|list| {
                            let grid = scroll_grid(list, ());
                            list.commands().entity(grid).with_children(|grid| {
                                for n in 0..RECIPES.len() {
                                    grid.spawn((slot(), ListCell(n), CraftButton::Recipe(0)))
                                        .with_children(|cell| {
                                            cell.spawn((icon(), ListIcon(n)));
                                            cell.spawn((count_label(), ListCount(n)));
                                        });
                                }
                            });
                        });
                    body.spawn((detail, Hovered::default()))
                        .with_children(|detail| {
                            detail.spawn((ink(""), DetailName));
                            let mods = Node {
                                column_gap: px(HUD_MARGIN),
                                flex_wrap: FlexWrap::Wrap,
                                ..default()
                            };
                            detail.spawn((mods, DetailMods));
                            detail.spawn((
                                label("Needs a Workbench nearby"),
                                TextColor(SHORT_COLOR),
                                StationLine,
                            ));
                            let recipe_row = Node {
                                column_gap: px(SLOT_GAP),
                                align_items: AlignItems::FlexStart,
                                ..default()
                            };
                            detail.spawn(recipe_row).with_children(|row| {
                                for i in 0..MAX_INPUTS {
                                    let column = Node {
                                        width: px(SLOT_SIZE),
                                        flex_direction: FlexDirection::Column,
                                        align_items: AlignItems::Center,
                                        ..default()
                                    };
                                    row.spawn((column, InputColumn(i))).with_children(|column| {
                                        column.spawn(slot()).with_children(|cell| {
                                            cell.spawn((icon(), InputIcon(i)));
                                        });
                                        column.spawn((ink(""), InputCount(i)));
                                    });
                                }
                                let arrow = Node {
                                    height: px(SLOT_SIZE),
                                    align_items: AlignItems::Center,
                                    ..default()
                                };
                                row.spawn(arrow).with_child((
                                    ImageNode::new(assets.load(ARROW_IMAGE)),
                                    icon_node(ARROW_SIZE),
                                ));
                                row.spawn((slot(), OutputSlot, Pulse::default()))
                                    .with_children(|cell| {
                                        cell.spawn((icon(), OutputIcon));
                                        cell.spawn((count_label(), OutputCount));
                                    });
                            });
                            let node = Node {
                                min_width: px(BUTTON_SIZE.x),
                                height: px(BUTTON_SIZE.y),
                                padding: UiRect::horizontal(px(2.0 * BUTTON_INSET)),
                                align_self: AlignSelf::FlexStart,
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            };
                            detail
                                .spawn((
                                    node,
                                    sliced(&assets, BUTTON_DISABLED_IMAGE, BUTTON_INSET),
                                    CraftButton::Craft,
                                    CraftEnabled(false),
                                    Hovered::default(),
                                ))
                                .with_child((
                                    label(craft_label()),
                                    TextColor(Color::WHITE),
                                    TextShadow::default(),
                                    ButtonLabel,
                                ));
                        });
                });
        },
    );
    commands
        .entity(panel)
        .insert(FitWindow { top: FIT_PANEL_TOP });
}

/// While the panel is open: whether a Workbench is in reach. Written only when it flips.
fn track_workbench(game: Res<Game>, open: Res<OpenPanel>, mut view: ResMut<CraftView>) {
    if *open != OpenPanel::Craft {
        return;
    }
    let near = game
        .me()
        .is_some_and(|me| workbench_near(&game.0.world, &me.body));
    if view.workbench_near != near {
        view.workbench_near = near;
    }
}

/// An item's icon into an existing image node, tinted.
fn set_icon(assets: &AssetServer, image: &mut ImageNode, id: Option<ItemId>, tint: Color) {
    *image = match id.and_then(item) {
        Some(def) => item_icon(assets, def.sprite).node(),
        None => ImageNode::default(),
    };
    image.color = if id.is_some() { tint } else { Color::NONE };
}

/// The enabled Craft button: "Craft (C)".
fn craft_label() -> String {
    format!("Craft ({})", key_name(CRAFT_KEY))
}

fn count_string(count: u16) -> String {
    if count > 1 {
        count.to_string()
    } else {
        String::new()
    }
}

type CellQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static ListCell,
        &'static mut CraftButton,
        &'static mut ItemTip,
        &'static mut ImageNode,
    ),
    With<ListCell>,
>;
type IconQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut ImageNode,
        Option<&'static ListIcon>,
        Option<&'static InputIcon>,
        Has<OutputIcon>,
    ),
    (
        Or<(With<ListIcon>, With<InputIcon>, With<OutputIcon>)>,
        Without<ListCell>,
    ),
>;
type TextQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Text,
        &'static mut TextColor,
        Option<&'static ListCount>,
        Option<&'static InputCount>,
        Has<OutputCount>,
        Has<ButtonLabel>,
        Has<DetailName>,
    ),
>;

/// Rewrites what changed: list order, icons, tints, counts, the detail column and the button.
#[allow(clippy::too_many_arguments)] // a Bevy system: state, events and the panel's parts
fn refresh_craft(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    open: Res<OpenPanel>,
    mut view: ResMut<CraftView>,
    mut events: MessageReader<GameEvent>,
    mut cells: CellQuery,
    mut icons: IconQuery,
    mut texts: TextQuery,
    (mut columns, mut station): (
        Query<(&InputColumn, &mut Node), Without<StationLine>>,
        Single<&mut Node, With<StationLine>>,
    ),
    (mut tips, slots, output): (
        Query<&mut ItemTip, Without<ListCell>>,
        Query<(&InputIcon, &ChildOf)>,
        Single<Entity, With<OutputSlot>>,
    ),
    (mods, mut button): (Single<Entity, With<DetailMods>>, Single<&mut CraftEnabled>),
    mut shown_selection: Local<Option<u16>>,
) {
    let mine = |event: &Event| matches!(*event, Event::InventoryChanged { player, .. } if player == game.1);
    // `filter().count()`, not `any()`: every message is read this frame.
    let changed = events.read().filter(|GameEvent(event)| mine(event)).count() > 0;
    if *open != OpenPanel::Craft || !(changed || open.is_changed() || view.is_changed()) {
        return;
    }
    let Some(me) = game.me() else { return };
    let have = |id| me.inventory.count(id);
    let near = view.workbench_near;
    let order = ordered_recipes(have, near);
    let selected = view.selected;
    if view.order != order {
        view.bypass_change_detection().order = order.clone();
    }

    // The list: place n shows `order[n]`.
    for (ListCell(n), mut button, mut tip, mut frame) in &mut cells {
        let Some(r) = order.get(*n).and_then(|&id| recipe(id).map(|r| (id, r))) else {
            continue;
        };
        let (id, r) = r;
        if !matches!(*button, CraftButton::Recipe(old) if old == id) {
            *button = CraftButton::Recipe(id);
        }
        tip.set_if_neq(ItemTip(Some(r.output)));
        let file = if id == selected {
            SLOT_SELECTED_IMAGE
        } else {
            SLOT_IMAGE
        };
        if frame.image.path().is_none_or(|p| p.to_string() != file) {
            frame.image = assets.load(file);
        }
    }

    let Some(r) = recipe(selected) else { return };
    let (output_item, output_count) = r.output;
    for (mut image, list, input, is_output) in &mut icons {
        if let Some(ListIcon(n)) = list {
            let Some(r) = order.get(*n).and_then(|&id| recipe(id)) else {
                continue;
            };
            let tint = if can_craft(r, have, near) {
                Color::WHITE
            } else {
                UNAVAILABLE_ICON_COLOR
            };
            set_icon(&assets, &mut image, Some(r.output.0), tint);
        } else if let Some(InputIcon(i)) = input {
            let id = r.inputs.get(*i).map(|&(id, _)| id);
            set_icon(&assets, &mut image, id, Color::WHITE);
        } else if is_output {
            set_icon(&assets, &mut image, Some(output_item), Color::WHITE);
        }
    }
    let reason = disabled_reason(r, have, near);
    for (mut text, mut colour, list, input, is_output, is_label, is_name) in &mut texts {
        let new = if let Some(ListCount(n)) = list {
            order
                .get(*n)
                .and_then(|&id| recipe(id))
                .map(|r| count_string(r.output.1))
        } else if let Some(InputCount(i)) = input {
            r.inputs.get(*i).map(|&(id, need)| {
                let held = have(id);
                let c = if held < need { SHORT_COLOR } else { INK_COLOR };
                colour.set_if_neq(TextColor(c));
                format!("{held}/{need}")
            })
        } else if is_output {
            Some(count_string(output_count))
        } else if is_label {
            Some(reason.clone().unwrap_or_else(craft_label))
        } else if is_name {
            Some(item(output_item).map_or("?", |def| def.name).to_string())
        } else {
            None
        };
        if let Some(new) = new
            && text.0 != new
        {
            text.0 = new;
        }
    }
    for (InputColumn(i), mut node) in &mut columns {
        let display = if *i < r.inputs.len() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    let display = if r.station.is_some() && !near {
        Display::Flex
    } else {
        Display::None
    };
    if station.display != display {
        station.display = display;
    }
    // Input and output slots tell the tooltip what they hold.
    for (InputIcon(i), parent) in &slots {
        if let Ok(mut tip) = tips.get_mut(parent.parent()) {
            tip.set_if_neq(ItemTip(r.inputs.get(*i).map(|&(id, _)| (id, 1))));
        }
    }
    if let Ok(mut tip) = tips.get_mut(*output) {
        tip.set_if_neq(ItemTip(Some((output_item, output_count))));
    }
    let enabled = reason.is_none();
    if button.0 != enabled {
        button.0 = enabled;
    }
    if *shown_selection != Some(selected) {
        *shown_selection = Some(selected);
        commands
            .entity(*mods)
            .despawn_children()
            .with_children(|row| spawn_mods(row, &assets, output_item, INK_COLOR));
    }
}

/// A click on a recipe selects it; on the Craft button it asks the rules, greyed or not (the
/// refusal then says why).
fn click_craft(
    mouse: Res<ButtonInput<MouseButton>>,
    open: Res<OpenPanel>,
    buttons: Query<(&CraftButton, &Hovered)>,
    mut view: ResMut<CraftView>,
    mut actions: MessageWriter<ActionMsg>,
    mut sfx: MessageWriter<Sfx>,
) {
    if *open != OpenPanel::Craft || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let Some((&button, _)) = buttons.iter().find(|(_, hovered)| hovered.get()) else {
        return;
    };
    sfx.write(Sfx::UiClick);
    match button {
        CraftButton::Recipe(id) => {
            if view.selected != id {
                view.selected = id;
            }
        }
        CraftButton::Craft => {
            actions.write(ActionMsg(Action::Craft {
                recipe: view.selected,
            }));
        }
    }
}

/// While the panel is open: arrows / d-pad, the craft key / South and East, taken from the
/// keyboard so the player does not also walk. Keys the chat line or a screenshot run cleared in
/// `PreUpdate` are already gone.
fn take_keys(
    open: Res<OpenPanel>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut out: ResMut<CraftKeys>,
) {
    if *open != OpenPanel::Craft {
        return;
    }
    let pad = |button| pads.iter().any(|pad| pad.just_pressed(button));
    let key = |code| keys.just_pressed(code);
    let dx = i32::from(key(KeyCode::ArrowRight) || pad(GamepadButton::DPadRight))
        - i32::from(key(KeyCode::ArrowLeft) || pad(GamepadButton::DPadLeft));
    let dy = i32::from(key(KeyCode::ArrowDown) || pad(GamepadButton::DPadDown))
        - i32::from(key(KeyCode::ArrowUp) || pad(GamepadButton::DPadUp));
    if dx != 0 || dy != 0 {
        out.step = Some((dx, dy));
    }
    out.craft |= key(CRAFT_KEY) || key(CRAFT_ENTER) || pad(GamepadButton::South);
    out.close |= pad(GamepadButton::East);
    for code in NAV_KEYS.into_iter().chain([CRAFT_KEY, CRAFT_ENTER]) {
        if keys.pressed(code) {
            keys.reset(code);
        }
    }
}

fn use_keys(
    mut keys: ResMut<CraftKeys>,
    mut open: ResMut<OpenPanel>,
    mut view: ResMut<CraftView>,
    mut actions: MessageWriter<ActionMsg>,
    mut sfx: MessageWriter<Sfx>,
) {
    let CraftKeys { step, craft, close } = std::mem::take(&mut *keys);
    if *open != OpenPanel::Craft {
        return;
    }
    // Opened (by its tab, the wrench, or a tab key): the selection starts on the first recipe.
    if open.is_changed()
        && let Some(&first) = view.order.first()
        && view.selected != first
    {
        view.selected = first;
    }
    if let Some(step) = step {
        let index = view.order.iter().position(|&id| id == view.selected);
        let next = step_index(index.unwrap_or(0), view.order.len(), step, LIST_COLUMNS);
        if let Some(&id) = view.order.get(next)
            && id != view.selected
        {
            view.selected = id;
            sfx.write(Sfx::UiClick);
        }
    }
    if craft {
        sfx.write(Sfx::UiClick);
        actions.write(ActionMsg(Action::Craft {
            recipe: view.selected,
        }));
    }
    if close {
        *open = OpenPanel::None;
    }
}

/// The Craft button's art: disabled, or hover and pressed versions. Writes only on a change.
fn skin_button(
    assets: Res<AssetServer>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut buttons: Query<(&CraftEnabled, &Hovered, &mut ImageNode)>,
) {
    for (enabled, hovered, mut image) in &mut buttons {
        let file = match (enabled.0, hovered.get(), mouse.pressed(MouseButton::Left)) {
            (false, ..) => BUTTON_DISABLED_IMAGE,
            (true, true, true) => BUTTON_PRESSED_IMAGE,
            (true, true, false) => BUTTON_HOVER_IMAGE,
            _ => BUTTON_IMAGE,
        };
        if image
            .image
            .path()
            .is_none_or(|path| path.to_string() != file)
        {
            image.image = assets.load(file);
        }
    }
}

/// The output slot flashes after the local player's craft and fades back.
fn pulse_output(
    time: Res<Time>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    slot: Single<(&mut Pulse, &mut ImageNode), With<OutputSlot>>,
) {
    let (mut pulse, mut image) = slot.into_inner();
    let now = time.elapsed_secs();
    let crafted = events
        .read()
        .filter(|GameEvent(e)| matches!(*e, Event::Crafted { player, .. } if player == game.1))
        .count()
        > 0;
    if crafted {
        pulse.0 = now + PULSE_SECS;
    }
    let left = ((pulse.0 - now) / PULSE_SECS).clamp(0.0, 1.0);
    let colour = Color::WHITE.mix(&PULSE_COLOR, left);
    if image.color != colour {
        image.color = colour;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{LOG_END, PLANKS};

    fn have(planks: u16, logs: u16) -> impl Fn(ItemId) -> u16 {
        move |id: ItemId| match id {
            PLANKS => planks,
            LOG_END => logs,
            _ => 0,
        }
    }

    #[test]
    fn every_recipe_is_listed_craftable_first() {
        let empty = ordered_recipes(have(0, 0), false);
        assert_eq!(empty.len(), RECIPES.len(), "station recipes too");
        assert!(empty.windows(2).all(|w| w[0] < w[1]), "table order");
        // 10 planks + 4 log end: the Workbench (0) anywhere; crate (1), ladder (2) and the wooden
        // pickaxe (3) only next to a Workbench.
        let alone = ordered_recipes(have(10, 4), false);
        assert_eq!(alone[0], 0);
        assert!(alone[1..].windows(2).all(|w| w[0] < w[1]));
        let near = ordered_recipes(have(10, 4), true);
        assert_eq!(&near[..4], &[0, 1, 2, 3]);
        assert_eq!(near.len(), RECIPES.len());
    }

    #[test]
    fn the_craft_button_needs_every_input_and_the_station() {
        // Recipe 0: 10 planks + 4 log end, no station. Recipe 1: 8 planks + 2 log end, Workbench.
        let workbench = &RECIPES[0];
        assert!(can_craft(workbench, have(10, 4), false));
        assert!(can_craft(workbench, have(50, 50), true));
        assert!(!can_craft(workbench, have(9, 4), false), "one plank short");
        assert!(!can_craft(workbench, have(10, 3), true), "one log short");
        let crate_recipe = &RECIPES[1];
        assert!(can_craft(crate_recipe, have(8, 2), true));
        assert!(
            !can_craft(crate_recipe, have(8, 2), false),
            "no Workbench in reach"
        );
        assert!(!can_craft(crate_recipe, have(7, 2), true));
    }

    #[test]
    fn the_greyed_button_says_why() {
        let workbench = &RECIPES[0];
        assert_eq!(disabled_reason(workbench, have(10, 4), false), None);
        let planks = item(PLANKS).unwrap().name;
        assert_eq!(
            disabled_reason(workbench, have(7, 0), false),
            Some(format!("Need 3 more {planks}")),
            "the first input short"
        );
        let crate_recipe = &RECIPES[1];
        assert_eq!(
            disabled_reason(crate_recipe, have(8, 2), false).as_deref(),
            Some("Find a Workbench")
        );
    }

    #[test]
    fn arrows_wrap_sideways_and_stop_at_the_ends_vertically() {
        assert_eq!(step_index(0, 10, (1, 0), 3), 1);
        assert_eq!(step_index(9, 10, (1, 0), 3), 0, "wraps to the start");
        assert_eq!(step_index(0, 10, (-1, 0), 3), 9, "wraps to the end");
        assert_eq!(step_index(1, 10, (0, 1), 3), 4);
        assert_eq!(step_index(8, 10, (0, 1), 3), 8, "no row below");
        assert_eq!(step_index(1, 10, (0, -1), 3), 1, "no row above");
        assert_eq!(step_index(20, 10, (0, 0), 3), 9, "clamped into the list");
        assert_eq!(step_index(0, 0, (1, 0), 3), 0);
    }
}
