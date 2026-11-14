//! Hotbar, inventory, wardrobe, store, claim panel, gem counter and messages. Everything here
//! changes when a `GameEvent` or `ActionResult` arrives or the player opens/selects something,
//! never by polling the state every frame. The HUD decides nothing: buying, equipping and toggling
//! send an `Action`. Panels use the wood-and-parchment kit in assets/ui/ (assets/README.md, "UI").

use bevy::input::touch::Touches;
use bevy::picking::Pickable;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::{UiGlobalTransform, VisualBox};
use bevy::ui_widgets::{ControlOrientation, ScrollArea, Scrollbar, ScrollbarThumb};
use bevy::window::PrimaryWindow;
use game_shared::claims::{CLAIMS, claim_kind};
use game_shared::crafting::recipe;
use game_shared::farming::grow_seconds;
use game_shared::inventory::{MAX_UPGRADES, SLOTS_PER_UPGRADE, upgrade_cost};
use game_shared::items::{
    FIST, ITEMS, ItemDef, ItemId, ItemKind, LADDER, Slot, WORKBENCH, WRENCH, equip_price, item,
    seed_of, slot_of,
};
use game_shared::levels::{MAX_LEVEL, xp_to_next};
use game_shared::mods::{FIST_DAMAGE, Mod, mods_of};
use game_shared::rules::{Action, Event, Player, Reject};

use crate::audio::Sfx;
use crate::chat::OpenChat;
use crate::screenshot;
use crate::settings::{Settings, SettingsTab, VOICE_KEY};
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
/// chat.rs opens the chat line on Enter and has no constant of its own; named here for the
/// top bar's label only.
const CHAT_KEY: KeyCode = KeyCode::Enter;
/// Opens and closes the craft panel from anywhere (docs/knowledge/ux/workbench.md); Y on a gamepad.
pub(crate) const CRAFT_PANEL_KEY: KeyCode = KeyCode::KeyC;
const CRAFT_PANEL_PAD: GamepadButton = GamepadButton::North;

// The UI kit (assets/ui/). Sizes and insets are in the art's own pixels; it is drawn at
// `KIT_SCALE` logical px per art px.
pub(crate) const KIT_SCALE: f32 = 2.0;
pub(crate) const PANEL_IMAGE: &str = "ui/panel.png";
pub(crate) const PANEL_INSET: f32 = 18.0;
pub(crate) const SLOT_IMAGE: &str = "ui/slot.png";
pub(crate) const SLOT_SELECTED_IMAGE: &str = "ui/slot_selected.png";
const SLOT_WORN_IMAGE: &str = "ui/slot_worn.png";
const SLOT_EMPTY_IMAGE: &str = "ui/slot_equip_empty.png";
const SLOT_INSET: f32 = 8.0;
const TAB_IMAGE: &str = "ui/tab.png";
const TAB_INSET: f32 = 8.0;
const FRAME_IMAGE: &str = "ui/preview_frame.png";
const FRAME_INSET: f32 = 20.0;
pub(crate) const CLOSE_IMAGE: &str = "ui/close.png";
/// `ui/slot_icon_<name>.png`, in `Slot::ALL` order.
const SLOT_ICON_NAMES: [&str; 8] = [
    "hair", "hat", "face", "shirt", "pants", "shoes", "back", "hand",
];

// Px (logical). Slots are touch targets: at least 44.
pub(crate) const SLOT_SIZE: f32 = 37.0 * KIT_SCALE;
const _: () = assert!(SLOT_SIZE >= 44.0);
pub(crate) const SLOT_GAP: f32 = 4.0;
pub(crate) const ICON_SIZE: f32 = 48.0;
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
pub(crate) const PANEL_PADDING: f32 = 32.0;
const PANEL_TOP: f32 = 48.0;
const STORE_COLUMNS: f32 = 8.0;
pub(crate) const fn grid_width(columns: f32) -> f32 {
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
pub(crate) const FIT_PANEL_TOP: f32 = HUD_MARGIN + TOPBAR_HEIGHT + CLOSE_OVERHANG + HUD_MARGIN;
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
pub(crate) const FONT: &str = "fonts/growsociaety_pixel.ttf";
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
pub(crate) const TOPBAR_HEIGHT: f32 = TOP_BUTTON_SIZE + 2.0 * TOPBAR_PADDING;
/// The almanac's key, for its top-bar label only (J / F1 are read in `almanac.rs`).
const MIC_ON_ICON: &str = "ui/voice_mic_on.png";
const MIC_MUTED_ICON: &str = "ui/voice_mic_muted.png";
/// Top-bar buttons, left to right, with their icons.
const TOP_BUTTONS: [(TopButton, &str); 9] = [
    (TopButton::Store, "ui/settings/icon_shop.png"),
    (TopButton::Inventory, "ui/settings/icon_inventory.png"),
    (TopButton::Wardrobe, "ui/settings/icon_wardrobe.png"),
    (TopButton::Craft, CRAFT_ICON),
    (TopButton::Chat, "ui/settings/icon_chat.png"),
    (TopButton::Almanac, "ui/settings/icon_help.png"),
    (TopButton::World, "ui/settings/icon_world.png"),
    (TopButton::Voice, MIC_MUTED_ICON),
    (TopButton::Settings, "ui/settings_gear.png"),
];
const GEM_ICON_SIZE: f32 = 24.0;
/// The XP bar beside the level, under the top-left counters.
const XP_BAR_SIZE: Vec2 = Vec2::new(64.0, 6.0);
const XP_BAR_BACK: Color = Color::srgb(0.2, 0.2, 0.24);
const XP_BAR_FILL: Color = Color::srgb(0.45, 0.85, 1.0);

pub(crate) const PANEL_COLOR: Color = Color::srgba(0.08, 0.08, 0.1, 0.7);
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
const BAG_TABS: [(OpenPanel, &str); 3] = [
    (OpenPanel::Inventory, "Inventory"),
    (OpenPanel::Wardrobe, "Wardrobe"),
    (OpenPanel::Craft, "Craft"),
];
/// On the Craft tab, before its label.
const CRAFT_ICON: &str = "ui/icon_craft.png";
const TAB_ICON_SIZE: f32 = 32.0;
/// Mod icons beside an item's name: the 32 px art at half size, one text line high.
const MOD_ICON_SIZE: f32 = 16.0;
/// Wardrobe slot columns, left and right of the preview.
const WORN_COLUMNS: [[Slot; 4]; 2] = [
    [Slot::Hair, Slot::Hat, Slot::Face, Slot::Shirt],
    [Slot::Pants, Slot::Shoes, Slot::Back, Slot::Hand],
];

/// Px the pointer must move with the button held on an inventory slot before the press becomes a
/// drag; less is a click (select).
const DRAG_THRESHOLD: f32 = 6.0;
/// Alpha of the dragged item's icon under the pointer.
const DRAG_GHOST_ALPHA: f32 = 0.6;

/// Seconds a message stays on screen.
const TOAST_SECS: f32 = 3.0;
/// The same refusal text is not shown again within this many seconds (holding the mouse on
/// bedrock must not flicker).
const REJECT_REPEAT_SECS: f32 = 2.0;
/// Rejects that stay silent on purpose (docs/knowledge/ux/refusals-and-big-buys.md, U2): the
/// world already shows them, or they fire constantly. No toast and no tile flash.
const SILENT_REJECTS: [Reject; 3] = [Reject::Occupied, Reject::OutOfRange, Reject::Cooldown];

/// Purchases at this price or more take a second click (U7). Same idea as the trash key's
/// press-twice (`interact::TRASH_CONFIRM_SECS`), with longer to read the price.
const CONFIRM_PRICE: u32 = 1000;
const CONFIRM_SECS: f32 = 3.0;
/// The armed price of a wide claim row: "Buy for <gem> <price>?".
const BUY_CONFIRM_TEXT: &str = "Buy for";
/// The armed narrow equipment cell's price line; the strip above the hotbar says the rest:
/// "Click again to buy for <gem> <price>".
const BUY_SHORT_TEXT: &str = "Buy?";
const STRIP_ARMED_TEXT: &str = "Click again to buy for";
/// Between the parts of the selection strip.
const STRIP_SEP: &str = "·";
/// Store focus: arrows / d-pad move it, Enter / South clicks, East disarms then closes.
const STORE_NAV_KEYS: [KeyCode; 4] = [
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
];
const STORE_ACTIVATE_KEY: KeyCode = KeyCode::Enter;
/// Previous / next tab in a tabbed panel (store categories, Inventory / Wardrobe / Craft),
/// wrapping around; LB / RB on a gamepad. Their hints sit at the ends of each tab bar.
const PREV_TAB_KEY: KeyCode = KeyCode::BracketLeft;
const NEXT_TAB_KEY: KeyCode = KeyCode::BracketRight;
const PREV_TAB_HINT: &str = "[";
const NEXT_TAB_HINT: &str = "]";
const BUY_IMAGE: &str = "ui/lobby/button_primary.png";
const BUY_HOVER_IMAGE: &str = "ui/lobby/button_primary_hover.png";
const BUY_INSET: f32 = 10.0;
/// Touch targets.
const TOUCH_MIN: f32 = 44.0;

/// The item tooltip (U3): `ui/tooltip.png` 9-sliced, shown after the pointer rests on a cell.
pub(crate) const TOOLTIP_IMAGE: &str = "ui/tooltip.png";
pub(crate) const TOOLTIP_INSET: f32 = 6.0;
pub(crate) const TOOLTIP_MAX_WIDTH: f32 = 280.0;
pub(crate) const TOOLTIP_DELAY_SECS: f32 = 0.3;
/// A store price the player cannot pay, on the tooltip's parchment.
const SHORT_PRICE_COLOR: Color = Color::srgb(0.7, 0.1, 0.08);
/// Between the parts of a line: "Block · 6 punches".
pub(crate) const SEP: &str = " · ";

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

/// True while a HUD text field has the keyboard (the claim panel's name field), so other keys
/// (the chat line's Enter) leave it alone. Written by the panel that owns the field.
#[derive(Resource, Default, PartialEq)]
pub struct TextFocus(pub bool);

/// An item cell: hovering it shows the shared tooltip, and it drives the selection strip. The
/// stack shown, if any (count as in the cell).
#[derive(Component, Clone, Copy, PartialEq, Default)]
pub(crate) struct ItemTip(pub Option<(ItemId, u16)>);

/// The store entry armed by a first click on an expensive item, and when it disarms.
#[derive(Resource, Default, PartialEq)]
struct ArmedBuy(Option<(UiButton, f32)>);

/// The store entry with the keyboard / gamepad focus: an index into `store_entries`. Back to the
/// first one when the store opens or the tab changes.
#[derive(Resource, Default)]
struct StoreFocus(usize);

/// Tab keys: -1 previous, +1 next, taken while a tabbed panel is open.
#[derive(Resource, Default)]
pub(crate) struct TabKeys(i32);

/// Store keys, read before the player's movement sees them.
#[derive(Resource, Default)]
struct StoreKeys {
    step: Option<(i32, i32)>,
    activate: bool,
    back: bool,
}

/// The armed price cell's button art (hover swaps it).
#[derive(Component)]
struct ArmedArt;

/// The shared tooltip box.
#[derive(Component)]
struct Tooltip;

/// The equipment item selected in the wardrobe grid.
#[derive(Resource, Default)]
pub struct WardrobeSelected(pub Option<ItemId>);

/// The slot the wardrobe grid is narrowed to; `None` shows all equipment.
#[derive(Resource, Default)]
struct WardrobeFilter(Option<Slot>);

/// Dragging an item between the hotbar and the inventory grid (`drag_items`). `from` is an index
/// into `Inventory::slots`.
#[derive(Resource, Default, Clone, Copy, PartialEq, Debug)]
pub enum ItemDrag {
    #[default]
    Idle,
    /// The button went down on a slot at `at` (window px); not yet a drag.
    Pressed {
        from: usize,
        at: Vec2,
    },
    Dragging {
        from: usize,
    },
}

impl ItemDrag {
    /// True from the press on a slot until the release: the world gets no punch or place.
    pub fn busy(&self) -> bool {
        *self != ItemDrag::Idle
    }
}

/// A cell of the hotbar or the inventory grid: its place in the view of non-equipment items
/// (`carried_slots`), filled or empty. Hotbar cell n and grid cell n are the same place.
#[derive(Component)]
struct CarriedCell(usize);

/// The dragged item's icon, following the pointer.
#[derive(Component)]
struct DragGhost;

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
    /// The craft panel (`craft_ui.rs`).
    Craft,
    /// The claim panel of the claim inspected with the wrench (`claim_ui.rs`).
    Claim,
    /// The almanac (`almanac.rs`).
    Almanac,
    /// The World menu (`world_select.rs`).
    WorldSelect,
    /// The menu beside a clicked player (`avatar_menu.rs`).
    AvatarMenu,
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
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
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
    Top(TopButton),
}

