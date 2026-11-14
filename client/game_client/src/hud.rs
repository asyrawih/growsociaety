//! Hotbar, inventory, wardrobe, store, claim panel, gem counter and messages. Everything here
//! changes when a `GameEvent` or `ActionResult` arrives or the player opens/selects something,
//! never by polling the state every frame. The HUD decides nothing: buying, equipping and toggling
//! send an `Action`. Panels use the wood-and-parchment kit in assets/ui/ (assets/README.md, "UI").

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::VisualBox;
use bevy::ui_widgets::{ControlOrientation, ScrollArea, Scrollbar, ScrollbarThumb};
use game_shared::claims::CLAIMS;
use game_shared::inventory::{MAX_UPGRADES, SLOTS_PER_UPGRADE, upgrade_cost};
use game_shared::items::{ITEMS, ItemId, ItemKind, Slot, equip_price, item, slot_of};
use game_shared::levels::{MAX_LEVEL, xp_to_next};
use game_shared::rules::{Action, Event, Player, Reject};
use game_shared::world::World;

use crate::audio::Sfx;
use crate::chat::OpenChat;
use crate::interact::InspectedClaim;
use crate::screenshot;
use crate::settings::{Settings, SettingsTab};
use crate::state::{ActionMsg, ActionResult, Game, GameEvent, Phase};
use crate::wardrobe::{PREVIEW_BELOW_FEET, PREVIEW_SIZE, WardrobePreview};
use crate::world_view::item_icon;

const HOTBAR_SLOTS: usize = 8;
const SELECT_KEYS: [KeyCode; HOTBAR_SLOTS] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
];
const INVENTORY_KEYS: [KeyCode; 2] = [KeyCode::KeyE, KeyCode::KeyI];
const STORE_KEY: KeyCode = KeyCode::KeyB;
const CLOSE_KEY: KeyCode = KeyCode::Escape;

// The UI kit (assets/ui/). Sizes and insets are in the art's own pixels; it is drawn at
// `KIT_SCALE` logical px per art px.
pub(crate) const KIT_SCALE: f32 = 2.0;
const PANEL_IMAGE: &str = "ui/panel.png";
const PANEL_INSET: f32 = 18.0;
const SLOT_IMAGE: &str = "ui/slot.png";
const SLOT_SELECTED_IMAGE: &str = "ui/slot_selected.png";
const SLOT_WORN_IMAGE: &str = "ui/slot_worn.png";
const SLOT_EMPTY_IMAGE: &str = "ui/slot_equip_empty.png";
const SLOT_INSET: f32 = 8.0;
const TAB_IMAGE: &str = "ui/tab.png";
const TAB_INSET: f32 = 8.0;
const FRAME_IMAGE: &str = "ui/preview_frame.png";
const FRAME_INSET: f32 = 20.0;
const CLOSE_IMAGE: &str = "ui/close.png";
/// `ui/slot_icon_<name>.png`, in `Slot::ALL` order.
const SLOT_ICON_NAMES: [&str; 8] = [
    "hair", "hat", "face", "shirt", "pants", "shoes", "back", "hand",
];

// Px (logical). Slots are touch targets: at least 44.
const SLOT_SIZE: f32 = 37.0 * KIT_SCALE;
const _: () = assert!(SLOT_SIZE >= 44.0);
pub(crate) const SLOT_GAP: f32 = 4.0;
const ICON_SIZE: f32 = 48.0;
/// Item count, inset from the slot's bottom-right corner so it sits inside the rim.
const COUNT_INSET: f32 = 6.0;
const TAB_HEIGHT: f32 = 23.0 * KIT_SCALE;
const TAB_MIN_WIDTH: f32 = 51.0 * KIT_SCALE;
const TAB_PADDING: f32 = 12.0;
const CLOSE_SIZE: f32 = 27.0 * KIT_SCALE;
/// How far the close button sticks out past the panel's top-right corner.
const CLOSE_OVERHANG: f32 = CLOSE_SIZE / 3.0;
pub(crate) const HUD_MARGIN: f32 = 8.0;
/// A claims-tab row of the store: an unscaled icon plus a margin above and below; a touch target.
const ROW_ICON_SIZE: f32 = 32.0;
const ROW_MIN_HEIGHT: f32 = ROW_ICON_SIZE + 2.0 * HUD_MARGIN;
const _: () = assert!(ROW_MIN_HEIGHT >= 44.0);
/// Inside a 9-sliced slot stretched into a row: clear of its rim.
const ROW_PADDING: f32 = SLOT_INSET * KIT_SCALE;
const PANEL_PADDING: f32 = 32.0;
const PANEL_TOP: f32 = 48.0;
const STORE_COLUMNS: f32 = 8.0;
const fn grid_width(columns: f32) -> f32 {
    columns * (SLOT_SIZE + SLOT_GAP) - SLOT_GAP
}
/// A scrolling grid's scrollbar, right of the grid (`scroll_grid`).
pub(crate) const SCROLLBAR_WIDTH: f32 = 8.0;
/// Px of width a scrolling grid adds beside its slots: the gap and the scrollbar.
pub(crate) const SCROLLBAR_SPACE: f32 = SLOT_GAP + SCROLLBAR_WIDTH;
const SCROLL_TRACK_COLOR: Color = Color::srgba(0.29, 0.17, 0.08, 0.25);
const SCROLL_THUMB_COLOR: Color = INK_COLOR;
const MIN_THUMB_LENGTH: f32 = 24.0;
const STORE_WIDTH: f32 = grid_width(STORE_COLUMNS) + SCROLLBAR_SPACE + 2.0 * PANEL_PADDING;
/// Roughly one line of `TEXT_FONT_SIZE` text.
const LINE_HEIGHT: f32 = 20.0;
/// Px kept free at the bottom of the window for the hotbar column (message line, item name,
/// hotbar) and a margin: a centre panel ends above it.
/// Top edge of the fitted centre panels: below the top bar, close button included, since a wide
/// panel reaches under it.
const FIT_PANEL_TOP: f32 = HUD_MARGIN + TOPBAR_HEIGHT + CLOSE_OVERHANG + HUD_MARGIN;
const HOTBAR_RESERVE: f32 = HUD_MARGIN + SLOT_SIZE + 2.0 * (LINE_HEIGHT + SLOT_GAP) + HUD_MARGIN;
/// A panel never gets shorter than this, however small the window.
const MIN_PANEL_HEIGHT: f32 = 200.0;
/// Wardrobe filter buttons: touch targets.
const FILTER_SIZE: f32 = 44.0;
const FILTER_ICON_SIZE: f32 = 32.0;
/// Wardrobe: preview frame (art px, owner's minimum 136x150) between the two slot columns.
const FRAME_SIZE: Vec2 = Vec2::new(136.0 * KIT_SCALE, 150.0 * KIT_SCALE);
/// Art px of wood between the frame's outer edge and its inner (grey) area.
const FRAME_BORDER: f32 = 7.0;
/// Art px between the rig's feet and the frame's inner bottom edge.
const FEET_ABOVE_FRAME: f32 = 10.0;
/// The rig is drawn at 2x the art's pixels (assets/README.md).
const PREVIEW_SCALE: f32 = 2.0 * KIT_SCALE;
const WARDROBE_GAP: f32 = 4.0;
/// Between the preview (with its slot columns) and the equipment grid on its right.
const WARDROBE_SPLIT: f32 = 16.0;
const WARDROBE_LEFT: f32 = 2.0 * SLOT_SIZE + 2.0 * WARDROBE_GAP + FRAME_SIZE.x;
/// The inventory and wardrobe grids take as many columns as the window has room for.
const INVENTORY_GRID: GridPanel = GridPanel {
    beside: 0.0,
    columns: (6, 10),
};
const WARDROBE_GRID: GridPanel = GridPanel {
    beside: WARDROBE_LEFT + WARDROBE_SPLIT,
    columns: (3, 10),
};
/// assets/README.md, "Font": glyphs sit on a 16 px grid, so only multiples of 16 stay crisp.
const FONT: &str = "fonts/growsociaety_pixel.ttf";
const TEXT_FONT_SIZE: f32 = 16.0;
const GEM_ICON: &str = "blocks/45_gem.png";
/// Top bar, top-right corner (assets/README.md, "Top bar and settings"): the strip at its art's
/// own pixels, round 41 px buttons in 44 px touch targets, a 32 px icon on each.
const TOPBAR_IMAGE: &str = "ui/settings/topbar.png";
const TOPBAR_INSET: f32 = 10.0;
const TOPBAR_PADDING: f32 = 6.0;
const TOP_BUTTON_IMAGE: &str = "ui/settings/icon_button.png";
const TOP_BUTTON_ART: f32 = 41.0;
const TOP_BUTTON_SIZE: f32 = 44.0;
const TOP_ICON_SIZE: f32 = 32.0;
const TOPBAR_HEIGHT: f32 = TOP_BUTTON_SIZE + 2.0 * TOPBAR_PADDING;
const MIC_ON_ICON: &str = "ui/voice_mic_on.png";
const MIC_MUTED_ICON: &str = "ui/voice_mic_muted.png";
/// Top-bar buttons, left to right, with their icons.
const TOP_BUTTONS: [(TopButton, &str); 6] = [
    (TopButton::Store, "ui/settings/icon_shop.png"),
    (TopButton::Inventory, "ui/settings/icon_inventory.png"),
    (TopButton::Wardrobe, "ui/settings/icon_wardrobe.png"),
    (TopButton::Chat, "ui/settings/icon_chat.png"),
    (TopButton::Voice, MIC_MUTED_ICON),
    (TopButton::Settings, "ui/settings_gear.png"),
];
const GEM_ICON_SIZE: f32 = 24.0;
/// The XP bar beside the level, under the top-left counters.
const XP_BAR_SIZE: Vec2 = Vec2::new(64.0, 6.0);
const XP_BAR_BACK: Color = Color::srgb(0.2, 0.2, 0.24);
const XP_BAR_FILL: Color = Color::srgb(0.45, 0.85, 1.0);

pub(crate) const PANEL_COLOR: Color = Color::srgba(0.08, 0.08, 0.1, 0.7);
const WINDOW_COLOR: Color = Color::srgba(0.12, 0.12, 0.16, 0.95);
pub(crate) const BUTTON_COLOR: Color = Color::srgb(0.22, 0.24, 0.32);
/// Text on the parchment of a panel.
pub(crate) const INK_COLOR: Color = Color::srgb(0.29, 0.17, 0.08);
const DISABLED_TEXT_COLOR: Color = Color::srgb(0.55, 0.5, 0.45);
/// Multiplied into a tab that is not the one on show.
pub(crate) const INACTIVE_TAB_COLOR: Color = Color::srgb(0.5, 0.5, 0.5);
/// Marks a worn item that is also selected (its slot shows the selection rim instead of the badge).
const WORN_TEXT_COLOR: Color = Color::srgb(0.45, 0.9, 0.45);
/// Multiplied into the icon of a store item the player cannot pay for.
const DISABLED_ICON_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.35);

/// Store tabs, left to right, and their labels.
const STORE_TABS: [(StoreTab, &str); 9] = [
    (StoreTab::Claims, "Claims"),
    (StoreTab::Equip(Slot::Hair), "Hair"),
    (StoreTab::Equip(Slot::Hat), "Hat"),
    (StoreTab::Equip(Slot::Face), "Face"),
    (StoreTab::Equip(Slot::Shirt), "Shirt"),
    (StoreTab::Equip(Slot::Pants), "Pants"),
    (StoreTab::Equip(Slot::Shoes), "Shoes"),
    (StoreTab::Equip(Slot::Back), "Wings"),
    (StoreTab::Equip(Slot::Hand), "Pickaxes"),
];
/// The inventory/wardrobe tabs.
const BAG_TABS: [(OpenPanel, &str); 2] = [
    (OpenPanel::Inventory, "Inventory"),
    (OpenPanel::Wardrobe, "Wardrobe"),
];
/// Wardrobe slot columns, left and right of the preview.
const WORN_COLUMNS: [[Slot; 4]; 2] = [
    [Slot::Hair, Slot::Hat, Slot::Face, Slot::Shirt],
    [Slot::Pants, Slot::Shoes, Slot::Back, Slot::Hand],
];

/// Seconds a message stays on screen.
const TOAST_SECS: f32 = 3.0;

/// Index of the selected inventory slot (an index into `Inventory::slots`). The hotbar and the
/// inventory grid only offer non-equipment items; equipment is selected in the wardrobe.
#[derive(Resource, Default)]
pub struct Selected(pub usize);

impl Selected {
    /// The item in the selected slot, if the slot holds one.
    pub fn item(&self, game: &Game) -> Option<ItemId> {
        let slots = game.me()?.inventory.slots();
        slots.get(self.0).map(|slot| slot.0)
    }
}

/// The equipment item selected in the wardrobe grid.
#[derive(Resource, Default)]
pub struct WardrobeSelected(pub Option<ItemId>);

/// The slot the wardrobe grid is narrowed to; `None` shows all equipment.
#[derive(Resource, Default)]
struct WardrobeFilter(Option<Slot>);

/// Which centre panel is open.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenPanel {
    #[default]
    None,
    Inventory,
    Wardrobe,
    Store,
    /// The settings window (`settings.rs`).
    Settings,
}

/// The item Q (drop) and X (trash) act on: the one selected in the wardrobe grid while the
/// wardrobe is open, else the selected inventory item.
pub fn discard_target(
    game: &Game,
    open: OpenPanel,
    selected: &Selected,
    wardrobe: &WardrobeSelected,
) -> Option<ItemId> {
    if open == OpenPanel::Wardrobe {
        let me = game.me()?;
        wardrobe.0.filter(|&id| me.inventory.count(id) > 0)
    } else {
        selected.item(game)
    }
}

/// The store tab on show: claims and the backpack, or the equipment of one slot.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
enum StoreTab {
    #[default]
    Claims,
    Equip(Slot),
}

/// The HUD's pixel font, loaded once.
#[derive(Resource)]
struct HudFont(Handle<Font>);

impl FromWorld for HudFont {
    fn from_world(world: &mut bevy::ecs::world::World) -> Self {
        Self(world.resource::<AssetServer>().load(FONT))
    }
}

/// HUD text: gets the `HudFont` as soon as it is spawned (`use_hud_font`).
#[derive(Component)]
pub(crate) struct HudText;

/// A line of text for the message area, from client code that has something to say.
#[derive(Message)]
pub struct Notice(pub String);

/// What a click on a HUD element does. Every clickable element also carries `Hovered`.
#[derive(Component, Clone, Copy)]
enum UiButton {
    /// An inventory slot (hotbar or inventory grid): select it.
    Slot(usize),
    /// An item in the wardrobe grid: select it and wear it.
    Equip(ItemId),
    /// A filled wardrobe slot: take the item off.
    Worn(Slot),
    /// A wardrobe filter button: show only this slot's equipment (`None`: all).
    Filter(Option<Slot>),
    /// The inventory/wardrobe tabs.
    Show(OpenPanel),
    Close,
    Tab(StoreTab),
    Buy(ItemId),
    BuyUpgrade,
    ToggleClaimPublic,
    Top(TopButton),
}

/// A top-bar button: opens (or closes) its panel; Chat opens the chat line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TopButton {
    Store,
    Inventory,
    Wardrobe,
    Chat,
    /// The settings on the Voice tab; its icon shows whether the mic can be on.
    Voice,
    Settings,
}

/// The icon of the top bar's voice button.
#[derive(Component)]
struct TopbarMic;

#[derive(Component)]
struct HotbarRow;
#[derive(Component)]
struct InventoryGrid;
/// A wardrobe slot column; the index into `WORN_COLUMNS`.
#[derive(Component)]
struct WornColumn(usize);
#[derive(Component)]
struct WardrobeGrid;
#[derive(Component)]
struct FilterRow;
#[derive(Component)]
struct StoreList;
/// The store's equipment grid (a `scroll_grid`).
#[derive(Component)]
struct StoreGrid;

/// A panel put `top` px from the window's top edge, its height capped so it ends above the
/// hotbar (`fit_panels`); its scrolling grid takes what is left.
#[derive(Component)]
pub(crate) struct FitWindow {
    pub top: f32,
}