/// A top-bar button: opens (or closes) its panel; Chat opens the chat line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TopButton {
    Store,
    Inventory,
    Wardrobe,
    Craft,
    Chat,
    /// The almanac (`almanac.rs`).
    Almanac,
    /// The World menu (`world_select.rs`).
    World,
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
/// Name of the selected item. Seeds share one sprite, so the name is the reliable way to tell them apart.
#[derive(Component)]
struct SelectedName;
/// The mods of the item named in `SelectedName`.
#[derive(Component)]
struct ModLine;
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
            Some("craft") => OpenPanel::Craft,
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
            .init_resource::<ItemDrag>()
            .init_resource::<TextFocus>()
            .init_resource::<ArmedBuy>()
            .init_resource::<StoreFocus>()
            .init_resource::<StoreKeys>()
            .init_resource::<TabKeys>()
            // After `PreUpdate` (the chat line clears keys there), before the player's
            // `FixedUpdate` would walk on the arrows.
            .add_systems(
                RunFixedMainLoop,
                (take_store_keys, take_tab_keys)
                    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .insert_resource(open)
            .insert_resource(tab)
            .add_message::<Notice>()
            .add_systems(Startup, spawn_hud)
            .add_systems(
                Update,
                (
                    toggle_panels,
                    click_buttons,
                    use_tab_keys,
                    use_store_keys,
                    drag_items,
                )
                    .chain()
                    .in_set(Phase::Input),
            )
            .add_systems(
                Update,
                (
                    show_panels,
                    fit_panels,
                    (follow_moved_selection, refresh_slots).chain(),
                    refresh_item_info,
                    refresh_wardrobe,
                    refresh_gems,
                    refresh_level,
                    refresh_store,
                    refresh_topbar_mic,
                    show_toasts,
                    show_tooltip,
                    skin_armed,
                    ladder_tip,
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

pub(crate) fn icon_node(size: f32) -> Node {
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
pub(crate) fn slot_cell(assets: &AssetServer, file: &'static str) -> impl Bundle {
    (
        slot_node(),
        sliced(assets, file, SLOT_INSET),
        Hovered::default(),
    )
}

pub(crate) fn item_icon_node(assets: &AssetServer, id: ItemId) -> Option<impl Bundle> {
    let def = item(id)?;
    Some((item_icon(assets, def.sprite).node(), icon_node(ICON_SIZE)))
}

/// The count in a slot's bottom-right corner, when there is more than one.
pub(crate) fn count_text(count: u16) -> Option<impl Bundle> {
    (count > 1).then(|| (label(count.to_string()), count_node()))
}

/// An empty count text for a slot whose count is rewritten in place.
pub(crate) fn count_label() -> impl Bundle {
    (label(""), count_node())
}

fn count_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        right: px(COUNT_INSET),
        bottom: px(COUNT_INSET / 2.0),
        ..default()
    }
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
pub(crate) fn bag_tabs(parent: &mut ChildSpawnerCommands, assets: &AssetServer, active: OpenPanel) {
    let bar = Node {
        align_items: AlignItems::Center,
        ..row(SLOT_GAP)
    };
    parent
        .spawn((bar, Hovered::default()))
        .with_children(|tabs| {
            tabs.spawn((ink(PREV_TAB_HINT), Hovered::default()));
            for (panel, name) in BAG_TABS {
                tabs.spawn(tab(assets, UiButton::Show(panel), panel == active))
                    .with_children(|button| {
                        if panel == OpenPanel::Craft {
                            button.spawn((
                                ImageNode::new(assets.load(CRAFT_ICON)),
                                icon_node(TAB_ICON_SIZE),
                            ));
                        }
                        button.spawn(label(name));
                    });
            }
            tabs.spawn((ink(NEXT_TAB_HINT), Hovered::default()));
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
        column
            .spawn(Node {
                column_gap: px(HUD_MARGIN),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|line| {
                line.spawn((label(""), SelectedName));
                line.spawn((row(HUD_MARGIN), ModLine));
                crate::inventory_filter::spawn_strip_buttons(line, &assets);
            });
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
            crate::inventory_filter::spawn_filter_bar(panel, &assets);
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

    // Above everything and invisible to picking, so the slot under it stays the hovered one.
    commands.spawn((
        DragGhost,
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            ..icon_node(ICON_SIZE)
        },
        ImageNode::default(),
        GlobalZIndex(i32::MAX),
        Pickable::IGNORE,
    ));

    // The shared tooltip: on top of the panels and invisible to picking, so the cell under the
    // pointer stays the hovered one (`show_tooltip`).
    commands.spawn((
        Tooltip,
        Node {
            position_type: PositionType::Absolute,
            max_width: px(TOOLTIP_MAX_WIDTH),
            padding: UiRect::all(px(TOOLTIP_INSET * KIT_SCALE)),
            flex_direction: FlexDirection::Column,
            row_gap: px(SLOT_GAP),
            display: Display::None,
            ..default()
        },
        sliced(&assets, TOOLTIP_IMAGE, TOOLTIP_INSET),
        GlobalZIndex(i32::MAX - 1),
        tip_part(),
    ));
}

/// Every node of the tooltip: never under the pointer, but counted by `pointer_over_ui` alike.
pub(crate) fn tip_part() -> (Pickable, Hovered) {
    (Pickable::IGNORE, Hovered::default())
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

/// E/I opens the inventory or closes the inventory/wardrobe window, B the store, C (gamepad Y)
/// the craft panel; Escape closes (an armed store purchase is disarmed first). A focused text field
/// has already taken the keys (`claim_ui::take_panel_keys`), so typing a C never toggles.
fn toggle_panels(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut open: ResMut<OpenPanel>,
    mut armed: ResMut<ArmedBuy>,
) {
    if keys.just_pressed(CLOSE_KEY) && armed.0.is_some() {
        armed.0 = None;
        return;
    }
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
    } else if keys.just_pressed(CRAFT_PANEL_KEY)
        || pads.iter().any(|pad| pad.just_pressed(CRAFT_PANEL_PAD))
    {
        if *open == OpenPanel::Craft {
            OpenPanel::None
        } else {
            OpenPanel::Craft
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
#[allow(clippy::too_many_arguments)] // two input devices, view states, outputs
fn click_buttons(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    game: Res<Game>,
    buttons: Query<(&UiButton, &Hovered)>,
    mut selected: ResMut<Selected>,
    mut wardrobe: ResMut<WardrobeSelected>,
    mut filter: ResMut<WardrobeFilter>,
    mut open: ResMut<OpenPanel>,
    mut tab: ResMut<StoreTab>,
    mut settings_tab: ResMut<SettingsTab>,
    mut armed: ResMut<ArmedBuy>,
    mut actions: MessageWriter<ActionMsg>,
    mut chat: MessageWriter<OpenChat>,
    mut sfx: MessageWriter<Sfx>,
) {
    let now = time.elapsed_secs();
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
    // Disarmed by time, by the store closing, or by a click anywhere but the armed entry.
    let disarm = armed.0.is_some_and(|(button, until)| {
        now > until
            || *open != OpenPanel::Store
            || (mouse.just_pressed(MouseButton::Left) && clicked != Some(button))
    });
    if disarm {
        armed.0 = None;
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
        Some(button @ (UiButton::Buy(_) | UiButton::BuyUpgrade)) => {
            if let Some(me) = game.me() {
                buy_or_arm(button, me, now, &mut armed, &mut actions);
            }
            None
        }
        Some(UiButton::Top(button)) => {
            let (panel, settings_page) = match button {
                TopButton::Store => (OpenPanel::Store, None),
                TopButton::Inventory => (OpenPanel::Inventory, None),
                TopButton::Wardrobe => (OpenPanel::Wardrobe, None),
                TopButton::Craft => (OpenPanel::Craft, None),
                TopButton::Almanac => (OpenPanel::Almanac, None),
                TopButton::World => (OpenPanel::WorldSelect, None),
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

/// The raw slot (`Inventory::slots` index) to send as `MoveItem::to` when the item in raw slot
/// `from` is dropped on view place `position` (hotbar or grid cell, `CarriedCell`). `carried` is
/// `carried_slots`, `len` the number of slots. On a filled place: that item's raw slot, so the two
/// swap (`Inventory::move_slot`) and the dragged item shows at `position`. On an empty place: `len`,
/// past the end, which moves the item to the end of the inventory, making it the last
/// non-equipment item shown (equipment after it is never shown). `None` when nothing would change.
fn move_target(carried: &[usize], len: usize, from: usize, position: usize) -> Option<u16> {
    let to = match carried.get(position) {
        Some(&to) => to,
        None if carried.last() == Some(&from) => return None,
        None => len,
    };
    (to != from).then(|| u16::try_from(to).ok()).flatten()
}

/// Whether a press at `at` has moved far enough to `cursor` to be a drag.
fn drag_moved(at: Vec2, cursor: Vec2) -> bool {
    at.distance(cursor) >= DRAG_THRESHOLD
}

/// What a release does: a move only for a real drag over an inventory place (`position`); a press
/// that never moved was a click, and a release anywhere else cancels.
fn drop_action(
    drag: ItemDrag,
    position: Option<usize>,
    carried: &[usize],
    len: usize,
) -> Option<Action> {
    let ItemDrag::Dragging { from } = drag else {
        return None;
    };
    let to = move_target(carried, len, from, position?)?;
    Some(Action::MoveItem {
        from: u16::try_from(from).ok()?,
        to,
    })
}

/// Where slot `index` is after `Inventory::move_slot(from, to)` on `len` slots.
fn index_after_move(index: usize, from: usize, to: usize, len: usize) -> usize {
    match (to < len, index) {
        (true, i) if i == from => to,
        (true, i) if i == to => from,
        (false, i) if i == from => len.saturating_sub(1),
        (false, i) if i > from => i - 1,
        (_, i) => i,
    }
}

/// The selection follows its item when the rules move it.
fn follow_moved_selection(
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut selected: ResMut<Selected>,
) {
    let Some(len) = game.me().map(|me| me.inventory.slots().len()) else {
        return;
    };
    for GameEvent(event) in events.read() {
        if let Event::InventoryMoved { player, from, to } = *event
            && player == game.1
        {
            let index = index_after_move(selected.0, from.into(), to.into(), len);
            if selected.0 != index {
                selected.0 = index;
            }
        }
    }
}

/// Press on a filled hotbar or inventory cell and move: the item's icon follows the pointer;
/// releasing it on another cell asks the rules to move it there (`MoveItem`). The slots are redrawn
/// from the `InventoryMoved` that comes back, never here. The press itself still selects
/// (`click_buttons`). A panel opening or closing cancels the drag.
#[allow(clippy::too_many_arguments)] // input, cells, state, the ghost and the action output
fn drag_items(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    game: Res<Game>,
    open: Res<OpenPanel>,
    cells: Query<(&CarriedCell, &Hovered, Option<&UiButton>)>,
    ui: Query<&Hovered>,
    local: Single<&crate::player::Player>,
    mut drag: ResMut<ItemDrag>,
    ghost: Single<(Entity, &mut Node), With<DragGhost>>,
    mut actions: MessageWriter<ActionMsg>,
) {
    let (ghost, mut ghost_node) = ghost.into_inner();
    let cursor = window.cursor_position();
    let hovered = cells.iter().find(|(_, hovered, _)| hovered.get());
    let next = match *drag {
        ItemDrag::Idle => match (hovered, cursor) {
            (Some((_, _, Some(&UiButton::Slot(from)))), Some(at))
                if mouse.just_pressed(MouseButton::Left) =>
            {
                ItemDrag::Pressed { from, at }
            }
            _ => ItemDrag::Idle,
        },
        _ if open.is_changed() => ItemDrag::Idle,
        current if !mouse.pressed(MouseButton::Left) => {
            if let Some(me) = game.me() {
                let position = hovered.map(|(cell, _, _)| cell.0);
                let len = me.inventory.slots().len();
                if let Some(action) = drop_action(current, position, &carried_slots(me), len) {
                    actions.write(ActionMsg(action));
                } else if let ItemDrag::Dragging { from } = current
                    && *open == OpenPanel::Inventory
                    && crate::interact::drags_out(position.is_some(), pointer_over_ui(&ui))
                    && let Some(&(id, held)) = me.inventory.slots().get(from)
                    && crate::interact::discardable(id)
                {
                    // Carried out of the panel onto the world: the whole stack drops.
                    actions.write(ActionMsg(Action::Drop {
                        item: id,
                        count: crate::interact::stack_count(held),
                        left: local.facing_west,
                    }));
                }
            }
            ItemDrag::Idle
        }
        ItemDrag::Pressed { from, at } if cursor.is_some_and(|c| drag_moved(at, c)) => {
            let sprite = game
                .me()
                .and_then(|me| me.inventory.slots().get(from))
                .and_then(|&(id, _)| item(id));
            if let Some(def) = sprite {
                let mut icon = item_icon(&assets, def.sprite).node();
                icon.color = icon.color.with_alpha(DRAG_GHOST_ALPHA);
                commands.entity(ghost).insert(icon);
            }
            ItemDrag::Dragging { from }
        }
        current => current,
    };
    drag.set_if_neq(next);
    let display = match (next, cursor) {
        (ItemDrag::Dragging { .. }, Some(cursor)) => {
            let corner = cursor - Vec2::splat(ICON_SIZE / 2.0);
            ghost_node.left = px(corner.x);
            ghost_node.top = px(corner.y);
            Display::Flex
        }
        _ => Display::None,
    };
    if ghost_node.display != display {
        ghost_node.display = display;
    }
}

/// One inventory slot: empty, or an item with its count. `index` is into `Inventory::slots`.
fn spawn_inventory_slot(
    parent: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    me: &Player,
    position: usize,
    index: Option<usize>,
    selected: usize,
) {
    let file = if index == Some(selected) {
        SLOT_SELECTED_IMAGE
    } else {
        SLOT_IMAGE
    };
    let mut cell = parent.spawn((slot_cell(assets, file), CarriedCell(position)));
    let Some((index, &(id, count))) = index.and_then(|i| Some((i, me.inventory.slots().get(i)?)))
    else {
        return;
    };
    cell.insert((UiButton::Slot(index), ItemTip(Some((id, count)))))
        .with_children(|cell| {
            if let Some(icon) = item_icon_node(assets, id) {
                cell.spawn(icon);
            }
            if let Some(badge) = crate::vfx::seed_badge(assets, id) {
                cell.spawn(badge);
            }
            if let Some(count) = count_text(count) {
                cell.spawn(count);
            }
        });
}

/// Rebuilds the hotbar and the inventory grid from the player. The grid shows the chip, search and
/// sort view (`inventory_filter.rs`); its cells keep their real place for dragging.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn refresh_slots(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    selected: Res<Selected>,
    view: Res<crate::inventory_filter::InventoryView>,
    mut events: MessageReader<GameEvent>,
    hotbar: Single<Entity, With<HotbarRow>>,
    grid: Single<Entity, With<InventoryGrid>>,
) {
    let inventory_changed = events.read().any(|GameEvent(event)| match *event {
        Event::InventoryChanged { player, .. }
        | Event::BackpackUpgraded { player, .. }
        | Event::InventoryMoved { player, .. } => player == game.1,
        _ => false,
    });
    // `selected` also reads as changed on the first run, which fills the HUD once at startup.
    if !inventory_changed && !selected.is_changed() && !view.is_changed() {
        return;
    }
    let Some(me) = game.me() else { return };
    let carried = carried_slots(me);
    commands
        .entity(*hotbar)
        .despawn_children()
        .with_children(|row| {
            for n in 0..HOTBAR_SLOTS {
                spawn_inventory_slot(row, &assets, me, n, carried.get(n).copied(), selected.0);
            }
        });
    // Free backpack slots are shown empty after the items, unless a filter is on.
    let free = usize::from(me.inventory.capacity).saturating_sub(me.inventory.slots().len());
    let (positions, show_free) = view.cells(me, &carried);
    let free = (carried.len()..carried.len() + free).filter(|_| show_free);
    let cells = positions.iter().map(|&p| (p, Some(carried[p])));
    let cells = cells.chain(free.map(|p| (p, None)));
    commands
        .entity(*grid)
        .despawn_children()
        .with_children(|grid| {
            for (position, index) in cells {
                spawn_inventory_slot(grid, &assets, me, position, index, selected.0);
            }
            if positions.is_empty() {
                crate::inventory_filter::spawn_empty(grid, &view);
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
                                .spawn((
                                    slot_cell(&assets, SLOT_WORN_IMAGE),
                                    UiButton::Worn(slot),
                                    ItemTip(Some((id, 1))),
                                ))
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
                grid.spawn((
                    slot_cell(&assets, file),
                    UiButton::Equip(id),
                    ItemTip(Some((id, count))),
                ))
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
    armed: Res<ArmedBuy>,
    focus: Res<StoreFocus>,
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
    if !changed && !tab.is_changed() && !armed.is_changed() && !focus.is_changed() && *filled {
        return;
    }
    *filled = true;
    let Some(me) = game.me() else { return };
    let armed = armed.0.map(|(button, _)| button);
    let focused = store_entries(*tab).get(focus.0).copied();
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
        // The balance, top right, in the price style; clear of the close button.
        let balance = Node {
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            column_gap: px(SLOT_GAP),
            margin: UiRect::right(px(CLOSE_SIZE - CLOSE_OVERHANG - PANEL_PADDING)),
            ..default()
        };
        list.spawn((balance, Hovered::default()))
            .with_children(|balance| {
                balance.spawn(gem_icon(&assets));
                balance.spawn((label(me.gems.to_string()), TextColor(INK_COLOR)));
            });
        list.spawn((tabs, Hovered::default()))
            .with_children(|tabs| {
                tabs.spawn((ink(PREV_TAB_HINT), Hovered::default()));
                for (choice, name) in STORE_TABS {
                    tabs.spawn(tab_bundle(&assets, choice, *tab))
                        .with_children(|button| {
                            button.spawn(label(name));
                        });
                }
                tabs.spawn((ink(NEXT_TAB_HINT), Hovered::default()));
            });
    });
    let StoreTab::Equip(slot) = *tab else {
        claims_and_backpack(&mut commands, &assets, me, *list, armed, focused);
        return;
    };
    let offered = store_offer(slot);
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
                let button = UiButton::Buy(def.id);
                let is_armed = armed == Some(button);
                // Armed: the frame becomes the button (the strip says the rest).
                let (file, inset) = if is_armed {
                    (BUY_IMAGE, BUY_INSET)
                } else if focused == Some(button) {
                    (SLOT_SELECTED_IMAGE, SLOT_INSET)
                } else if owned {
                    (SLOT_WORN_IMAGE, SLOT_INSET)
                } else {
                    (SLOT_IMAGE, SLOT_INSET)
                };
                let cell = Node {
                    width: px(SLOT_SIZE),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                };
                grid.spawn((cell, button, ItemTip(Some((def.id, 1))), Hovered::default()))
                    .with_children(|cell| {
                        let mut frame = cell.spawn((
                            slot_node(),
                            sliced(&assets, file, inset),
                            Hovered::default(),
                        ));
                        if is_armed {
                            frame.insert(ArmedArt);
                        }
                        frame.with_children(|slot| {
                            let mut icon = item_icon(&assets, def.sprite).node();
                            icon.color = tint;
                            slot.spawn((icon, icon_node(ICON_SIZE)));
                        });
                        if is_armed {
                            cell.spawn((label(BUY_SHORT_TEXT), TextColor(INK_COLOR)));
                        } else {
                            let line = Node {
                                column_gap: px(SLOT_GAP / 2.0),
                                align_items: AlignItems::Center,
                                ..default()
                            };
                            cell.spawn((line, Hovered::default()))
                                .with_children(|line| {
                                    line.spawn(gem_icon(&assets));
                                    line.spawn((label(price.to_string()), TextColor(text)));
                                });
                        }
                    });
            }
        });
    });
}

/// The gem icon in front of a price, one text line high.
fn gem_icon(assets: &AssetServer) -> impl Bundle {
    (
        ImageNode::new(assets.load(GEM_ICON)),
        icon_node(MOD_ICON_SIZE),
    )
}

/// The armed price of a claim row (U7): primary button art saying "Buy for <gem> <price>?", the
/// full row height.
fn armed_price(parent: &mut ChildSpawnerCommands, assets: &AssetServer, price: u32) {
    let node = Node {
        min_height: px(TOUCH_MIN),
        padding: UiRect::axes(px(BUY_INSET), px(SLOT_GAP)),
        column_gap: px(SLOT_GAP),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        align_self: AlignSelf::Stretch,
        flex_shrink: 0.0,
        ..default()
    };
    let words = |text: String| (label(text), TextColor(Color::WHITE), TextShadow::default());
    parent
        .spawn((
            node,
            sliced(assets, BUY_IMAGE, BUY_INSET),
            ArmedArt,
            Hovered::default(),
        ))
        .with_children(|button| {
            button.spawn(words(BUY_CONFIRM_TEXT.to_string()));
            button
                .spawn((row(SLOT_GAP / 2.0), Hovered::default()))
                .with_children(|amount| {
                    amount.spawn(gem_icon(assets));
                    amount.spawn(words(format!("{price}?")));
                });
        });
}

/// The armed price cell's art follows the pointer.
fn skin_armed(
    assets: Res<AssetServer>,
    mut buttons: Query<(&Hovered, &mut ImageNode), With<ArmedArt>>,
) {
    for (hovered, mut image) in &mut buttons {
        let file = if hovered.get() {
            BUY_HOVER_IMAGE
        } else {
            BUY_IMAGE
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

/// Whether buying at `price` with `gems` takes a second click: expensive, and affordable (an
/// unaffordable click goes straight to the rules, which answer `Not enough gems`).
fn needs_confirm(price: u32, gems: u32) -> bool {
    price >= CONFIRM_PRICE && gems >= price
}

/// The store price behind a buy button; `None` when there is nothing to buy.
fn buy_price(button: UiButton, me: &Player) -> Option<u32> {
    match button {
        UiButton::Buy(id) => claim_kind(id)
            .map(|kind| kind.price)
            .or_else(|| equip_price(id)),
        UiButton::BuyUpgrade => {
            let upgrades = me.inventory.upgrades();
            (upgrades < MAX_UPGRADES).then(|| upgrade_cost(upgrades))
        }
        _ => None,
    }
}

/// A click (or Enter / South on the focused entry) on a buy button: arms an expensive one, else
/// sends the purchase. The confirm only delays sending; the rules still decide.
fn buy_or_arm(
    button: UiButton,
    me: &Player,
    now: f32,
    armed: &mut ArmedBuy,
    actions: &mut MessageWriter<ActionMsg>,
) {
    let was_armed = armed.0.map(|(b, _)| b);
    let confirm = buy_price(button, me).is_some_and(|price| needs_confirm(price, me.gems));
    if confirm && was_armed != Some(button) {
        armed.0 = Some((button, now + CONFIRM_SECS));
        return;
    }
    if armed.0.is_some() {
        armed.0 = None;
    }
    actions.write(ActionMsg(match button {
        UiButton::Buy(item) => Action::Buy { item },
        _ => Action::BuyBackpackUpgrade,
    }));
}

/// The equipment of one slot on sale, cheapest first (table order within one price).
fn store_offer(slot: Slot) -> Vec<(&'static ItemDef, u32)> {
    let mut offered: Vec<_> = ITEMS
        .iter()
        .filter(|def| def.kind == ItemKind::Equip(slot))
        .filter_map(|def| Some((def, equip_price(def.id)?)))
        .collect();
    offered.sort_by_key(|&(_, price)| price);
    offered
}

/// The store tab's entries in the order shown, for the keyboard / gamepad focus.
fn store_entries(tab: StoreTab) -> Vec<UiButton> {
    match tab {
        StoreTab::Claims => CLAIMS
            .iter()
            .filter(|kind| item(kind.item).is_some())
            .map(|kind| UiButton::Buy(kind.item))
            .chain([UiButton::BuyUpgrade])
            .collect(),
        StoreTab::Equip(slot) => store_offer(slot)
            .into_iter()
            .map(|(def, _)| UiButton::Buy(def.id))
            .collect(),
    }
}

/// While the store is open: arrows / d-pad, Enter / South and East, taken from the keyboard so the
/// player does not also walk.
fn take_store_keys(
    open: Res<OpenPanel>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut out: ResMut<StoreKeys>,
) {
    if *open != OpenPanel::Store {
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
    out.activate |= key(STORE_ACTIVATE_KEY) || pad(GamepadButton::South);
    out.back |= pad(GamepadButton::East);
    for code in STORE_NAV_KEYS.into_iter().chain([STORE_ACTIVATE_KEY]) {
        if keys.pressed(code) {
            keys.reset(code);
        }
    }
}

/// The `[` / `]` keys and LB / RB while a tabbed panel is open, kept from the game and the chat.
pub(crate) fn take_tab_keys(
    open: Res<OpenPanel>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    mut out: ResMut<TabKeys>,
) {
    let tabbed = *open == OpenPanel::Store || BAG_TABS.iter().any(|(panel, _)| *panel == *open);
    if !tabbed {
        return;
    }
    let pad = |button| pads.iter().any(|pad| pad.just_pressed(button));
    out.0 += i32::from(keys.just_pressed(NEXT_TAB_KEY) || pad(GamepadButton::RightTrigger))
        - i32::from(keys.just_pressed(PREV_TAB_KEY) || pad(GamepadButton::LeftTrigger));
    for code in [PREV_TAB_KEY, NEXT_TAB_KEY] {
        if keys.pressed(code) {
            keys.reset(code);
        }
    }
}

/// `index` moved by `step` in a list of `len`, wrapping around.
fn cycle(index: usize, len: usize, step: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (index as i64 + i64::from(step)).rem_euclid(len as i64) as usize
}

/// Switches the tab of the open panel: the store's category, or the bag's panel. Focus goes back
/// to the first entry (`use_store_keys`, `craft_ui::use_keys`).
fn use_tab_keys(
    mut keys: ResMut<TabKeys>,
    mut open: ResMut<OpenPanel>,
    mut tab: ResMut<StoreTab>,
    mut sfx: MessageWriter<Sfx>,
) {
    let step = std::mem::take(&mut keys.0);
    if step == 0 {
        return;
    }
    if *open == OpenPanel::Store {
        let at = STORE_TABS.iter().position(|(t, _)| *t == *tab).unwrap_or(0);
        tab.set_if_neq(STORE_TABS[cycle(at, STORE_TABS.len(), step)].0);
    } else if let Some(at) = BAG_TABS.iter().position(|(panel, _)| *panel == *open) {
        open.set_if_neq(BAG_TABS[cycle(at, BAG_TABS.len(), step)].0);
    } else {
        return;
    }
    sfx.write(Sfx::UiClick);
}

/// The store's focus: moved by the arrows, a click by Enter / South (arm, then buy), East
/// disarms first, then closes (Esc does the same in `toggle_panels`).
#[allow(clippy::too_many_arguments)] // a Bevy system
fn use_store_keys(
    time: Res<Time>,
    game: Res<Game>,
    tab: Res<StoreTab>,
    mut open: ResMut<OpenPanel>,
    mut keys: ResMut<StoreKeys>,
    mut focus: ResMut<StoreFocus>,
    mut armed: ResMut<ArmedBuy>,
    mut actions: MessageWriter<ActionMsg>,
    mut sfx: MessageWriter<Sfx>,
) {
    let StoreKeys {
        step,
        activate,
        back,
    } = std::mem::take(&mut *keys);
    if (open.is_changed() || tab.is_changed()) && focus.0 != 0 {
        focus.0 = 0;
    }
    if *open != OpenPanel::Store {
        return;
    }
    let Some(me) = game.me() else { return };
    let entries = store_entries(*tab);
    let columns = match *tab {
        StoreTab::Claims => 1,
        StoreTab::Equip(_) => STORE_COLUMNS as usize,
    };
    if let Some(step) = step {
        let next = crate::craft_ui::step_index(focus.0, entries.len(), step, columns);
        if next != focus.0 {
            focus.0 = next;
            sfx.write(Sfx::UiClick);
        }
    }
    if activate && let Some(&button) = entries.get(focus.0) {
        sfx.write(Sfx::UiClick);
        buy_or_arm(button, me, time.elapsed_secs(), &mut armed, &mut actions);
    }
    if back {
        if armed.0.is_some() {
            armed.0 = None;
        } else {
            *open = OpenPanel::None;
        }
    }
}

fn tab_bundle(assets: &AssetServer, choice: StoreTab, on_show: StoreTab) -> impl Bundle {
    tab(assets, UiButton::Tab(choice), choice == on_show)
}

/// The store's line under a claim: what it covers (docs/knowledge/ux/claims.md, 6).
fn claim_coverage(tiles: Option<u32>) -> String {
    match tiles {
        Some(n) => format!("Protects the {n} tiles nearest it"),
        None => "Protects the whole world. Only if no one else has a claim here".to_string(),
    }
}

/// Shown above the hotbar while the pointer is on a claim row of the store.
const CLAIM_ROW_HINT: &str = "Hold it in the world to see the area first";

/// The claims tab: the four claims and the backpack upgrade, one row each.
fn claims_and_backpack(
    commands: &mut Commands,
    assets: &AssetServer,
    me: &Player,
    list: Entity,
    armed: Option<UiButton>,
    focused: Option<UiButton>,
) {
    let upgrades = me.inventory.upgrades();
    let claims = CLAIMS.iter().filter_map(|kind| {
        let def = item(kind.item)?;
        Some((
            UiButton::Buy(kind.item),
            Some(def.sprite),
            def.name.to_string(),
            Some(claim_coverage(kind.tiles)),
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
        None,
        (upgrades < MAX_UPGRADES).then(|| upgrade_cost(upgrades)),
    );
    let entries: Vec<_> = claims.chain([upgrade]).collect();
    commands.entity(list).with_children(|list| {
        for (button, sprite, text, line, price) in entries {
            let affordable = price.is_some_and(|price| me.gems >= price);
            let colour = if affordable {
                Color::WHITE
            } else {
                DISABLED_TEXT_COLOR
            };
            let armed_at = price.filter(|_| armed == Some(button));
            let frame = if focused == Some(button) {
                SLOT_SELECTED_IMAGE
            } else {
                SLOT_IMAGE
            };
            let row = Node {
                min_height: px(ROW_MIN_HEIGHT),
                padding: UiRect::axes(px(ROW_PADDING), px(HUD_MARGIN)),
                ..button_node()
            };
            let mut entry = list.spawn((
                row,
                sliced(assets, frame, SLOT_INSET),
                button,
                Hovered::default(),
            ));
            if let UiButton::Buy(id) = button {
                entry.insert(ItemTip(Some((id, 1))));
            }
            entry.with_children(|entry| {
                match sprite {
                    Some(sprite) => {
                        entry.spawn((item_icon(assets, sprite).node(), icon_node(ROW_ICON_SIZE)))
                    }
                    None => entry.spawn(icon_node(ROW_ICON_SIZE)),
                };
                let grow = Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    ..default()
                };
                entry.spawn(grow).with_children(|words| {
                    words.spawn((label(text), TextColor(colour)));
                    if let Some(line) = line {
                        words.spawn((label(line), TextColor(colour)));
                    }
                });
                match (armed_at, price) {
                    (Some(at), _) => armed_price(entry, assets, at),
                    (None, Some(price)) => {
                        let line = Node {
                            column_gap: px(SLOT_GAP / 2.0),
                            align_items: AlignItems::Center,
                            ..default()
                        };
                        entry
                            .spawn((line, Hovered::default()))
                            .with_children(|line| {
                                line.spawn(gem_icon(assets));
                                line.spawn((label(price.to_string()), TextColor(colour)));
                            });
                    }
                    (None, None) => {
                        entry.spawn((label("sold out"), TextColor(colour)));
                    }
                }
            });
        }
    });
}

/// A mod in a few words, for the line beside an item's name.
pub(crate) fn mod_label(m: Mod) -> String {
    match m {
        Mod::ExtraJump => "Extra jump".to_string(),
        Mod::Glide => "Glide".to_string(),
        Mod::RunBoost(pct) => format!("+{pct}% run"),
        Mod::DigBoost(points) => {
            let times = format!("{:.2}", f32::from(points) / f32::from(FIST_DAMAGE));
            // 1.50 -> 1.5, 2.00 -> 2.0, 1.25 stays.
            let times = times.strip_suffix('0').unwrap_or(&times);
            format!("Dig {times}×")
        }
        Mod::Reach(tiles) => format!("+{tiles} reach"),
        Mod::GemFinder(bonus) => format!("+{bonus} gems"),
    }
}

pub(crate) fn mod_icon(m: Mod) -> &'static str {
    match m {
        Mod::ExtraJump => "ui/mods/mod_extra_jump.png",
        Mod::Glide => "ui/mods/mod_glide.png",
        Mod::RunBoost(_) => "ui/mods/mod_run_boost.png",
        Mod::DigBoost(_) => "ui/mods/mod_dig_boost.png",
        Mod::Reach(_) => "ui/mods/mod_reach.png",
        Mod::GemFinder(_) => "ui/mods/mod_gem_finder.png",
    }
}

/// An icon and a label per mod of `id`, into a row; nothing for an item without mods.
pub(crate) fn spawn_mods(
    row: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    id: ItemId,
    color: Color,
) {
    for m in mods_of(id) {
        spawn_mod_entry(row, assets, m, color);
    }
}

/// One mod: its icon and its label.
fn spawn_mod_entry(row: &mut ChildSpawnerCommands, assets: &AssetServer, m: Mod, color: Color) {
    row.spawn(Node {
        column_gap: px(SLOT_GAP),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|entry| {
        entry.spawn((
            ImageNode::new(assets.load(mod_icon(m))),
            icon_node(MOD_ICON_SIZE),
        ));
        entry.spawn((label(mod_label(m)), TextColor(color)));
    });
}

/// What the tooltip and the selection strip say about an item (U3): lines left out when there is
/// nothing to say.
#[derive(Clone, PartialEq, Debug)]
struct ItemInfo {
    /// Name in title case, with the count when more than one.
    title: String,
    mods: Vec<Mod>,
    /// What it is (`kind_line`).
    kind: Option<String>,
    /// Store only: the price and whether the player can pay it.
    price: Option<(u32, bool)>,
    /// Owned and droppable: `Q drop · Shift+Q drop stack`.
    hint: Option<String>,
}

fn item_info(id: ItemId, count: u16, worn: bool, price: Option<(u32, bool)>) -> Option<ItemInfo> {
    let def = item(id)?;
    let name = title_case(def.name);
    Some(ItemInfo {
        title: if count > 1 {
            format!("{name} ×{count}")
        } else {
            name
        },
        mods: mods_of(id).collect(),
        kind: kind_line(def, worn),
        price,
        hint: (price.is_none() && crate::interact::discardable(id)).then(drop_hint),
    })
}

/// The drop keys, from the constant.
fn drop_hint() -> String {
    let key = key_name(crate::interact::DROP_KEY);
    format!("{key} drop{SEP}Shift+{key} drop stack")
}

/// "golden pickaxe" -> "Golden Pickaxe".
pub(crate) fn title_case(name: &str) -> String {
    let word = |w: &str| {
        let mut chars = w.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    };
    name.split(' ').map(word).collect::<Vec<_>>().join(" ")
}

/// Seconds as "45s", "3m 20s", "1h 5m" (a zero last part is left out).
/// The one duration format: the seed tooltip, the plant toast and the tree card (farming_ui.rs).
pub(crate) fn duration_text(secs: u64) -> String {
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, 0, _) => format!("{h}h"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// Tooltip line 3: what the item is, by kind; `· Farmable` when it has a seed form.
pub(crate) fn kind_line(def: &ItemDef, worn: bool) -> Option<String> {
    let line = kind_text(def, worn)?;
    Some(match seed_of(def.id) {
        Some(_) => format!("{line}{SEP}Farmable"),
        None => line,
    })
}

fn kind_text(def: &ItemDef, worn: bool) -> Option<String> {
    Some(match def.kind {
        _ if def.id == LADDER => format!("Ladder{SEP}climb with W / S"),
        _ if def.id == WORKBENCH => {
            format!(
                "Crafting station{SEP}press {} nearby",
                key_name(CRAFT_PANEL_KEY)
            )
        }
        ItemKind::Seed => format!(
            "Grows: {}{SEP}{}",
            title_case(game_shared::items::seed_parent(def.id).map_or("?", item_name)),
            duration_text(grow_seconds(def.rarity))
        ),
        ItemKind::Block => match def.hits {
            0 => "Block".to_string(),
            1 => format!("Block{SEP}1 punch"),
            hits => format!("Block{SEP}{hits} punches"),
        },
        ItemKind::Background => "Background".to_string(),
        ItemKind::Claim => claim_coverage(claim_kind(def.id)?.tiles),
        ItemKind::Tool if def.id == WRENCH => "Click your claim or a Workbench".to_string(),
        ItemKind::Tool if def.id == FIST => "Punch blocks, harvest trees".to_string(),
        ItemKind::Tool => return None,
        ItemKind::Equip(slot) => {
            let worn = if worn {
                format!("{SEP}worn")
            } else {
                String::new()
            };
            format!("{slot:?}{worn}")
        }
        ItemKind::Resource => "Used in crafting".to_string(),
    })
}

/// A key as the player reads it on a label: `KeyB` -> "B", `Enter` -> "Enter".
pub(crate) fn key_name(key: KeyCode) -> String {
    let name = format!("{key:?}");
    let short = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"));
    short.unwrap_or(&name).to_string()
}

/// The tooltip under a top-bar button (U9). Keys from the constants, never typed twice.
fn top_label(button: TopButton) -> String {
    match button {
        TopButton::Store => format!("Store ({})", key_name(STORE_KEY)),
        TopButton::Inventory => format!("Inventory ({})", key_name(INVENTORY_KEYS[0])),
        TopButton::Wardrobe => "Wardrobe".to_string(),
        TopButton::Craft => format!("Craft ({})", key_name(CRAFT_PANEL_KEY)),
        TopButton::Chat => format!("Chat ({})", key_name(CHAT_KEY)),
        TopButton::Almanac => format!("Almanac ({})", key_name(crate::almanac::ALMANAC_KEY)),
        TopButton::World => format!("Worlds ({})", key_name(crate::world_select::WORLD_KEY)),
        TopButton::Voice => format!("Voice ({})", key_name(VOICE_KEY)),
        TopButton::Settings => "Settings".to_string(),
    }
}

/// Where the tooltip goes (logical px): right of the cell and top-aligned, left of it when the
/// window ends first, or under it (`below`, the top bar); always fully on screen.
fn tip_position(cell: Rect, tip: Vec2, window: Vec2, below: bool) -> Vec2 {
    let at = if below {
        Vec2::new(cell.max.x - tip.x, cell.max.y + SLOT_GAP)
    } else if cell.max.x + SLOT_GAP + tip.x <= window.x {
        Vec2::new(cell.max.x + SLOT_GAP, cell.min.y)
    } else {
        Vec2::new(cell.min.x - SLOT_GAP - tip.x, cell.min.y)
    };
    at.clamp(Vec2::ZERO, (window - tip).max(Vec2::ZERO))
}

/// What the tooltip shows.
#[derive(Clone, PartialEq)]
enum TipContent {
    Item(ItemInfo),
    Text(String),
}

/// A node's rectangle in logical px.
fn logical_rect(node: &ComputedNode, transform: &UiGlobalTransform) -> Rect {
    let scale = node.inverse_scale_factor();
    Rect::from_center_size(transform.translation * scale, node.size * scale)
}

/// The shared tooltip (U3, U9): after the pointer rests `TOOLTIP_DELAY_SECS` on an item cell or a
/// top-bar button, beside the cell. Hidden at once when the pointer leaves, while dragging, on
/// touch, and for an empty cell; rebuilt only when what it says changes.
#[allow(clippy::too_many_arguments)] // a Bevy system: state, input, the cells and the box
fn show_tooltip(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    game: Res<Game>,
    drag: Res<ItemDrag>,
    touches: Res<Touches>,
    window: Single<&Window, With<PrimaryWindow>>,
    cells: Query<(
        Entity,
        &ItemTip,
        &Hovered,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&UiButton>,
    )>,
    tops: Query<
        (
            Entity,
            &UiButton,
            &Hovered,
            &ComputedNode,
            &UiGlobalTransform,
        ),
        Without<ItemTip>,
    >,
    tip: Single<(Entity, &mut Node, &ComputedNode), With<Tooltip>>,
    mut since: Local<Option<(Entity, f32)>>,
    mut shown: Local<Option<TipContent>>,
) {
    let quiet = drag.busy() || touches.iter().next().is_some();
    let hovered_cell = cells.iter().find(|(_, _, hovered, ..)| hovered.get());
    let target = if quiet {
        None
    } else if let Some((entity, ItemTip(stack), _, node, at, button)) = hovered_cell {
        stack.and_then(|(id, count)| {
            let me = game.me()?;
            let worn = me.equipped.contains(&Some(id));
            let price = match button {
                Some(&b @ UiButton::Buy(_)) => buy_price(b, me).map(|p| (p, me.gems >= p)),
                _ => None,
            };
            let info = item_info(id, count, worn, price)?;
            Some((
                entity,
                TipContent::Item(info),
                logical_rect(node, at),
                false,
            ))
        })
    } else {
        tops.iter()
            .find_map(|(entity, button, hovered, node, at)| match button {
                UiButton::Top(top) if hovered.get() => Some((
                    entity,
                    TipContent::Text(top_label(*top)),
                    logical_rect(node, at),
                    true,
                )),
                _ => None,
            })
    };
    let now = time.elapsed_secs();
    let entity = target.as_ref().map(|t| t.0);
    if since.map(|(e, _)| e) != entity {
        *since = entity.map(|e| (e, now));
    }
    let ready = since.is_some_and(|(_, from)| now - from >= TOOLTIP_DELAY_SECS);
    let (tip_entity, mut node, computed) = tip.into_inner();
    let Some((_, content, cell, below)) = target.filter(|_| ready) else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        *shown = None;
        return;
    };
    if shown.as_ref() != Some(&content) {
        commands
            .entity(tip_entity)
            .despawn_children()
            .with_children(|tip| spawn_tip_lines(tip, &assets, &content));
        *shown = Some(content);
    }
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    // The box's own size is known a frame after it is first shown; it settles then.
    let size = computed.size * computed.inverse_scale_factor();
    let at = tip_position(cell, size, window.size(), below);
    if node.left != px(at.x) || node.top != px(at.y) {
        node.left = px(at.x);
        node.top = px(at.y);
    }
}

/// The tooltip's lines: name, mods, what it is, price (red when the player can't pay).
fn spawn_tip_lines(tip: &mut ChildSpawnerCommands, assets: &AssetServer, content: &TipContent) {
    let info = match content {
        TipContent::Text(text) => {
            tip.spawn((ink(text.clone()), tip_part()));
            return;
        }
        TipContent::Item(info) => info,
    };
    tip.spawn((ink(info.title.clone()), tip_part()));
    if !info.mods.is_empty() {
        let mods = Node {
            column_gap: px(HUD_MARGIN),
            flex_wrap: FlexWrap::Wrap,
            ..default()
        };
        tip.spawn((mods, tip_part())).with_children(|row| {
            for &m in &info.mods {
                let entry = Node {
                    column_gap: px(SLOT_GAP),
                    align_items: AlignItems::Center,
                    ..default()
                };
                row.spawn((entry, tip_part())).with_children(|entry| {
                    entry.spawn((
                        ImageNode::new(assets.load(mod_icon(m))),
                        icon_node(MOD_ICON_SIZE),
                        tip_part(),
                    ));
                    entry.spawn((ink(mod_label(m)), tip_part()));
                });
            }
        });
    }
    if let Some(kind) = &info.kind {
        tip.spawn((ink(kind.clone()), tip_part()));
    }
    if let Some(hint) = &info.hint {
        tip.spawn((ink(hint.clone()), tip_part()));
    }
    if let Some((price, affordable)) = info.price {
        let colour = if affordable {
            INK_COLOR
        } else {
            SHORT_PRICE_COLOR
        };
        let line = Node {
            column_gap: px(SLOT_GAP),
            align_items: AlignItems::Center,
            ..default()
        };
        tip.spawn((line, tip_part())).with_children(|line| {
            line.spawn((gem_icon(assets), tip_part()));
            line.spawn((label(price.to_string()), TextColor(colour), tip_part()));
        });
    }
}

/// The selection strip above the hotbar: name, mods and what it is (tooltip lines 1-3, ` · `
/// between them) of the item cell under the pointer, else the last one pointed at or tapped while
/// a panel is open, else the selected item (`discard_target`). An armed equipment cell says
/// `Click again to buy for <gem> <price>` instead. Rewritten only when what it says changes.
#[allow(clippy::too_many_arguments)] // state, two selections, the cells and the two outputs
fn refresh_item_info(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    open: Res<OpenPanel>,
    selected: Res<Selected>,
    wardrobe: Res<WardrobeSelected>,
    armed: Res<ArmedBuy>,
    cells: Query<(&ItemTip, &Hovered, Option<&UiButton>)>,
    mut name: Single<&mut Text, With<SelectedName>>,
    mods: Single<Entity, With<ModLine>>,
    mut last_cell: Local<Option<(ItemId, u16)>>,
    mut shown: Local<Option<(Option<ItemInfo>, bool, Option<u32>)>>,
) {
    let hovered = cells.iter().find(|(_, hovered, _)| hovered.get());
    if open.is_changed() {
        *last_cell = None;
    }
    if let Some((ItemTip(Some(stack)), _, _)) = hovered
        && *open != OpenPanel::None
    {
        *last_cell = Some(*stack);
    }
    let Some(me) = game.me() else { return };
    let focus = hovered
        .and_then(|(tip, _, _)| tip.0)
        .or(*last_cell)
        .or_else(|| {
            let id = discard_target(&game, *open, &selected, &wardrobe)?;
            Some((id, me.inventory.count(id)))
        });
    let info =
        focus.and_then(|(id, count)| item_info(id, count, me.equipped.contains(&Some(id)), None));
    let claim_row = matches!(
        hovered.and_then(|(_, _, button)| button.copied()),
        Some(UiButton::Buy(id)) if claim_kind(id).is_some()
    );
    // Only the narrow equipment cells move their confirm text here; claim rows say it inline.
    let armed_cell = armed.0.and_then(|(button, _)| match button {
        UiButton::Buy(id) => equip_price(id),
        _ => None,
    });
    let view = (info, claim_row, armed_cell);
    if shown.as_ref() == Some(&view) {
        return;
    }
    let (info, claim_row, armed_cell) = &view;
    name.0 = match (armed_cell, info) {
        (Some(_), _) => STRIP_ARMED_TEXT.to_string(),
        (None, Some(info)) if *claim_row => format!("{}: {CLAIM_ROW_HINT}", info.title),
        (None, Some(info)) => info.title.clone(),
        (None, None) => String::new(),
    };
    commands
        .entity(*mods)
        .despawn_children()
        .with_children(|row| {
            if let Some(price) = armed_cell {
                row.spawn(gem_icon(&assets));
                row.spawn(label(price.to_string()));
                return;
            }
            let Some(info) = info else { return };
            for &m in &info.mods {
                row.spawn(label(STRIP_SEP));
                spawn_mod_entry(row, &assets, m, Color::WHITE);
            }
            if let Some(kind) = &info.kind {
                row.spawn(label(STRIP_SEP));
                row.spawn(label(kind.clone()));
            }
            if let Some(hint) = &info.hint {
                row.spawn(label(STRIP_SEP));
                row.spawn(label(hint.clone()));
            }
        });
    *shown = Some(view);
}

pub(crate) fn item_name(id: ItemId) -> &'static str {
    item(id).map_or("?", |def| def.name)
}

/// What to tell the player about an event, if anything.
fn toast_for_event(event: &Event) -> Option<String> {
    match *event {
        // Plant and splice toasts are the planter's own: `farming_ui.rs`.
        Event::BackpackUpgraded { capacity, .. } => {
            Some(format!("Backpack now holds {capacity} item types"))
        }
        // Private: only the crafter receives it.
        Event::Crafted { recipe: number, .. } => {
            recipe(number).map(|r| format!("Crafted {}", item_name(r.output.0)))
        }
        _ => None,
    }
}

/// The one-time ladder hint (docs/knowledge/ux/ladders.md).
const LADDER_TIP: &str = "W / S to climb · Space to jump off";

/// Once ever (`Settings::ladder_tip_seen`): the first time the local player is on a ladder tile
/// (read from the mirrored world), the message line says how to climb.
fn ladder_tip(game: Res<Game>, mut settings: ResMut<Settings>, mut notices: MessageWriter<Notice>) {
    if settings.ladder_tip_seen {
        return;
    }
    if game
        .me()
        .is_some_and(|me| game_shared::physics::on_ladder(&game.0.world, me.body.pos))
    {
        settings.ladder_tip_seen = true;
        notices.write(Notice(LADDER_TIP.to_string()));
    }
}

/// The player must always learn why nothing happened (U2), except where the world already shows
/// it (`SILENT_REJECTS`). Claim refusals name the owner and come from `claim_ui.rs`. A craft has
/// its own words for the refusals that mean something else there.
fn toast_for_reject(action: &Action, reject: Reject) -> Option<&'static str> {
    if let Action::Craft { .. } = action {
        match reject {
            Reject::OutOfRange => return Some("Stand next to a Workbench to craft that"),
            Reject::NoItem => return Some("Not enough ingredients"),
            _ => {}
        }
    }
    // A seed on a full tile (a placed fern looks like a plant): say what it needs. Other
    // `Occupied` stays silent, the tile is visibly full.
    if let Action::Place { item: placed, .. } = *action
        && reject == Reject::Occupied
        && item(placed).is_some_and(|def| def.kind == ItemKind::Seed)
    {
        return Some("Seeds need an empty tile or a sapling");
    }
    match reject {
        Reject::NoRecipe => Some("These two seeds cannot be spliced"),
        Reject::AlreadySpliced => {
            Some("This tree has already been spliced. Harvest it and plant a fresh seed")
        }
        Reject::NeedsGround => Some("Seeds need solid ground below"),
        Reject::DropLimit => Some("This world cannot hold more dropped items"),
        Reject::NotEnoughGems => Some("Not enough gems"),
        Reject::InventoryFull => Some("No room in the backpack"),
        Reject::WorldAlreadyClaimed => Some("This world is already claimed"),
        Reject::ForeignClaims => Some("Someone else has a claim in this world"),
        Reject::Unbreakable => Some("This block can't be broken"),
        Reject::NoItem => Some("You don't have that item"),
        Reject::Invalid => Some("That didn't work"),
        // Named by `claim_ui.rs`.
        Reject::NoPermission | Reject::NotOwner => None,
        Reject::Occupied | Reject::OutOfRange | Reject::Cooldown => None,
    }
}

/// Whether a refusal stays silent on purpose: no toast, no tile flash (`interact.rs`).
pub(crate) fn silent_reject(reject: Reject) -> bool {
    SILENT_REJECTS.contains(&reject)
}

/// Whether a refusal text may show at `now`: not the same text as the last one within
/// `REJECT_REPEAT_SECS`.
fn may_repeat(last: Option<(&str, f32)>, text: &str, now: f32) -> bool {
    last.is_none_or(|(shown, at)| shown != text || now >= at + REJECT_REPEAT_SECS)
}

/// A non-seed item placed on a sapling or tree (refused `Occupied`): how splicing works instead.
/// `tree_at` says whether a tile holds one (the mirrored world).
fn splice_hint(
    action: &Action,
    reject: Reject,
    tree_at: impl Fn(u32, u32) -> bool,
) -> Option<String> {
    let Action::Place { x, y, item: placed } = *action else {
        return None;
    };
    let seed = item(placed).is_some_and(|def| def.kind == ItemKind::Seed);
    if reject != Reject::Occupied || seed || !tree_at(x, y) {
        return None;
    }
    Some(match seed_of(placed).and(item(placed)) {
        Some(def) => format!(
            "To splice, use a seed (e.g. {} seed) on the sapling",
            title_case(def.name)
        ),
        None => "To splice, use a seed on the sapling".to_string(),
    })
}

fn show_toasts(
    time: Res<Time>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut results: MessageReader<ActionResult>,
    mut notices: MessageReader<Notice>,
    toast: Single<(&mut Text, &mut Toast)>,
    mut last_reject: Local<Option<(String, f32)>>,
) {
    let (mut text, mut toast) = toast.into_inner();
    let now = time.elapsed_secs();
    let from_events = events
        .read()
        .filter_map(|GameEvent(event)| toast_for_event(event));
    let mut from_rejects = Vec::new();
    for ActionResult { action, result } in results.read() {
        let Err(reject) = result else { continue };
        let reject = *reject;
        if reject == Reject::Invalid {
            // The catch-all: logged so it can get a real message later.
            warn!("{action:?} rejected: {reject:?}");
        }
        let tree_at = |x, y| game.0.world.tree(x, y).is_some();
        let message = splice_hint(action, reject, tree_at)
            .or_else(|| toast_for_reject(action, reject).map(String::from));
        if let Some(message) = message
            && may_repeat(
                last_reject.as_ref().map(|(t, at)| (t.as_str(), *at)),
                &message,
                now,
            )
        {
            *last_reject = Some((message.clone(), now));
            from_rejects.push(message);
        }
    }
    let from_notices = notices.read().map(|Notice(text)| text.clone());
    if let Some(message) = from_events.chain(from_rejects).chain(from_notices).last() {
        text.0 = message;
        toast.until = now + TOAST_SECS;
    } else if !text.0.is_empty() && now >= toast.until {
        text.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::DIRT;

    #[test]
    fn every_mod_reads_in_a_few_words() {
        assert_eq!(mod_label(Mod::ExtraJump), "Extra jump");
        assert_eq!(mod_label(Mod::Glide), "Glide");
        assert_eq!(mod_label(Mod::RunBoost(20)), "+20% run");
        assert_eq!(mod_label(Mod::DigBoost(4)), "Dig 1.0×");
        assert_eq!(mod_label(Mod::DigBoost(5)), "Dig 1.25×");
        assert_eq!(mod_label(Mod::DigBoost(6)), "Dig 1.5×");
        assert_eq!(mod_label(Mod::DigBoost(8)), "Dig 2.0×");
        assert_eq!(mod_label(Mod::Reach(1)), "+1 reach");
        assert_eq!(mod_label(Mod::GemFinder(1)), "+1 gems");
    }

    #[test]
    fn claim_rows_say_what_they_cover() {
        assert_eq!(claim_coverage(Some(48)), "Protects the 48 tiles nearest it");
        assert_eq!(
            claim_coverage(None),
            "Protects the whole world. Only if no one else has a claim here"
        );
    }

    #[test]
    fn crafting_says_what_was_made() {
        let crafted = Event::Crafted {
            player: game_shared::rules::PlayerId(1),
            recipe: 0,
        };
        assert_eq!(
            toast_for_event(&crafted).as_deref(),
            Some("Crafted workbench")
        );
        let unknown = Event::Crafted {
            player: game_shared::rules::PlayerId(1),
            recipe: u16::MAX,
        };
        assert_eq!(toast_for_event(&unknown), None);
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
    fn a_hotbar_place_maps_to_the_raw_slot_shown_there() {
        // Raw slots: 0 fist, 1 hat, 2 wrench, 3 shirt, 4 dirt, 5 hair. Equipment (1, 3, 5) is
        // never shown, so the view is [0, 2, 4].
        let carried = [0, 2, 4];
        let len = 6;
        assert_eq!(
            move_target(&carried, len, 4, 0),
            Some(0),
            "dirt onto hotbar 1: swap with fist"
        );
        assert_eq!(
            move_target(&carried, len, 0, 1),
            Some(2),
            "fist onto hotbar 2: the wrench's raw slot"
        );
        assert_eq!(move_target(&carried, len, 2, 1), None, "onto itself");
        // Empty places: past the end, the item goes last, behind the trailing equipment.
        assert_eq!(move_target(&carried, len, 0, 5), Some(6));
        assert_eq!(
            move_target(&carried, len, 4, 7),
            None,
            "already the last shown item"
        );
        // After a move to the end, the item is the last one shown.
        let mut slots: Vec<usize> = (0..len).collect();
        let item = slots.remove(0);
        slots.push(item);
        let shown: Vec<_> = slots.iter().filter(|&&i| [0, 2, 4].contains(&i)).collect();
        assert_eq!(shown.last(), Some(&&0));
    }

    #[test]
    fn the_selection_follows_a_moved_item() {
        assert_eq!(index_after_move(3, 3, 0, 5), 0);
        assert_eq!(index_after_move(0, 3, 0, 5), 3);
        assert_eq!(index_after_move(2, 3, 0, 5), 2);
        assert_eq!(index_after_move(1, 1, 5, 5), 4, "moved to the end");
        assert_eq!(index_after_move(3, 1, 5, 5), 2, "shifted down");
        assert_eq!(index_after_move(0, 1, 5, 5), 0);
    }

    #[test]
    fn only_a_real_drag_released_on_a_slot_moves_an_item() {
        let at = Vec2::new(100.0, 100.0);
        assert!(
            !drag_moved(at, at),
            "a press without movement stays a click"
        );
        assert!(!drag_moved(at, at + Vec2::new(DRAG_THRESHOLD - 1.0, 0.0)));
        assert!(drag_moved(at, at + Vec2::new(0.0, DRAG_THRESHOLD)));
        let carried = [0, 1, 2];
        let pressed = ItemDrag::Pressed { from: 2, at };
        assert_eq!(
            drop_action(pressed, Some(0), &carried, 3),
            None,
            "a click moves nothing"
        );
        let dragging = ItemDrag::Dragging { from: 2 };
        assert_eq!(
            drop_action(dragging, None, &carried, 3),
            None,
            "released outside: cancelled"
        );
        assert_eq!(
            drop_action(dragging, Some(0), &carried, 3),
            Some(Action::MoveItem { from: 2, to: 0 })
        );
        assert!(dragging.busy() && pressed.busy() && !ItemDrag::Idle.busy());
    }

    /// Every `Reject` is either mapped to its exact text, named by the claim UI, or silent on
    /// purpose. The `match` stops compiling when a variant is added, so none is forgotten.
    #[test]
    fn every_refusal_says_why_or_is_silent_on_purpose() {
        fn listed(reject: Reject) -> Option<&'static str> {
            match reject {
                Reject::NoRecipe => Some("These two seeds cannot be spliced"),
                Reject::AlreadySpliced => {
                    Some("This tree has already been spliced. Harvest it and plant a fresh seed")
                }
                Reject::NeedsGround => Some("Seeds need solid ground below"),
                Reject::DropLimit => Some("This world cannot hold more dropped items"),
                Reject::NotEnoughGems => Some("Not enough gems"),
                Reject::InventoryFull => Some("No room in the backpack"),
                Reject::WorldAlreadyClaimed => Some("This world is already claimed"),
                Reject::ForeignClaims => Some("Someone else has a claim in this world"),
                Reject::Unbreakable => Some("This block can't be broken"),
                Reject::NoItem => Some("You don't have that item"),
                Reject::Invalid => Some("That didn't work"),
                Reject::NoPermission | Reject::NotOwner => None,
                Reject::Occupied | Reject::OutOfRange | Reject::Cooldown => None,
            }
        }
        let all = [
            Reject::OutOfRange,
            Reject::NoPermission,
            Reject::NotOwner,
            Reject::Occupied,
            Reject::NoItem,
            Reject::Unbreakable,
            Reject::Cooldown,
            Reject::NoRecipe,
            Reject::AlreadySpliced,
            Reject::NeedsGround,
            Reject::DropLimit,
            Reject::NotEnoughGems,
            Reject::InventoryFull,
            Reject::WorldAlreadyClaimed,
            Reject::ForeignClaims,
            Reject::Invalid,
        ];
        let punch = Action::Punch { x: 1, y: 1 };
        for reject in all {
            let text = toast_for_reject(&punch, reject);
            assert_eq!(text, listed(reject), "{reject:?}");
            let claim_named = matches!(reject, Reject::NoPermission | Reject::NotOwner);
            assert_eq!(
                text.is_none(),
                silent_reject(reject) || claim_named,
                "{reject:?}: mapped, or silent on purpose"
            );
        }
        let craft = Action::Craft { recipe: 0 };
        assert_eq!(
            toast_for_reject(&craft, Reject::OutOfRange),
            Some("Stand next to a Workbench to craft that")
        );
        assert_eq!(
            toast_for_reject(&craft, Reject::NoItem),
            Some("Not enough ingredients")
        );
        let seed = ITEMS.iter().find(|d| d.kind == ItemKind::Seed).unwrap().id;
        let plant = |item| Action::Place { x: 1, y: 1, item };
        assert_eq!(
            toast_for_reject(&plant(seed), Reject::Occupied),
            Some("Seeds need an empty tile or a sapling")
        );
        assert_eq!(
            toast_for_reject(&plant(DIRT), Reject::Occupied),
            None,
            "a block: silent"
        );
        assert_eq!(
            toast_for_reject(&plant(seed), Reject::AlreadySpliced),
            Some("This tree has already been spliced. Harvest it and plant a fresh seed")
        );
        assert_eq!(
            toast_for_reject(&craft, Reject::InventoryFull),
            Some("No room in the backpack")
        );
    }

    #[test]
    fn tab_keys_wrap_around() {
        assert_eq!(cycle(0, 9, 1), 1);
        assert_eq!(cycle(8, 9, 1), 0, "past the last: the first");
        assert_eq!(cycle(0, 3, -1), 2, "before the first: the last");
        assert_eq!(cycle(0, 0, 1), 0);
    }

    #[test]
    fn a_block_on_a_sapling_explains_splicing() {
        let on = |item| Action::Place { x: 3, y: 4, item };
        let tree = |x, y| (x, y) == (3, 4);
        assert!(seed_of(DIRT).is_some());
        assert_eq!(
            splice_hint(&on(DIRT), Reject::Occupied, tree).as_deref(),
            Some("To splice, use a seed (e.g. Dirt seed) on the sapling")
        );
        assert!(seed_of(WORKBENCH).is_none());
        assert_eq!(
            splice_hint(&on(WORKBENCH), Reject::Occupied, tree).as_deref(),
            Some("To splice, use a seed on the sapling")
        );
        let no_tree = |_, _| false;
        assert_eq!(
            splice_hint(&on(DIRT), Reject::Occupied, no_tree),
            None,
            "a full tile: silent"
        );
        assert_eq!(toast_for_reject(&on(DIRT), Reject::Occupied), None);
        let seed = seed_of(DIRT).unwrap();
        assert_eq!(
            splice_hint(&on(seed), Reject::Occupied, tree),
            None,
            "a seed has its own text"
        );
        assert_eq!(splice_hint(&on(DIRT), Reject::NoItem, tree), None);
    }

    #[test]
    fn the_same_refusal_waits_two_seconds() {
        assert!(may_repeat(None, "a", 0.0));
        assert!(!may_repeat(Some(("a", 1.0)), "a", 2.9));
        assert!(may_repeat(Some(("a", 1.0)), "a", 3.0));
        assert!(may_repeat(Some(("a", 1.0)), "b", 1.5), "another text");
    }

    #[test]
    fn only_affordable_buys_of_a_thousand_gems_or_more_ask_twice() {
        assert!(!needs_confirm(999, 5000));
        assert!(needs_confirm(1000, 5000));
        assert!(needs_confirm(1000, 1000));
        assert!(
            !needs_confirm(1000, 999),
            "unaffordable: straight to the rules"
        );
        assert_eq!(CONFIRM_PRICE, 1000);
    }

    /// `" · Farmable"` for an item with a seed form.
    fn farmable(id: ItemId) -> &'static str {
        if seed_of(id).is_some() {
            " · Farmable"
        } else {
            ""
        }
    }

    #[test]
    fn the_tooltip_says_name_mods_and_what_it_is() {
        let find = |kind: ItemKind| ITEMS.iter().find(|d| d.kind == kind).unwrap();
        // Block, with count.
        let dirt = item(DIRT).unwrap();
        let info = item_info(DIRT, 5, false, None).unwrap();
        assert_eq!(info.title, "Dirt ×5");
        assert!(info.mods.is_empty());
        assert_eq!(
            info.kind,
            Some(format!("Block · {} punches{}", dirt.hits, farmable(DIRT)))
        );
        assert_eq!(info.price, None);
        assert_eq!(info.hint.as_deref(), Some("Q drop · Shift+Q drop stack"));
        // The fist and the wrench never leave: no hint.
        assert_eq!(item_info(FIST, 1, false, None).unwrap().hint, None);
        assert_eq!(item_info(WRENCH, 1, false, None).unwrap().hint, None);
        assert_eq!(item_info(DIRT, 1, false, None).unwrap().title, "Dirt");
        // Workbench, tools, background, resource, claim.
        assert_eq!(
            item_info(WORKBENCH, 1, false, None)
                .unwrap()
                .kind
                .as_deref(),
            Some(format!("Crafting station · press C nearby{}", farmable(WORKBENCH)).as_str())
        );
        assert_eq!(
            item_info(LADDER, 1, false, None).unwrap().kind.as_deref(),
            Some(format!("Ladder · climb with W / S{}", farmable(LADDER)).as_str())
        );
        assert_eq!(
            item_info(WRENCH, 1, false, None).unwrap().kind.as_deref(),
            Some("Click your claim or a Workbench")
        );
        assert_eq!(
            item_info(FIST, 1, false, None).unwrap().kind.as_deref(),
            Some("Punch blocks, harvest trees")
        );
        let wall = find(ItemKind::Background);
        assert_eq!(
            kind_line(wall, false),
            Some(format!("Background{}", farmable(wall.id)))
        );
        // No resource item yet: a made-up one.
        let resource = ItemDef {
            kind: ItemKind::Resource,
            ..*dirt
        };
        assert_eq!(
            kind_line(&resource, false),
            Some(format!("Used in crafting{}", farmable(DIRT)))
        );
        let small = &CLAIMS[0];
        assert_eq!(
            item_info(small.item, 1, false, Some((small.price, true))).unwrap(),
            ItemInfo {
                title: title_case(item(small.item).unwrap().name),
                mods: Vec::new(),
                kind: Some(claim_coverage(small.tiles)),
                price: Some((small.price, true)),
                hint: None,
            }
        );
        // Seed: what it grows and how long (farming: docs/knowledge/ux/vfx-farming-punch.md §d).
        let seed = item(seed_of(DIRT).unwrap()).unwrap();
        assert_eq!(
            kind_line(seed, false),
            Some(format!(
                "Grows: Dirt · {}",
                duration_text(grow_seconds(seed.rarity))
            ))
        );
        // Equipment: slot, worn, mods.
        let wings = ITEMS
            .iter()
            .find(|d| d.kind == ItemKind::Equip(Slot::Back) && mods_of(d.id).next().is_some())
            .unwrap();
        let info = item_info(wings.id, 1, true, Some((10_000, false))).unwrap();
        assert_eq!(info.kind.as_deref(), Some("Back · worn"));
        assert_eq!(info.mods, mods_of(wings.id).collect::<Vec<_>>());
        assert_eq!(info.price, Some((10_000, false)));
        let hat = find(ItemKind::Equip(Slot::Hat));
        assert_eq!(kind_line(hat, false).as_deref(), Some("Hat"));
    }

    #[test]
    fn names_are_title_case_and_times_read_short() {
        assert_eq!(title_case("golden pickaxe"), "Golden Pickaxe");
        assert_eq!(title_case("dirt"), "Dirt");
        assert_eq!(duration_text(45), "45s");
        assert_eq!(duration_text(200), "3m 20s");
        assert_eq!(duration_text(180), "3m");
        assert_eq!(duration_text(3900), "1h 5m");
        assert_eq!(duration_text(7200), "2h");
    }

    #[test]
    fn top_bar_labels_name_their_keys() {
        assert_eq!(top_label(TopButton::Store), "Store (B)");
        assert_eq!(top_label(TopButton::Inventory), "Inventory (E)");
        assert_eq!(top_label(TopButton::Wardrobe), "Wardrobe");
        assert_eq!(top_label(TopButton::Craft), "Craft (C)");
        assert_eq!(top_label(TopButton::Chat), "Chat (Enter)");
        assert_eq!(top_label(TopButton::Almanac), "Almanac (J)");
        assert_eq!(top_label(TopButton::World), "Worlds (M)");
        assert_eq!(top_label(TopButton::Voice), "Voice (N)");
        assert_eq!(top_label(TopButton::Settings), "Settings");
        assert_eq!(key_name(KeyCode::KeyC), "C");
    }

    #[test]
    fn the_tooltip_sits_beside_the_cell_and_on_screen() {
        let window = Vec2::new(800.0, 600.0);
        let tip = Vec2::new(200.0, 100.0);
        let cell = Rect::new(100.0, 100.0, 174.0, 174.0);
        assert_eq!(
            tip_position(cell, tip, window, false),
            Vec2::new(178.0, 100.0),
            "right"
        );
        let edge = Rect::new(700.0, 100.0, 774.0, 174.0);
        assert_eq!(
            tip_position(edge, tip, window, false),
            Vec2::new(496.0, 100.0),
            "left"
        );
        let low = Rect::new(100.0, 550.0, 174.0, 600.0);
        assert_eq!(
            tip_position(low, tip, window, false).y,
            500.0,
            "kept on screen"
        );
        let top = Rect::new(740.0, 8.0, 784.0, 52.0);
        assert_eq!(
            tip_position(top, tip, window, true),
            Vec2::new(584.0, 56.0),
            "under"
        );
    }
}