/// A centre panel whose width follows the window: as many grid columns (within `columns`) as fit
/// beside `beside` px of other content.
#[derive(Component, Clone, Copy)]
struct GridPanel {
    beside: f32,
    columns: (usize, usize),
}

impl GridPanel {
    /// The panel's width in a window `window_width` px wide.
    fn width_for(self, window_width: f32) -> f32 {
        let fixed = self.beside + SCROLLBAR_SPACE + 2.0 * PANEL_PADDING;
        let room = window_width - 2.0 * HUD_MARGIN - fixed;
        let columns = columns_for_width(room, self.columns.0, self.columns.1);
        fixed + grid_width(columns as f32)
    }
}
/// A centre panel and the `OpenPanel` value that shows it.
#[derive(Component)]
struct PanelOf(OpenPanel);
#[derive(Component)]
struct ClaimPanel;
#[derive(Component)]
struct ClaimInfo;
#[derive(Component)]
struct ClaimPublicLabel;
/// Name of the selected item. Seeds share one sprite, so the name is the reliable way to tell them apart.
#[derive(Component)]
struct SelectedName;
#[derive(Component)]
struct GemCount;
#[derive(Component)]
struct LevelLabel;
#[derive(Component)]
struct XpFill;
/// The message line; `until` is when it is cleared (`Time::elapsed_secs`).
#[derive(Component, Default)]
struct Toast {
    until: f32,
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        // A screenshot run can ask for a panel (and a store tab: "store-pickaxes"), since nobody
        // is there to press the key.
        let request = screenshot::ui_request().unwrap_or_default();
        let open = match request.split('-').next() {
            Some("inventory") => OpenPanel::Inventory,
            Some("wardrobe") => OpenPanel::Wardrobe,
            Some("store") => OpenPanel::Store,
            Some("settings") => OpenPanel::Settings,
            _ => OpenPanel::None,
        };
        let tab = STORE_TABS
            .iter()
            .find(|(_, name)| request.strip_prefix("store-") == Some(&name.to_lowercase()))
            .map_or(StoreTab::Claims, |&(tab, _)| tab);
        app.init_resource::<Selected>()
            .init_resource::<HudFont>()
            .add_observer(use_hud_font)
            .init_resource::<WardrobeSelected>()
            .init_resource::<WardrobeFilter>()
            .insert_resource(open)
            .insert_resource(tab)
            .add_message::<Notice>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, (toggle_panels, click_buttons).in_set(Phase::Input))
            .add_systems(
                Update,
                (
                    show_panels,
                    fit_panels,
                    refresh_slots,
                    refresh_wardrobe,
                    refresh_gems,
                    refresh_level,
                    refresh_store,
                    refresh_claim_panel,
                    refresh_topbar_mic,
                    show_toasts,
                )
                    .in_set(Phase::View),
            );
    }
}

/// True while the pointer is on any HUD element. The one "is this click for the UI" check:
/// `interact` asks it before treating the pointer as being over the world, and the camera before
/// zooming with the wheel. Every node that can be under the pointer carries `Hovered`.
pub fn pointer_over_ui(elements: &Query<&Hovered>) -> bool {
    elements.iter().any(Hovered::get)
}

pub(crate) fn label(text: impl Into<String>) -> (Text, TextFont, HudText) {
    let font = TextFont::from_font_size(TEXT_FONT_SIZE);
    (Text::new(text), font, HudText)
}

fn use_hud_font(added: On<Add<HudText>>, font: Res<HudFont>, mut texts: Query<&mut TextFont>) {
    if let Ok(mut text) = texts.get_mut(added.entity) {
        text.font = FontSource::Handle(font.0.clone());
    }
}

/// A label on parchment.
pub(crate) fn ink(text: impl Into<String>) -> impl Bundle {
    (label(text), TextColor(INK_COLOR))
}

fn icon_node(size: f32) -> Node {
    Node {
        width: px(size),
        height: px(size),
        ..default()
    }
}

/// A kit image stretched over its whole node, corners kept crisp (9-slice).
pub(crate) fn sliced(assets: &AssetServer, file: &'static str, inset: f32) -> ImageNode {
    ImageNode {
        image: assets.load(file),
        image_mode: NodeImageMode::Sliced(TextureSlicer {
            border: BorderRect::all(inset),
            max_corner_scale: KIT_SCALE,
            ..default()
        }),
        visual_box: VisualBox::BorderBox,
        ..default()
    }
}

fn slot_node() -> Node {
    Node {
        width: px(SLOT_SIZE),
        height: px(SLOT_SIZE),
        flex_shrink: 0.0,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    }
}

/// An item slot drawn with one of the slot images.
fn slot_cell(assets: &AssetServer, file: &'static str) -> impl Bundle {
    (
        slot_node(),
        sliced(assets, file, SLOT_INSET),
        Hovered::default(),
    )
}

fn item_icon_node(assets: &AssetServer, id: ItemId) -> Option<impl Bundle> {
    let def = item(id)?;
    Some((item_icon(assets, def.sprite).node(), icon_node(ICON_SIZE)))
}

/// The count in a slot's bottom-right corner, when there is more than one.
fn count_text(count: u16) -> Option<impl Bundle> {
    (count > 1).then(|| {
        (
            label(count.to_string()),
            Node {
                position_type: PositionType::Absolute,
                right: px(COUNT_INSET),
                bottom: px(COUNT_INSET / 2.0),
                ..default()
            },
        )
    })
}

/// A tab button's node and art; the one on show is drawn at full brightness.
pub(crate) fn tab_node(assets: &AssetServer, active: bool) -> (Node, ImageNode) {
    let tint = if active {
        Color::WHITE
    } else {
        INACTIVE_TAB_COLOR
    };
    (
        Node {
            min_width: px(TAB_MIN_WIDTH),
            height: px(TAB_HEIGHT),
            padding: UiRect::horizontal(px(TAB_PADDING)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        sliced(assets, TAB_IMAGE, TAB_INSET).with_color(tint),
    )
}

fn tab(assets: &AssetServer, button: UiButton, active: bool) -> impl Bundle {
    (tab_node(assets, active), button, Hovered::default())
}

fn row(gap: f32) -> Node {
    Node {
        column_gap: px(gap),
        ..default()
    }
}

fn wrapping_grid() -> Node {
    Node {
        flex_wrap: FlexWrap::Wrap,
        column_gap: px(SLOT_GAP),
        row_gap: px(SLOT_GAP),
        ..default()
    }
}

/// Px: the tallest a panel whose top edge is `top` px down may be in a window `window_height`
/// px tall, so that it ends above the hotbar.
pub(crate) fn panel_max_height(window_height: f32, top: f32) -> f32 {
    (window_height - top - HOTBAR_RESERVE).max(MIN_PANEL_HEIGHT)
}

/// How many slot columns fit in `width` px, kept within `min..=max`.
fn columns_for_width(width: f32, min: usize, max: usize) -> usize {
    let fit = ((width + SLOT_GAP) / (SLOT_SIZE + SLOT_GAP))
        .floor()
        .max(0.0) as usize;
    fit.clamp(min, max)
}

/// The owned equipment the wardrobe grid shows: every equipment stack, or one slot's.
fn wardrobe_items(slots: &[(ItemId, u16)], filter: Option<Slot>) -> Vec<(ItemId, u16)> {
    let shown = |id: ItemId| slot_of(id).is_some_and(|slot| filter.is_none_or(|f| f == slot));
    slots.iter().copied().filter(|&(id, _)| shown(id)).collect()
}

/// A wrapping slot grid that scrolls vertically (wheel or trackpad: `ScrollArea`), with a
/// scrollbar on its right whose thumb shows and drags the scroll position (`Scrollbar`). It takes
/// the height its parent column leaves; `grid` goes on the grid entity, which holds the slots and
/// keeps its `ScrollPosition` when they are rebuilt. Returns the grid entity.
pub(crate) fn scroll_grid(parent: &mut ChildSpawnerCommands, grid: impl Bundle) -> Entity {
    let area = Node {
        flex_grow: 1.0,
        min_height: px(0),
        column_gap: px(SLOT_GAP),
        ..default()
    };
    let mut entity = Entity::PLACEHOLDER;
    parent
        .spawn((area, Hovered::default()))
        .with_children(|area| {
            let node = Node {
                flex_grow: 1.0,
                flex_basis: px(0),
                min_height: px(0),
                overflow: Overflow::scroll_y(),
                align_content: AlignContent::FlexStart,
                ..wrapping_grid()
            };
            entity = area
                .spawn((node, ScrollArea, Hovered::default(), grid))
                .id();
            let track = Node {
                width: px(SCROLLBAR_WIDTH),
                flex_shrink: 0.0,
                ..default()
            };
            area.spawn((
                track,
                BackgroundColor(SCROLL_TRACK_COLOR),
                Scrollbar::new(entity, ControlOrientation::Vertical, MIN_THUMB_LENGTH),
                Hovered::default(),
            ))
            .with_child((
                ScrollbarThumb::default(),
                BackgroundColor(SCROLL_THUMB_COLOR),
                Hovered::default(),
            ));
        });
    entity
}

/// A window, horizontally centred, with the panel art and a close button in its top-right corner.
/// Its `Hovered` keeps clicks on it away from the world.
pub(crate) fn spawn_centre_panel(
    commands: &mut Commands,
    assets: &AssetServer,
    panel: OpenPanel,
    width: f32,
    children: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    let node = Node {
        position_type: PositionType::Absolute,
        left: percent(50),
        top: px(PANEL_TOP),
        margin: UiRect::left(px(-width / 2.0)),
        width: px(width),
        padding: UiRect::all(px(PANEL_PADDING)),
        flex_direction: FlexDirection::Column,
        row_gap: px(HUD_MARGIN),
        display: Display::None,
        ..default()
    };
    let close = (
        Node {
            position_type: PositionType::Absolute,
            top: px(-CLOSE_OVERHANG),
            right: px(-CLOSE_OVERHANG),
            width: px(CLOSE_SIZE),
            height: px(CLOSE_SIZE),
            ..default()
        },
        ImageNode::new(assets.load(CLOSE_IMAGE)),
        UiButton::Close,
        Hovered::default(),
    );
    commands
        .spawn((
            PanelOf(panel),
            Hovered::default(),
            node,
            sliced(assets, PANEL_IMAGE, PANEL_INSET),
        ))
        .with_children(|parent| {
            children(parent);
            parent.spawn(close);
        })
        .id()
}

/// The Inventory / Wardrobe tab row at the top of both panels.
fn bag_tabs(parent: &mut ChildSpawnerCommands, assets: &AssetServer, active: OpenPanel) {
    parent.spawn(row(SLOT_GAP)).with_children(|tabs| {
        for (panel, name) in BAG_TABS {
            tabs.spawn(tab(assets, UiButton::Show(panel), panel == active))
                .with_children(|button| {
                    button.spawn(label(name));
                });
        }
    });
}

fn spawn_hud(
    mut commands: Commands,
    assets: Res<AssetServer>,
    preview: Res<WardrobePreview>,
    game: Res<Game>,
) {
    let preview_image = preview.image.clone();
    // Gems, level and XP: top-left corner.
    let (gem_count, level, xp) = game.me().map_or((0, 1, 0), |me| (me.gems, me.level, me.xp));
    let gems = Node {
        position_type: PositionType::Absolute,
        left: px(HUD_MARGIN),
        top: px(HUD_MARGIN),
        padding: UiRect::all(px(SLOT_GAP)),
        column_gap: px(SLOT_GAP),
        align_items: AlignItems::Center,
        ..default()
    };
    commands
        .spawn((gems, BackgroundColor(PANEL_COLOR)))
        .with_children(|gems| {
            gems.spawn((
                ImageNode::new(assets.load(GEM_ICON)),
                icon_node(GEM_ICON_SIZE),
            ));
            gems.spawn((label(gem_count.to_string()), GemCount));
            let apart = Node {
                margin: UiRect::left(px(HUD_MARGIN)),
                ..default()
            };
            gems.spawn((label(level_label(level)), apart, LevelLabel));
            let back = Node {
                width: px(XP_BAR_SIZE.x),
                height: px(XP_BAR_SIZE.y),
                ..default()
            };
            gems.spawn((back, BackgroundColor(XP_BAR_BACK)))
                .with_child((
                    Node {
                        width: percent(100.0 * xp_fraction(level, xp)),
                        height: percent(100),
                        ..default()
                    },
                    BackgroundColor(XP_BAR_FILL),
                    XpFill,
                ));
        });

    // Bottom edge, centred: message, selected item's name, hotbar. Full width, so it survives any window size.
    let column = Node {
        position_type: PositionType::Absolute,
        left: px(0),
        right: px(0),
        bottom: px(HUD_MARGIN),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: px(SLOT_GAP),
        ..default()
    };
    commands.spawn(column).with_children(|column| {
        column.spawn((label(""), Toast::default()));
        column.spawn((label(""), SelectedName));
        column.spawn((row(SLOT_GAP), HotbarRow));
    });

    // Inventory and wardrobe: width from the window, height capped above the hotbar; only the
    // grids scroll (`fit_panels`, `scroll_grid`).
    let inventory = spawn_centre_panel(
        &mut commands,
        &assets,
        OpenPanel::Inventory,
        INVENTORY_GRID.width_for(0.0),
        |panel| {
            bag_tabs(panel, &assets, OpenPanel::Inventory);
            scroll_grid(panel, InventoryGrid);
        },
    );
    let wardrobe = spawn_centre_panel(
        &mut commands,
        &assets,
        OpenPanel::Wardrobe,
        WARDROBE_GRID.width_for(0.0),
        |panel| {
            bag_tabs(panel, &assets, OpenPanel::Wardrobe);
            // Preview and slot columns on the left, the owned equipment on the right, so the
            // grid keeps a useful height in a 720 px window.
            let body = Node {
                column_gap: px(WARDROBE_SPLIT),
                min_height: px(0),
                ..default()
            };
            let preview = Node {
                column_gap: px(WARDROBE_GAP),
                align_items: AlignItems::Center,
                align_self: AlignSelf::FlexStart,
                flex_shrink: 0.0,
                ..default()
            };
            let column = |index| {
                let node = Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(SLOT_GAP),
                    ..default()
                };
                (node, WornColumn(index))
            };
            let frame = Node {
                width: px(FRAME_SIZE.x),
                height: px(FRAME_SIZE.y),
                flex_shrink: 0.0,
                ..default()
            };
            let rig_size = PREVIEW_SIZE.as_vec2() * PREVIEW_SCALE;
            let rig = Node {
                position_type: PositionType::Absolute,
                left: px((FRAME_SIZE.x - rig_size.x) / 2.0),
                bottom: px((FRAME_BORDER + FEET_ABOVE_FRAME) * KIT_SCALE
                    - PREVIEW_BELOW_FEET as f32 * PREVIEW_SCALE),
                width: px(rig_size.x),
                height: px(rig_size.y),
                ..default()
            };
            let owned = Node {
                flex_direction: FlexDirection::Column,
                row_gap: px(HUD_MARGIN),
                flex_grow: 1.0,
                min_height: px(0),
                ..default()
            };
            panel.spawn(body).with_children(|body| {
                body.spawn(preview).with_children(|preview| {
                    preview.spawn(column(0));
                    preview
                        .spawn((frame, sliced(&assets, FRAME_IMAGE, FRAME_INSET)))
                        .with_children(|frame| {
                            frame.spawn((rig, ImageNode::new(preview_image.clone())));
                        });
                    preview.spawn(column(1));
                });
                body.spawn(owned).with_children(|owned| {
                    owned.spawn(ink("Your equipment: click to wear"));
                    owned.spawn((wrapping_grid(), FilterRow));
                    scroll_grid(owned, WardrobeGrid);
                });
            });
        },
    );
    commands
        .entity(inventory)
        .insert((INVENTORY_GRID, FitWindow { top: FIT_PANEL_TOP }));
    commands
        .entity(wardrobe)
        .insert((WARDROBE_GRID, FitWindow { top: FIT_PANEL_TOP }));
    let store = spawn_centre_panel(
        &mut commands,
        &assets,
        OpenPanel::Store,
        STORE_WIDTH,
        |panel| {
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(HUD_MARGIN),
                    min_height: px(0),
                    ..default()
                },
                StoreList,
            ));
        },
    );
    commands
        .entity(store)
        .insert(FitWindow { top: FIT_PANEL_TOP });

    // Top bar: top-right corner.
    let bar = Node {
        position_type: PositionType::Absolute,
        right: px(HUD_MARGIN),
        top: px(HUD_MARGIN),
        height: px(TOPBAR_HEIGHT),
        padding: UiRect::all(px(TOPBAR_PADDING)),
        column_gap: px(SLOT_GAP / 2.0),
        align_items: AlignItems::Center,
        ..default()
    };
    commands
        .spawn((bar, sliced(&assets, TOPBAR_IMAGE, TOPBAR_INSET)))
        .with_children(|bar| {
            for (button, file) in TOP_BUTTONS {
                bar.spawn((
                    Node {
                        width: px(TOP_BUTTON_SIZE),
                        height: px(TOP_BUTTON_SIZE),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    UiButton::Top(button),
                    Hovered::default(),
                ))
                .with_children(|cell| {
                    // The round frame behind the icon.
                    cell.spawn((
                        ImageNode::new(assets.load(TOP_BUTTON_IMAGE)),
                        Node {
                            position_type: PositionType::Absolute,
                            ..icon_node(TOP_BUTTON_ART)
                        },
                    ));
                    let mut icon =
                        cell.spawn((icon_node(TOP_ICON_SIZE), ImageNode::new(assets.load(file))));
                    if button == TopButton::Voice {
                        icon.insert(TopbarMic);
                    }
                });
            }
        });

    // Claim panel: top-right corner under the top bar, shown while a claim is inspected with the wrench.
    let claim = Node {
        position_type: PositionType::Absolute,
        right: px(HUD_MARGIN),
        top: px(TOPBAR_HEIGHT + 2.0 * HUD_MARGIN),
        padding: UiRect::all(px(PANEL_PADDING / 2.0)),
        flex_direction: FlexDirection::Column,
        row_gap: px(HUD_MARGIN),
        display: Display::None,
        ..default()
    };
    commands
        .spawn((
            claim,
            ClaimPanel,
            Hovered::default(),
            BackgroundColor(WINDOW_COLOR),
        ))
        .with_children(|panel| {
            panel.spawn((label(""), ClaimInfo));
            panel
                .spawn((
                    button_node(),
                    UiButton::ToggleClaimPublic,
                    Hovered::default(),
                    BackgroundColor(BUTTON_COLOR),
                ))
                .with_children(|button| {
                    button.spawn((label(""), ClaimPublicLabel));
                });
        });
}

pub(crate) fn button_node() -> Node {
    Node {
        min_height: px(SLOT_SIZE),
        padding: UiRect::all(px(HUD_MARGIN)),
        column_gap: px(HUD_MARGIN),
        align_items: AlignItems::Center,
        ..default()
    }
}

/// E/I opens the inventory or closes the inventory/wardrobe window, B the store; Escape closes.
fn toggle_panels(keys: Res<ButtonInput<KeyCode>>, mut open: ResMut<OpenPanel>) {
    let next = if keys.any_just_pressed(INVENTORY_KEYS) {
        match *open {
            OpenPanel::Inventory | OpenPanel::Wardrobe => OpenPanel::None,
            _ => OpenPanel::Inventory,
        }
    } else if keys.just_pressed(STORE_KEY) {
        if *open == OpenPanel::Store {
            OpenPanel::None
        } else {
            OpenPanel::Store
        }
    } else if keys.just_pressed(CLOSE_KEY) {
        OpenPanel::None
    } else {
        return;
    };
    if *open != next {
        *open = next;
    }
}

fn show_panels(open: Res<OpenPanel>, mut panels: Query<(&PanelOf, &mut Node)>) {
    if open.is_changed() {
        for (panel, mut node) in &mut panels {
            node.display = if panel.0 == *open {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

/// Keeps the centre panels inside the window: their height ends above the hotbar, and a
/// `GridPanel` gets as many columns as fit. Writes only on a change, so layout is not redone
/// every frame; a resize or fullscreen toggle applies on the next frame.
fn fit_panels(
    window: Single<&Window>,
    mut panels: Query<(&FitWindow, Option<&GridPanel>, &mut Node)>,
) {
    let size = window.size();
    for (fit, grid, mut node) in &mut panels {
        if node.top != px(fit.top) {
            node.top = px(fit.top);
        }
        let max_height = px(panel_max_height(size.y, fit.top));
        if node.max_height != max_height {
            node.max_height = max_height;
        }
        if let Some(width) = grid.map(|grid| grid.width_for(size.x))
            && node.width != px(width)
        {
            node.width = px(width);
            node.margin.left = px(-width / 2.0);
        }
    }
}

/// Indices (into `Inventory::slots`) of the non-equipment items, in inventory order. The hotbar
/// shows the first `HOTBAR_SLOTS` of them; the inventory grid shows them all.
fn carried_slots(me: &Player) -> Vec<usize> {
    let slots = me.inventory.slots().iter().enumerate();
    let carried = slots.filter(|(_, (id, _))| slot_of(*id).is_none());
    carried.map(|(index, _)| index).collect()
}

/// A left click on a HUD element, or a number key: select, switch panels, or ask the rules for
/// something.
#[allow(clippy::too_many_arguments)] // two input devices, four view states, two outputs
fn click_buttons(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    inspected: Res<InspectedClaim>,
    buttons: Query<(&UiButton, &Hovered)>,
    mut selected: ResMut<Selected>,
    mut wardrobe: ResMut<WardrobeSelected>,
    mut filter: ResMut<WardrobeFilter>,
    mut open: ResMut<OpenPanel>,
    mut tab: ResMut<StoreTab>,
    mut settings_tab: ResMut<SettingsTab>,
    mut actions: MessageWriter<ActionMsg>,
    mut chat: MessageWriter<OpenChat>,
    mut sfx: MessageWriter<Sfx>,
) {
    let by_key = SELECT_KEYS
        .iter()
        .position(|&key| keys.just_pressed(key))
        .and_then(|n| carried_slots(game.me()?).get(n).copied());
    let clicked = buttons
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(button, _)| *button)
        .filter(|_| mouse.just_pressed(MouseButton::Left));
    if clicked.is_some() {
        sfx.write(Sfx::UiClick);
    }
    let slot = match clicked {
        Some(UiButton::Slot(index)) => Some(index),
        Some(UiButton::Equip(item)) => {
            if wardrobe.0 != Some(item) {
                wardrobe.0 = Some(item);
            }
            actions.write(ActionMsg(Action::Equip { item }));
            None
        }
        Some(UiButton::Worn(slot)) => {
            actions.write(ActionMsg(Action::Unequip { slot }));
            None
        }
        Some(UiButton::Filter(slot)) => {
            if filter.0 != slot {
                filter.0 = slot;
            }
            None
        }
        Some(UiButton::Show(panel)) => {
            if *open != panel {
                *open = panel;
            }
            None
        }
        Some(UiButton::Close) => {
            *open = OpenPanel::None;
            None
        }
        Some(UiButton::Tab(clicked)) => {
            if *tab != clicked {
                *tab = clicked;
            }
            None
        }
        Some(UiButton::Buy(item)) => {
            actions.write(ActionMsg(Action::Buy { item }));
            None
        }
        Some(UiButton::BuyUpgrade) => {
            actions.write(ActionMsg(Action::BuyBackpackUpgrade));
            None
        }
        Some(UiButton::ToggleClaimPublic) => {
            let claim = inspected.0.and_then(|(x, y)| {
                let claim = game.0.world.claim(World::tile_key(x, y)?)?;
                Some((x, y, !claim.public))
            });
            if let Some((x, y, public)) = claim {
                actions.write(ActionMsg(Action::SetClaimPublic { x, y, public }));
            }
            None
        }
        Some(UiButton::Top(button)) => {
            let (panel, settings_page) = match button {
                TopButton::Store => (OpenPanel::Store, None),
                TopButton::Inventory => (OpenPanel::Inventory, None),
                TopButton::Wardrobe => (OpenPanel::Wardrobe, None),
                TopButton::Voice => (OpenPanel::Settings, Some(SettingsTab::Voice)),
                TopButton::Settings => (OpenPanel::Settings, None),
                TopButton::Chat => {
                    chat.write(OpenChat);
                    return;
                }
            };
            // A second click on the button of the window on show closes it.
            let on_show = *open == panel && settings_page.is_none_or(|page| *settings_tab == page);
            *open = if on_show { OpenPanel::None } else { panel };
            if let Some(page) = settings_page {
                settings_tab.set_if_neq(page);
            }
            None
        }
        None => None,
    };
    if let Some(index) = by_key.or(slot)
        && index != selected.0
    {
        selected.0 = index;
    }
}

/// One inventory slot: empty, or an item with its count. `index` is into `Inventory::slots`.
fn spawn_inventory_slot(
    parent: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    me: &Player,
    index: Option<usize>,
    selected: usize,
) {
    let file = if index == Some(selected) {
        SLOT_SELECTED_IMAGE
    } else {
        SLOT_IMAGE
    };
    let mut cell = parent.spawn(slot_cell(assets, file));
    let Some((index, &(id, count))) = index.and_then(|i| Some((i, me.inventory.slots().get(i)?)))
    else {
        return;
    };
    cell.insert(UiButton::Slot(index)).with_children(|cell| {
        if let Some(icon) = item_icon_node(assets, id) {
            cell.spawn(icon);
        }
        if let Some(count) = count_text(count) {
            cell.spawn(count);
        }
    });
}

/// Rebuilds the hotbar and the inventory grid from the player.
#[allow(clippy::too_many_arguments)] // two containers and the name line
fn refresh_slots(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    selected: Res<Selected>,
    mut events: MessageReader<GameEvent>,
    hotbar: Single<Entity, With<HotbarRow>>,
    grid: Single<Entity, With<InventoryGrid>>,
    mut name: Single<&mut Text, With<SelectedName>>,
) {
    let inventory_changed = events.read().any(|GameEvent(event)| match *event {
        Event::InventoryChanged { player, .. } | Event::BackpackUpgraded { player, .. } => {
            player == game.1
        }
        _ => false,
    });
    // `selected` also reads as changed on the first run, which fills the HUD once at startup.
    if !inventory_changed && !selected.is_changed() {
        return;
    }
    let Some(me) = game.me() else { return };
    let selected_name = selected
        .item(&game)
        .and_then(item)
        .map_or("", |def| def.name);
    if name.0 != selected_name {
        name.0 = selected_name.to_string();
    }
    let carried = carried_slots(me);
    commands
        .entity(*hotbar)
        .despawn_children()
        .with_children(|row| {
            for n in 0..HOTBAR_SLOTS {
                spawn_inventory_slot(row, &assets, me, carried.get(n).copied(), selected.0);
            }
        });
    // Free backpack slots are shown empty after the items.
    let free = usize::from(me.inventory.capacity).saturating_sub(me.inventory.slots().len());
    let cells = carried.iter().map(|&index| Some(index));
    let cells = cells.chain(std::iter::repeat_n(None, free));
    commands
        .entity(*grid)
        .despawn_children()
        .with_children(|grid| {
            for index in cells {
                spawn_inventory_slot(grid, &assets, me, index, selected.0);
            }
        });
}

/// Rebuilds the wardrobe's slot columns, its filter buttons and its equipment grid. The preview
/// image itself is `wardrobe.rs`'s.
#[allow(clippy::too_many_arguments)] // four containers, two view states and the first-fill flag
fn refresh_wardrobe(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    selected: Res<WardrobeSelected>,
    filter: Res<WardrobeFilter>,
    mut events: MessageReader<GameEvent>,
    columns: Query<(Entity, &WornColumn)>,
    filters: Single<Entity, With<FilterRow>>,
    grid: Single<Entity, With<WardrobeGrid>>,
    mut filled: Local<bool>,
) {
    let changed = events.read().any(|GameEvent(event)| match *event {
        Event::InventoryChanged { player, .. } | Event::Equipped { player, .. } => player == game.1,
        _ => false,
    });
    if !changed && !selected.is_changed() && !filter.is_changed() && *filled {
        return;
    }
    *filled = true;
    let Some(me) = game.me() else { return };
    for (entity, column) in &columns {
        let Some(slots) = WORN_COLUMNS.get(column.0) else {
            continue;
        };
        commands
            .entity(entity)
            .despawn_children()
            .with_children(|column| {
                for &slot in slots {
                    match me.worn(slot) {
                        Some(id) => {
                            column
                                .spawn((slot_cell(&assets, SLOT_WORN_IMAGE), UiButton::Worn(slot)))
                                .with_children(|cell| {
                                    if let Some(icon) = item_icon_node(&assets, id) {
                                        cell.spawn(icon);
                                    }
                                });
                        }
                        None => {
                            let silhouette = SLOT_ICON_NAMES[slot as usize];
                            column
                                .spawn(slot_cell(&assets, SLOT_EMPTY_IMAGE))
                                .with_child((
                                    ImageNode::new(
                                        assets.load(format!("ui/slot_icon_{silhouette}.png")),
                                    ),
                                    icon_node(ICON_SIZE),
                                ));
                        }
                    }
                }
            });
    }
    commands
        .entity(*filters)
        .despawn_children()
        .with_children(|row| {
            let choices = std::iter::once(None).chain(Slot::ALL.map(Some));
            for choice in choices {
                let file = if choice == filter.0 {
                    SLOT_SELECTED_IMAGE
                } else {
                    SLOT_IMAGE
                };
                let node = Node {
                    min_width: px(FILTER_SIZE),
                    height: px(FILTER_SIZE),
                    padding: UiRect::horizontal(px(HUD_MARGIN)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                };
                let mut button = row.spawn((
                    node,
                    sliced(&assets, file, SLOT_INSET),
                    UiButton::Filter(choice),
                    Hovered::default(),
                ));
                match choice {
                    None => button.with_child(label("All")),
                    Some(slot) => button.with_child((
                        ImageNode::new(assets.load(format!(
                            "ui/slot_icon_{}.png",
                            SLOT_ICON_NAMES[slot as usize]
                        ))),
                        icon_node(FILTER_ICON_SIZE),
                    )),
                };
            }
        });
    let owned = wardrobe_items(me.inventory.slots(), filter.0);
    commands
        .entity(*grid)
        .despawn_children()
        .with_children(|grid| {
            for (id, count) in owned {
                let worn = me.equipped.contains(&Some(id));
                let is_selected = selected.0 == Some(id);
                let file = match (is_selected, worn) {
                    (true, _) => SLOT_SELECTED_IMAGE,
                    (false, true) => SLOT_WORN_IMAGE,
                    (false, false) => SLOT_IMAGE,
                };
                grid.spawn((slot_cell(&assets, file), UiButton::Equip(id)))
                    .with_children(|cell| {
                        if let Some(icon) = item_icon_node(&assets, id) {
                            cell.spawn(icon);
                        }
                        if let Some(count) = count_text(count) {
                            cell.spawn(count);
                        }
                        // The selection rim hides the worn badge: say it in words.
                        if is_selected && worn {
                            cell.spawn((
                                label("worn"),
                                TextColor(WORN_TEXT_COLOR),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px(COUNT_INSET),
                                    top: px(COUNT_INSET / 2.0),
                                    ..default()
                                },
                            ));
                        }
                    });
            }
        });
}

/// The top bar's mic: on while voice chat is on and the player has not muted themself.
fn refresh_topbar_mic(
    assets: Res<AssetServer>,
    settings: Res<Settings>,
    mut icon: Single<&mut ImageNode, With<TopbarMic>>,
) {
    if settings.is_changed() {
        let on = cfg!(feature = "voice") && settings.voice && !settings.self_muted;
        icon.image = assets.load(if on { MIC_ON_ICON } else { MIC_MUTED_ICON });
    }
}

fn refresh_gems(
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut count: Single<&mut Text, With<GemCount>>,
) {
    for GameEvent(event) in events.read() {
        if let Event::GemsChanged { player, gems } = *event
            && player == game.1
        {
            count.0 = gems.to_string();
        }
    }
}

/// "Lv N", or "Lv MAX" at the cap.
fn level_label(level: u16) -> String {
    if level >= MAX_LEVEL {
        "Lv MAX".to_string()
    } else {
        format!("Lv {level}")
    }
}

/// How full the XP bar is, 0..=1; full at the cap.
fn xp_fraction(level: u16, xp: u32) -> f32 {
    match xp_to_next(level) {
        0 => 1.0,
        need => (xp as f32 / need as f32).min(1.0),
    }
}

/// The level and XP bar, on the local player's `XpChanged` / `LevelUp`.
fn refresh_level(
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut label: Single<&mut Text, With<LevelLabel>>,
    mut fill: Single<&mut Node, With<XpFill>>,
) {
    let mine = |event: &Event| match *event {
        Event::XpChanged { player, .. } | Event::LevelUp { player, .. } => player == game.1,
        _ => false,
    };
    // `filter().count()`, not `any()`: every message is read this frame.
    if events.read().filter(|GameEvent(event)| mine(event)).count() == 0 {
        return;
    }
    // The state already holds the values the events carried.
    let Some(me) = game.me() else {
        return;
    };
    label.0 = level_label(me.level);
    fill.width = percent(100.0 * xp_fraction(me.level, me.xp));
}

/// The price list, one tab at a time: the four claims and the backpack upgrade, or the equipment of
/// one slot as an icon grid (owned items show the worn badge). Entries the player cannot pay for
/// are greyed out; the rules still have the last word when one is clicked. The equipment grid
/// scrolls, and keeps its place when a gem or inventory change rebuilds it.
#[allow(clippy::too_many_arguments)] // a Bevy system: state, events, the list and its old grid
fn refresh_store(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    tab: Res<StoreTab>,
    mut events: MessageReader<GameEvent>,
    list: Single<Entity, With<StoreList>>,
    old_grid: Query<&ScrollPosition, With<StoreGrid>>,
    mut filled: Local<bool>,
) {
    let changed = events.read().any(|GameEvent(event)| {
        matches!(
            event,
            Event::GemsChanged { .. }
                | Event::BackpackUpgraded { .. }
                | Event::InventoryChanged { .. }
        )
    });
    if !changed && !tab.is_changed() && *filled {
        return;
    }
    *filled = true;
    let Some(me) = game.me() else { return };
    let scroll = old_grid
        .iter()
        .next()
        .filter(|_| !tab.is_changed())
        .map_or(Vec2::ZERO, |position| position.0);
    commands.entity(*list).despawn_children();
    let tabs = Node {
        flex_wrap: FlexWrap::Wrap,
        column_gap: px(SLOT_GAP),
        row_gap: px(SLOT_GAP),
        ..default()
    };
    commands.entity(*list).with_children(|list| {
        list.spawn(tabs).with_children(|tabs| {
            for (choice, name) in STORE_TABS {
                tabs.spawn(tab_bundle(&assets, choice, *tab))
                    .with_children(|button| {
                        button.spawn(label(name));
                    });
            }
        });
    });
    let StoreTab::Equip(slot) = *tab else {
        claims_and_backpack(&mut commands, &assets, me, *list);
        return;
    };
    let offered = ITEMS
        .iter()
        .filter(|def| def.kind == ItemKind::Equip(slot))
        .filter_map(|def| Some((def, equip_price(def.id)?)));
    let mut offered: Vec<_> = offered.collect();
    // Cheapest first; the table order (stable) within one price.
    offered.sort_by_key(|&(_, price)| price);
    commands.entity(*list).with_children(|list| {
        list.spawn(ink("Green badge: owned. Click to buy."));
        let grid = scroll_grid(list, (StoreGrid, ScrollPosition(scroll)));
        list.commands().entity(grid).with_children(|grid| {
            for (def, price) in offered {
                let affordable = me.gems >= price;
                let owned = me.inventory.count(def.id) > 0;
                let (text, tint) = if affordable {
                    (INK_COLOR, Color::WHITE)
                } else {
                    (DISABLED_TEXT_COLOR, DISABLED_ICON_COLOR)
                };
                let file = if owned { SLOT_WORN_IMAGE } else { SLOT_IMAGE };
                let cell = Node {
                    width: px(SLOT_SIZE),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                };
                grid.spawn((cell, UiButton::Buy(def.id), Hovered::default()))
                    .with_children(|cell| {
                        cell.spawn((slot_node(), sliced(&assets, file, SLOT_INSET)))
                            .with_children(|slot| {
                                let mut icon = item_icon(&assets, def.sprite).node();
                                icon.color = tint;
                                slot.spawn((icon, icon_node(ICON_SIZE)));
                            });
                        cell.spawn((label(price.to_string()), TextColor(text)));
                    });
            }
        });
    });
}

fn tab_bundle(assets: &AssetServer, choice: StoreTab, on_show: StoreTab) -> impl Bundle {
    tab(assets, UiButton::Tab(choice), choice == on_show)
}

/// The claims tab: the four claims and the backpack upgrade, one row each.
fn claims_and_backpack(commands: &mut Commands, assets: &AssetServer, me: &Player, list: Entity) {
    let upgrades = me.inventory.upgrades();
    let claims = CLAIMS.iter().filter_map(|kind| {
        let def = item(kind.item)?;
        let size = match kind.tiles {
            Some(tiles) => format!("{tiles} tiles"),
            None => "all".to_string(),
        };
        let text = format!("{} ({size})", def.name);
        Some((
            UiButton::Buy(kind.item),
            Some(def.sprite),
            text,
            Some(kind.price),
        ))
    });
    let upgrade = (
        UiButton::BuyUpgrade,
        None,
        format!(
            "backpack +{SLOTS_PER_UPGRADE} slots (now {})",
            me.inventory.capacity
        ),
        (upgrades < MAX_UPGRADES).then(|| upgrade_cost(upgrades)),
    );
    let entries: Vec<_> = claims.chain([upgrade]).collect();
    commands.entity(list).with_children(|list| {
        for (button, sprite, text, price) in entries {
            let affordable = price.is_some_and(|price| me.gems >= price);
            let colour = if affordable {
                Color::WHITE
            } else {
                DISABLED_TEXT_COLOR
            };
            let price = price.map_or("sold out".to_string(), |price| format!("{price} gems"));
            let row = Node {
                min_height: px(ROW_MIN_HEIGHT),
                padding: UiRect::axes(px(ROW_PADDING), px(HUD_MARGIN)),
                ..button_node()
            };
            list.spawn((
                row,
                sliced(assets, SLOT_IMAGE, SLOT_INSET),
                button,
                Hovered::default(),
            ))
            .with_children(|entry| {
                match sprite {
                    Some(sprite) => {
                        entry.spawn((item_icon(assets, sprite).node(), icon_node(ROW_ICON_SIZE)))
                    }
                    None => entry.spawn(icon_node(ROW_ICON_SIZE)),
                };
                let grow = Node {
                    flex_grow: 1.0,
                    ..default()
                };
                entry.spawn((label(text), TextColor(colour), grow));
                entry.spawn((label(price), TextColor(colour)));
            });
        }
    });
}

/// Owner, size and the public switch of the claim inspected with the wrench.
fn refresh_claim_panel(
    game: Res<Game>,
    inspected: Res<InspectedClaim>,
    mut events: MessageReader<GameEvent>,
    mut panel: Single<&mut Node, With<ClaimPanel>>,
    mut info: Single<&mut Text, (With<ClaimInfo>, Without<ClaimPublicLabel>)>,
    mut public: Single<&mut Text, (With<ClaimPublicLabel>, Without<ClaimInfo>)>,
) {
    let claim_changed = events.read().any(|GameEvent(event)| {
        matches!(
            event,
            Event::ClaimUpdated { .. } | Event::ClaimRemoved { .. }
        )
    });
    if !claim_changed && !inspected.is_changed() {
        return;
    }
    let claim = inspected
        .0
        .and_then(|(x, y)| game.0.world.claim(World::tile_key(x, y)?));
    let Some(claim) = claim else {
        panel.display = Display::None;
        return;
    };
    panel.display = Display::Flex;
    let size = if claim.tiles.is_empty() {
        "the whole world".to_string()
    } else {
        format!("{} tiles", claim.tiles.len())
    };
    info.0 = format!("Claim of player {}\n{size}", claim.owner.0);
    public.0 = format!("Public: {}", if claim.public { "on" } else { "off" });
}

fn item_name(id: ItemId) -> &'static str {
    item(id).map_or("?", |def| def.name)
}

/// What to tell the player about an event, if anything.
fn toast_for_event(event: &Event) -> Option<String> {
    match *event {
        Event::TreeSpliced { tree, a, b, .. } => Some(format!(
            "{} and {} spliced into {}",
            item_name(a),
            item_name(b),
            item_name(tree.item)
        )),
        Event::BackpackUpgraded { capacity, .. } => {
            Some(format!("Backpack now holds {capacity} item types"))
        }
        _ => None,
    }
}

/// The player must always learn why nothing happened. Cooldown and plain misses stay silent.
fn toast_for_reject(reject: Reject) -> Option<&'static str> {
    match reject {
        Reject::NoPermission => Some("Locked: this area belongs to someone else"),
        Reject::NotOwner => Some("Only the owner of this claim can do that"),
        Reject::NoRecipe => Some("These two seeds cannot be spliced"),
        Reject::AlreadySpliced => Some("This tree has already been spliced"),
        Reject::NeedsGround => Some("Seeds need solid ground below"),
        Reject::DropLimit => Some("This world cannot hold more dropped items"),
        Reject::NotEnoughGems => Some("Not enough gems"),
        Reject::InventoryFull => Some("No room in the backpack"),
        Reject::WorldAlreadyClaimed => Some("This world is already claimed"),
        Reject::ForeignClaims => Some("Someone else has a claim in this world"),
        Reject::OutOfRange
        | Reject::Occupied
        | Reject::NoItem
        | Reject::Unbreakable
        | Reject::Cooldown
        | Reject::Invalid => None,
    }
}

fn show_toasts(
    time: Res<Time>,
    mut events: MessageReader<GameEvent>,
    mut results: MessageReader<ActionResult>,
    mut notices: MessageReader<Notice>,
    toast: Single<(&mut Text, &mut Toast)>,
) {
    let (mut text, mut toast) = toast.into_inner();
    let from_events = events
        .read()
        .filter_map(|GameEvent(event)| toast_for_event(event));
    let from_rejects = results
        .read()
        .filter_map(|result| result.result.as_ref().err().copied())
        .filter_map(toast_for_reject)
        .map(String::from);
    let from_notices = notices.read().map(|Notice(text)| text.clone());
    if let Some(message) = from_events.chain(from_rejects).chain(from_notices).last() {
        text.0 = message;
        toast.until = time.elapsed_secs() + TOAST_SECS;
    } else if !text.0.is_empty() && time.elapsed_secs() >= toast.until {
        text.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::farming::Tree;
    use game_shared::items::{DIRT, GRAVEL, STONE};

    #[test]
    fn splice_message_names_both_parents_and_the_result() {
        let tree = Tree {
            item: GRAVEL,
            planted_at: 0.0,
            fruit: 3,
            spliced: true,
        };
        let spliced = Event::TreeSpliced {
            x: 1,
            y: 1,
            tree,
            a: DIRT,
            b: STONE,
        };
        assert_eq!(
            toast_for_event(&spliced).as_deref(),
            Some("dirt and stone spliced into gravel")
        );
        assert_eq!(toast_for_event(&Event::TreeHarvested { x: 1, y: 1 }), None);
    }

    #[test]
    fn the_xp_bar_fills_with_progress_and_is_full_at_the_cap() {
        assert_eq!(xp_fraction(1, 0), 0.0);
        assert_eq!(xp_fraction(1, 50), 0.5);
        assert_eq!(xp_fraction(2, 200), 1.0, "clamped");
        assert_eq!(xp_fraction(MAX_LEVEL, 0), 1.0);
        assert_eq!(level_label(7), "Lv 7");
        assert_eq!(level_label(MAX_LEVEL), "Lv MAX");
    }

    #[test]
    fn a_panel_ends_above_the_hotbar_but_keeps_a_minimum_height() {
        assert_eq!(HOTBAR_RESERVE, 138.0);
        assert_eq!(panel_max_height(720.0, PANEL_TOP), 720.0 - 48.0 - 138.0);
        assert_eq!(panel_max_height(1080.0, 72.0), 1080.0 - 72.0 - 138.0);
        assert_eq!(panel_max_height(300.0, PANEL_TOP), MIN_PANEL_HEIGHT);
    }

    #[test]
    fn grid_columns_follow_the_width() {
        // A slot is 74 px with a 4 px gap: n columns need 78n - 4 px.
        assert_eq!(columns_for_width(grid_width(7.0), 3, 10), 7);
        assert_eq!(columns_for_width(grid_width(7.0) - 1.0, 3, 10), 6);
        assert_eq!(columns_for_width(-50.0, 3, 10), 3);
        assert_eq!(columns_for_width(5000.0, 3, 10), 10);
        // The panel is exactly as wide as its grid, scrollbar and padding, and fits the window.
        let width = WARDROBE_GRID.width_for(1280.0);
        assert!(width <= 1280.0 - 2.0 * HUD_MARGIN, "{width}");
        assert_eq!(
            INVENTORY_GRID.width_for(0.0),
            grid_width(6.0) + SCROLLBAR_SPACE + 64.0
        );
        assert_eq!(
            INVENTORY_GRID.width_for(1280.0),
            grid_width(10.0) + SCROLLBAR_SPACE + 64.0
        );
    }

    #[test]
    fn the_wardrobe_filter_keeps_one_slot_or_all_equipment() {
        let hat = ITEMS
            .iter()
            .find(|d| d.kind == ItemKind::Equip(Slot::Hat))
            .unwrap()
            .id;
        let hair = ITEMS
            .iter()
            .find(|d| d.kind == ItemKind::Equip(Slot::Hair))
            .unwrap()
            .id;
        let slots = [(DIRT, 5), (hat, 1), (hair, 2)];
        assert_eq!(wardrobe_items(&slots, None), vec![(hat, 1), (hair, 2)]);
        assert_eq!(wardrobe_items(&slots, Some(Slot::Hair)), vec![(hair, 2)]);
        assert_eq!(wardrobe_items(&slots, Some(Slot::Back)), vec![]);
    }

    /// `pointer_over_ui` only sees nodes with `Hovered`: the wheel over a scrolling grid must
    /// not reach the world or the camera.
    #[test]
    fn every_part_of_a_scroll_grid_counts_as_ui() {
        use bevy::ecs::world::CommandQueue;
        let mut world = bevy::ecs::world::World::new();
        let root = world.spawn_empty().id();
        let mut queue = CommandQueue::default();
        let mut grid = Entity::PLACEHOLDER;
        Commands::new(&mut queue, &world)
            .entity(root)
            .with_children(|parent| grid = scroll_grid(parent, InventoryGrid));
        queue.apply(&mut world);
        let area = world.get::<ChildOf>(grid).expect("in the area").parent();
        let mut parts = vec![area];
        let mut index = 0;
        while let Some(&entity) = parts.get(index) {
            parts.extend(world.get::<Children>(entity).into_iter().flatten());
            index += 1;
        }
        assert_eq!(parts.len(), 4, "area, grid, track, thumb");
        for entity in parts {
            assert!(world.get::<Hovered>(entity).is_some(), "{entity:?}");
        }
        assert!(world.get::<ScrollArea>(grid).is_some());
    }

    #[test]
    fn rejects_the_player_cannot_guess_get_a_message() {
        for reject in [
            Reject::NoPermission,
            Reject::NotOwner,
            Reject::NoRecipe,
            Reject::AlreadySpliced,
            Reject::NeedsGround,
            Reject::NotEnoughGems,
            Reject::InventoryFull,
            Reject::WorldAlreadyClaimed,
            Reject::ForeignClaims,
            Reject::DropLimit,
        ] {
            assert!(toast_for_reject(reject).is_some(), "{reject:?}");
        }
        assert_eq!(toast_for_reject(Reject::Cooldown), None);
    }
}
