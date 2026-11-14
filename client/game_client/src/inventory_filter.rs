//! The Inventory tab's category chips, live search and sort (docs/knowledge/ux/inventory-categories.md),
//! plus the selection strip's Drop / Trash buttons. A view only: the server's slot order, the
//! hotbar and trading never change. `hud::refresh_slots` asks `InventoryView::cells` what to show;
//! they filter the bag's grid rows only, never the hotbar row (docs/knowledge/ux/bottom-hud.md).

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::ScrollArea;
use game_shared::items::{ItemId, ItemKind, item};
use game_shared::rules::Player;

use crate::audio::Sfx;
use crate::hud::{
    HUD_MARGIN, INACTIVE_TAB_COLOR, INK_COLOR, KIT_SCALE, OpenPanel, SLOT_GAP, Selected,
    WardrobeSelected, discard_target, ink, label, sliced, tab_node, take_tab_keys,
};
use crate::interact::{DiscardPress, discardable};
use crate::settings::Settings;
use crate::state::{Game, Phase};

// Art (assets/README.md, "Farming, punch and inventory feedback").
const CHIP_IMAGE: &str = "ui/chip.png";
const CHIP_ACTIVE_IMAGE: &str = "ui/chip_active.png";
const CHIP_INSET: f32 = 6.0;
/// Touch: every chip at least this tall.
const CHIP_HEIGHT: f32 = 44.0;
const CHIP_PADDING: f32 = 12.0;
const FIELD_IMAGE: &str = "ui/lobby/input.png";
const FIELD_INSET: f32 = 8.0;
const FIELD_WIDTH: f32 = 260.0;
const FIELD_HEIGHT: f32 = 44.0;
const SEARCH_ICON: &str = "ui/icon_search.png";
const SEARCH_ICON_SIZE: f32 = 16.0 * KIT_SCALE;

const PLACEHOLDER: &str = "Search";
const CARET: &str = "_";
/// The pixel font has no `{ } ~ ^ \` or backtick; nothing longer is a name anyway.
const MAX_QUERY_LEN: usize = 24;
const MISSING_GLYPHS: [char; 5] = ['{', '}', '~', '^', '\\'];

const FIELD_TEXT_COLOR: Color = Color::srgb(0.96, 0.88, 0.7);
const PLACEHOLDER_COLOR: Color = Color::srgb(0.6, 0.55, 0.5);
const CHIP_TEXT_COLOR: Color = Color::srgb(0.96, 0.88, 0.7);
const EMPTY_CHIP_ALPHA: f32 = 0.4;

const SEARCH_KEY: KeyCode = KeyCode::KeyF;
const CTRL_KEYS: [KeyCode; 4] = [
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
];
const SHIFT_KEYS: [KeyCode; 2] = [KeyCode::ShiftLeft, KeyCode::ShiftRight];
const PREV_CHIP_KEY: KeyCode = KeyCode::BracketLeft;
const NEXT_CHIP_KEY: KeyCode = KeyCode::BracketRight;
/// LB / RB are the bag tabs (`hud::take_tab_keys`); LT / RT the chips.
const PREV_CHIP_PAD: GamepadButton = GamepadButton::LeftTrigger2;
const NEXT_CHIP_PAD: GamepadButton = GamepadButton::RightTrigger2;

/// A category chip, left to right.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Chip {
    #[default]
    All,
    Blocks,
    Backgrounds,
    Seeds,
    Wearables,
    Tools,
    Claims,
    Misc,
    /// The World Deed ring (docs/knowledge/ux/world-key.md): last, shown only while the player
    /// holds a deed. Its list replaces the grid rows (`deeds_ui::spawn_deed_rows`).
    Deeds,
}

impl Chip {
    const ALL: [Chip; 9] = [
        Chip::All,
        Chip::Blocks,
        Chip::Backgrounds,
        Chip::Seeds,
        Chip::Wearables,
        Chip::Tools,
        Chip::Claims,
        Chip::Misc,
        Chip::Deeds,
    ];

    fn label(self) -> &'static str {
        match self {
            Chip::All => "All",
            Chip::Blocks => "Blocks",
            Chip::Backgrounds => "Backgrounds",
            Chip::Seeds => "Seeds",
            Chip::Wearables => "Wearables",
            Chip::Tools => "Tools",
            Chip::Claims => "Claims",
            Chip::Misc => "Misc",
            Chip::Deeds => "Deeds",
        }
    }

    /// The chip an item kind belongs to (never `All`).
    fn of(kind: ItemKind) -> Chip {
        match kind {
            ItemKind::Block => Chip::Blocks,
            ItemKind::Background => Chip::Backgrounds,
            ItemKind::Seed => Chip::Seeds,
            ItemKind::Equip(_) => Chip::Wearables,
            ItemKind::Tool => Chip::Tools,
            ItemKind::Claim => Chip::Claims,
            ItemKind::Resource => Chip::Misc,
        }
    }

    fn shows(self, kind: ItemKind) -> bool {
        self == Chip::All || Chip::of(kind) == self
    }

    fn position(self) -> usize {
        Chip::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }
}

/// The grid's order, saved in the settings. Display only.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    /// As the slots lie (the hotbar's order).
    #[default]
    Slot,
    /// By chip (kind), and within a kind as the slots lie.
    Type,
    /// A-Z.
    Name,
    /// Most first.
    Count,
}

impl Sort {
    /// The value in `settings.txt`.
    pub fn key(self) -> &'static str {
        match self {
            Sort::Slot => "slot",
            Sort::Type => "type",
            Sort::Name => "name",
            Sort::Count => "count",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Sort::Slot, Sort::Type, Sort::Name, Sort::Count]
            .into_iter()
            .find(|sort| sort.key() == value)
    }

    fn next(self) -> Self {
        match self {
            Sort::Slot => Sort::Type,
            Sort::Type => Sort::Name,
            Sort::Name => Sort::Count,
            Sort::Count => Sort::Slot,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Slot => "Sort: Slot",
            Sort::Type => "Sort: Type",
            Sort::Name => "Sort: Name",
            Sort::Count => "Sort: Count",
        }
    }
}

/// Whether a chip is in the row: the Deeds chip only while the player holds at least one deed.
fn chip_shown(chip: Chip, deeds: usize) -> bool {
    chip != Chip::Deeds || deeds > 0
}

/// A chip's text: `Deeds (N)` counts the ring.
fn chip_text(chip: Chip, deeds: usize) -> String {
    match chip {
        Chip::Deeds => format!("{} ({deeds})", chip.label()),
        _ => chip.label().to_string(),
    }
}

/// Search and Sort show for the item chips only: the deed list has nothing to search or sort.
fn search_shown(chip: Chip) -> bool {
    chip != Chip::Deeds
}

/// Case-insensitive substring of the display name; an empty query matches everything.
fn search_matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.trim().to_lowercase())
}

/// One shown stack: its place in `hud::carried_slots`, the item and the count.
type Entry = (usize, ItemId, u16);

/// Puts `entries` in `sort` order. Ties fall back to name, then the slot order, so the grid never
/// shuffles between rebuilds.
fn sort_entries(entries: &mut [Entry], sort: Sort) {
    let name = |id: ItemId| item(id).map_or("", |def| def.name);
    let chip = |id: ItemId| {
        item(id)
            .map_or(Chip::Misc, |def| Chip::of(def.kind))
            .position()
    };
    match sort {
        Sort::Slot => entries.sort_by_key(|&(place, ..)| place),
        Sort::Type => entries.sort_by_key(|&(place, id, _)| (chip(id), place)),
        Sort::Name => entries.sort_by(|a, b| name(a.1).cmp(name(b.1)).then(a.0.cmp(&b.0))),
        Sort::Count => entries.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| name(a.1).cmp(name(b.1)))
                .then(a.0.cmp(&b.0))
        }),
    }
}

/// What the Inventory tab shows. The chip is remembered while the game runs; the sort is copied
/// from and to the settings.
#[derive(Resource, Default)]
pub struct InventoryView {
    pub chip: Chip,
    pub query: String,
    pub sort: Sort,
    /// The search field has the keyboard (`hud::TextFocus`, written in `claim_ui.rs`).
    pub focused: bool,
}

impl InventoryView {
    fn shows(&self, id: ItemId) -> bool {
        item(id)
            .is_some_and(|def| self.chip.shows(def.kind) && search_matches(def.name, &self.query))
    }

    /// The `carried` places to show in the grid, in order, and whether the free slots follow (only
    /// unfiltered). The first `hotbar` places are the hotbar row: never shown here, so never
    /// filtered or sorted.
    pub fn cells(
        &self,
        slots: &[(ItemId, u16)],
        carried: &[usize],
        hotbar: usize,
    ) -> (Vec<usize>, bool) {
        let mut entries: Vec<Entry> = carried
            .iter()
            .enumerate()
            .skip(hotbar)
            .filter_map(|(place, &index)| slots.get(index).map(|&(id, n)| (place, id, n)))
            .filter(|&(_, id, _)| self.shows(id))
            .collect();
        sort_entries(&mut entries, self.sort);
        let unfiltered = self.chip == Chip::All && self.query.trim().is_empty();
        (
            entries.into_iter().map(|(place, ..)| place).collect(),
            unfiltered,
        )
    }
}

#[derive(Component, Clone, Copy)]
struct ChipButton(Chip);
#[derive(Component)]
struct ChipLabel(Chip);
#[derive(Component)]
struct SearchField;
#[derive(Component)]
struct SearchText;
#[derive(Component)]
struct SortButton;
#[derive(Component)]
struct SortLabel;
#[derive(Component)]
struct CountLabel;
#[derive(Component)]
struct ClearLink;
/// The strip's Drop / Trash row, and its buttons.
#[derive(Component)]
struct StripButtons;
#[derive(Component, Clone, Copy)]
struct StripButton {
    trash: bool,
}

/// Keys the field and the chips took this frame, used in `Update`.
#[derive(Resource, Default)]
struct FilterKeys {
    chip_step: i32,
    focus: bool,
}

pub struct InventoryFilterPlugin;

impl Plugin for InventoryFilterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InventoryView>()
            .init_resource::<FilterKeys>()
            .add_systems(Startup, load_sort)
            // Before the bag tabs read `[` / `]`, and before the player walks on the letters.
            .add_systems(
                RunFixedMainLoop,
                take_filter_keys
                    .before(take_tab_keys)
                    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(Update, click_filter.in_set(Phase::Input))
            .add_systems(Update, (refresh_filter, refresh_strip).in_set(Phase::View));
    }
}

fn load_sort(settings: Res<Settings>, mut view: ResMut<InventoryView>) {
    view.sort = settings.inventory_sort;
}

fn chip_node(assets: &AssetServer, active: bool) -> (Node, ImageNode) {
    let file = if active {
        CHIP_ACTIVE_IMAGE
    } else {
        CHIP_IMAGE
    };
    (
        Node {
            height: px(CHIP_HEIGHT),
            padding: UiRect::horizontal(px(CHIP_PADDING)),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        sliced(assets, file, CHIP_INSET),
    )
}

/// The chip row and the search / sort / count row, under the bag tabs.
pub(crate) fn spawn_filter_bar(panel: &mut ChildSpawnerCommands, assets: &AssetServer) {
    // One row, so the bag keeps its rows on a 720 px window: chips past the bag's 8-column width
    // scroll sideways (trackpad / horizontal wheel) and are always reachable by Shift+[ ] / LT RT.
    let chips = Node {
        column_gap: px(SLOT_GAP),
        flex_shrink: 0.0,
        overflow: Overflow::scroll_x(),
        ..default()
    };
    panel
        .spawn((chips, ScrollArea, Hovered::default()))
        .with_children(|row| {
            for chip in Chip::ALL {
                row.spawn((
                    chip_node(assets, chip == Chip::All),
                    ChipButton(chip),
                    Hovered::default(),
                ))
                .with_child((
                    label(chip.label()),
                    TextColor(CHIP_TEXT_COLOR),
                    ChipLabel(chip),
                ));
            }
        });
    let bar = Node {
        align_items: AlignItems::Center,
        column_gap: px(HUD_MARGIN),
        flex_shrink: 0.0,
        ..default()
    };
    panel.spawn((bar, Hovered::default())).with_children(|bar| {
        let field = Node {
            width: px(FIELD_WIDTH),
            height: px(FIELD_HEIGHT),
            padding: UiRect::horizontal(px(FIELD_INSET * KIT_SCALE / 2.0)),
            column_gap: px(SLOT_GAP),
            align_items: AlignItems::Center,
            ..default()
        };
        bar.spawn((
            field,
            sliced(assets, FIELD_IMAGE, FIELD_INSET),
            SearchField,
            Hovered::default(),
        ))
        .with_children(|field| {
            field.spawn((
                ImageNode::new(assets.load(SEARCH_ICON)),
                Node {
                    width: px(SEARCH_ICON_SIZE),
                    height: px(SEARCH_ICON_SIZE),
                    ..default()
                },
            ));
            field.spawn((label(PLACEHOLDER), TextColor(PLACEHOLDER_COLOR), SearchText));
        });
        bar.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        bar.spawn((tab_node(assets, true), SortButton, Hovered::default()))
            .with_child((label(Sort::Slot.label()), SortLabel));
        bar.spawn((ink(""), CountLabel));
    });
}

/// What an empty grid says: the search that found nothing, else the empty chip.
fn empty_message(chip: Chip, query: &str) -> Option<String> {
    if !query.trim().is_empty() {
        Some(format!("No items match \"{}\"", query.trim()))
    } else if chip != Chip::All {
        Some(format!("No {} in your bag", chip.label()))
    } else {
        None
    }
}

/// `chip` moved `step` places, skipping chips without items (`All` always counts).
fn step_chip(chip: Chip, step: i32, has_items: impl Fn(Chip) -> bool) -> Chip {
    let len = Chip::ALL.len() as i64;
    let dir = i64::from(step.signum());
    let mut at = chip.position() as i64;
    for _ in 0..step.unsigned_abs() {
        for _ in 0..len {
            at = (at + dir).rem_euclid(len);
            let next = Chip::ALL[at as usize];
            if next == Chip::All || has_items(next) {
                break;
            }
        }
    }
    Chip::ALL[at as usize]
}

/// Which chips have something in `me`'s bag (the Deeds chip: on the ring).
fn chips_with_items(me: &Player) -> impl Fn(Chip) -> bool {
    let kinds: Vec<ItemKind> = me
        .inventory
        .slots()
        .iter()
        .filter_map(|&(id, _)| item(id).map(|def| def.kind))
        .collect();
    let deeds = !me.deeds.is_empty();
    move |chip| match chip {
        Chip::Deeds => deeds,
        _ => kinds.iter().any(|&kind| chip.shows(kind)),
    }
}

/// In an empty grid: `No items match "<text>"` with a `Clear` link, or `No <chip> in your bag`.
pub(crate) fn spawn_empty(grid: &mut ChildSpawnerCommands, view: &InventoryView) {
    let Some(message) = empty_message(view.chip, &view.query) else {
        return;
    };
    let search = !view.query.trim().is_empty();
    let row = Node {
        width: percent(100),
        column_gap: px(HUD_MARGIN),
        align_items: AlignItems::Center,
        ..default()
    };
    grid.spawn((row, Hovered::default())).with_children(|row| {
        row.spawn(ink(message));
        if !search {
            return;
        }
        let link = Node {
            min_height: px(CHIP_HEIGHT),
            align_items: AlignItems::Center,
            border: UiRect::bottom(px(1)),
            ..default()
        };
        row.spawn((link, ClearLink, Hovered::default()))
            .with_child(ink("Clear"));
    });
}

/// The strip's `Drop` / `Trash` buttons (shown while the inventory is open, for a droppable item).
pub(crate) fn spawn_strip_buttons(line: &mut ChildSpawnerCommands, assets: &AssetServer) {
    let node = Node {
        column_gap: px(SLOT_GAP),
        display: Display::None,
        ..default()
    };
    line.spawn((node, StripButtons)).with_children(|row| {
        for (trash, name) in [(false, "Drop"), (true, "Trash")] {
            row.spawn((
                tab_node(assets, true),
                StripButton { trash },
                Hovered::default(),
            ))
            .with_child(label(name));
        }
    });
}

fn typed_char(c: char) -> bool {
    (c.is_ascii_graphic() || c == ' ') && !MISSING_GLYPHS.contains(&c) && c != '`'
}

/// Ctrl+F, Shift+[ / Shift+], LT / RT, and typing while the field has focus (then nothing else
/// gets the keyboard: no walking, no chat, no hotbar). Esc clears the text, then gives focus back.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn take_filter_keys(
    open: Res<OpenPanel>,
    pads: Query<&Gamepad>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut view: ResMut<InventoryView>,
    mut out: ResMut<FilterKeys>,
) {
    let typed: Vec<KeyboardInput> = typed.read().cloned().collect();
    let ctrl = keys.any_pressed(CTRL_KEYS);
    if ctrl && keys.just_pressed(SEARCH_KEY) {
        out.focus = true;
        keys.reset(SEARCH_KEY);
        return;
    }
    if *open != OpenPanel::Inventory {
        return;
    }
    if !view.focused {
        let pad = |button| pads.iter().any(|pad| pad.just_pressed(button));
        let shift = keys.any_pressed(SHIFT_KEYS);
        let key = |code| shift && keys.just_pressed(code);
        out.chip_step += i32::from(key(NEXT_CHIP_KEY) || pad(NEXT_CHIP_PAD))
            - i32::from(key(PREV_CHIP_KEY) || pad(PREV_CHIP_PAD));
        if shift {
            for code in [PREV_CHIP_KEY, NEXT_CHIP_KEY] {
                keys.reset(code);
            }
        }
        return;
    }
    for key in typed.iter().filter(|k| k.state == ButtonState::Pressed) {
        if ctrl || !keys.pressed(key.key_code) {
            continue;
        }
        match &key.logical_key {
            Key::Escape if view.query.is_empty() => view.focused = false,
            Key::Escape => view.query.clear(),
            Key::Enter => view.focused = false,
            Key::Backspace => {
                view.query.pop();
            }
            Key::Space if view.query.len() < MAX_QUERY_LEN => view.query.push(' '),
            Key::Character(text) => {
                for c in text.chars().filter(|&c| typed_char(c)) {
                    if view.query.len() < MAX_QUERY_LEN {
                        view.query.push(c);
                    }
                }
            }
            _ => {}
        }
    }
    // The keyboard is the field's (the Esc that gave focus back too, so it doesn't close).
    keys.reset_all();
}

/// Clicks on chips, the field, the sort button, `Clear` and the strip buttons; the keys
/// `take_filter_keys` took. Focus leaves the field on a click elsewhere or when the panel closes.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn click_filter(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut open: ResMut<OpenPanel>,
    mut view: ResMut<InventoryView>,
    mut out: ResMut<FilterKeys>,
    mut settings: ResMut<Settings>,
    game: Res<Game>,
    chips: Query<(&ChipButton, &Hovered)>,
    field: Single<&Hovered, With<SearchField>>,
    sort: Single<&Hovered, With<SortButton>>,
    clear: Query<&Hovered, With<ClearLink>>,
    strip: Query<(&StripButton, &Hovered)>,
    mut presses: MessageWriter<DiscardPress>,
    mut sfx: MessageWriter<Sfx>,
) {
    if std::mem::take(&mut out.focus) {
        open.set_if_neq(OpenPanel::Inventory);
        view.focused = true;
        return;
    }
    // The last deed left the ring: its chip is gone, back to All.
    if view.chip == Chip::Deeds && game.me().is_some_and(|me| me.deeds.is_empty()) {
        view.chip = Chip::All;
    }
    if *open != OpenPanel::Inventory {
        if view.focused {
            view.focused = false;
        }
        return;
    }
    let Some(me) = game.me() else { return };
    let has_items = chips_with_items(me);
    let step = std::mem::take(&mut out.chip_step);
    if step != 0 {
        let chip = step_chip(view.chip, step, &has_items);
        if view.chip != chip {
            view.chip = chip;
            sfx.write(Sfx::UiClick);
        }
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let on_field = field.get();
    if view.focused != on_field {
        view.focused = on_field;
    }
    let mut clicked = on_field;
    // A greyed (empty) chip takes no click.
    let hovered_chip = chips
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(&ChipButton(chip), _)| chip)
        .filter(|&chip| chip == Chip::All || has_items(chip));
    if let Some(chip) = hovered_chip {
        if view.chip != chip {
            view.chip = chip;
        }
        clicked = true;
    }
    if sort.get() {
        view.sort = view.sort.next();
        settings.inventory_sort = view.sort;
        clicked = true;
    }
    if clear.iter().any(Hovered::get) {
        view.query.clear();
        view.chip = Chip::All;
        clicked = true;
    }
    if let Some((&StripButton { trash }, _)) = strip.iter().find(|(_, hovered)| hovered.get()) {
        let stack = keys.any_pressed(SHIFT_KEYS);
        presses.write(DiscardPress { trash, stack });
        clicked = true;
    }
    if clicked {
        sfx.write(Sfx::UiClick);
    }
}

/// Chip art (active, greyed when it has no items), the field's text, the sort label and the
/// `used/capacity` count follow the view and the inventory.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn refresh_filter(
    assets: Res<AssetServer>,
    game: Res<Game>,
    view: Res<InventoryView>,
    mut chips: Query<(&ChipButton, &mut ImageNode, &mut Node)>,
    mut chip_labels: Query<
        (&ChipLabel, &mut Text, &mut TextColor),
        (Without<SearchText>, Without<SortLabel>, Without<CountLabel>),
    >,
    mut search: Single<(&mut Text, &mut TextColor), With<SearchText>>,
    mut sort: Single<&mut Text, (With<SortLabel>, Without<SearchText>)>,
    mut count: Single<&mut Text, (With<CountLabel>, Without<SearchText>, Without<SortLabel>)>,
    mut search_sort: Query<
        &mut Node,
        (
            Or<(With<SearchField>, With<SortButton>)>,
            Without<ChipButton>,
        ),
    >,
) {
    let Some(me) = game.me() else { return };
    let slots = me.inventory.slots();
    let has_items = chips_with_items(me);
    let deeds = me.deeds.len();
    let display = if search_shown(view.chip) {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in &mut search_sort {
        if node.display != display {
            node.display = display;
        }
    }
    for (&ChipButton(chip), mut image, mut node) in &mut chips {
        let display = if chip_shown(chip, deeds) {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let file = if chip == view.chip {
            CHIP_ACTIVE_IMAGE
        } else {
            CHIP_IMAGE
        };
        let handle = assets.load(file);
        if image.image != handle {
            image.image = handle;
        }
        let tint = if has_items(chip) {
            Color::WHITE
        } else {
            INACTIVE_TAB_COLOR.with_alpha(EMPTY_CHIP_ALPHA)
        };
        if image.color != tint {
            image.color = tint;
        }
    }
    for (&ChipLabel(chip), mut text, mut colour) in &mut chip_labels {
        let want_text = chip_text(chip, deeds);
        if text.0 != want_text {
            text.0 = want_text;
        }
        let base = if chip == view.chip {
            INK_COLOR
        } else {
            CHIP_TEXT_COLOR
        };
        let want = if has_items(chip) {
            base
        } else {
            base.with_alpha(EMPTY_CHIP_ALPHA)
        };
        if colour.0 != want {
            colour.0 = want;
        }
    }
    let (text, colour) = match (view.query.is_empty(), view.focused) {
        (true, false) => (PLACEHOLDER.to_string(), PLACEHOLDER_COLOR),
        (_, true) => (format!("{}{CARET}", view.query), FIELD_TEXT_COLOR),
        (false, false) => (view.query.clone(), FIELD_TEXT_COLOR),
    };
    let (search_text, search_colour) = &mut *search;
    if search_text.0 != text {
        search_text.0 = text;
    }
    if search_colour.0 != colour {
        search_colour.0 = colour;
    }
    if sort.0 != view.sort.label() {
        sort.0 = view.sort.label().to_string();
    }
    let used = format!("{}/{}", slots.len(), me.inventory.capacity);
    if count.0 != used {
        count.0 = used;
    }
}

/// The strip's Drop / Trash buttons: while the inventory is open, for an item that may leave.
fn refresh_strip(
    game: Res<Game>,
    open: Res<OpenPanel>,
    selected: Res<Selected>,
    wardrobe: Res<WardrobeSelected>,
    mut row: Single<&mut Node, With<StripButtons>>,
) {
    let target = discard_target(&game, *open, &selected, &wardrobe);
    let shown = *open == OpenPanel::Inventory && target.is_some_and(discardable);
    let display = if shown { Display::Flex } else { Display::None };
    if row.display != display {
        row.display = display;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{DIRT, FIST, ITEMS, Slot};

    #[test]
    fn chips_map_from_item_kinds() {
        assert_eq!(Chip::of(ItemKind::Block), Chip::Blocks);
        assert_eq!(Chip::of(ItemKind::Background), Chip::Backgrounds);
        assert_eq!(Chip::of(ItemKind::Seed), Chip::Seeds);
        assert_eq!(Chip::of(ItemKind::Equip(Slot::Hat)), Chip::Wearables);
        assert_eq!(Chip::of(ItemKind::Tool), Chip::Tools);
        assert_eq!(Chip::of(ItemKind::Claim), Chip::Claims);
        assert_eq!(Chip::of(ItemKind::Resource), Chip::Misc);
        // All shows every kind; any other chip only its own.
        for def in ITEMS {
            assert!(Chip::All.shows(def.kind));
            assert!(Chip::of(def.kind).shows(def.kind));
        }
        assert!(!Chip::Seeds.shows(ItemKind::Block));
        let labels: Vec<_> = Chip::ALL.iter().map(|c| c.label()).collect();
        assert_eq!(
            labels,
            [
                "All",
                "Blocks",
                "Backgrounds",
                "Seeds",
                "Wearables",
                "Tools",
                "Claims",
                "Misc",
                "Deeds"
            ]
        );
        // No item is a deed: deeds live on the ring, not in the slots.
        assert!(ITEMS.iter().all(|def| !Chip::Deeds.shows(def.kind)));
    }

    #[test]
    fn the_deeds_chip_shows_only_with_a_deed_and_counts_them() {
        assert!(!chip_shown(Chip::Deeds, 0));
        assert!(chip_shown(Chip::Deeds, 1));
        assert!(chip_shown(Chip::Misc, 0), "item chips always show");
        assert_eq!(chip_text(Chip::Deeds, 2), "Deeds (2)");
        assert_eq!(chip_text(Chip::Misc, 2), "Misc");
        assert_eq!(Chip::ALL.last(), Some(&Chip::Deeds), "the last chip");
        assert!(
            !search_shown(Chip::Deeds),
            "no Search / Sort over the deed list"
        );
        assert!(search_shown(Chip::All) && search_shown(Chip::Misc));
        // Stepping skips it while the ring is empty.
        assert_eq!(step_chip(Chip::Misc, 1, |c| c == Chip::Misc), Chip::All);
        assert_eq!(
            step_chip(Chip::Misc, 1, |c| matches!(c, Chip::Misc | Chip::Deeds)),
            Chip::Deeds
        );
    }

    #[test]
    fn search_is_a_case_insensitive_substring() {
        assert!(search_matches("dirt seed", "SEED"));
        assert!(search_matches("dirt seed", "Dirt S"));
        assert!(search_matches("dirt seed", ""));
        assert!(search_matches("dirt seed", "  dirt "));
        assert!(!search_matches("dirt seed", "stone"));
        assert!(typed_char('a') && typed_char('#') && typed_char(' '));
        assert!(!typed_char('{') && !typed_char('`') && !typed_char('é'));
    }

    #[test]
    fn sort_orders_and_ties() {
        let fist = item(FIST).unwrap();
        let dirt = item(DIRT).unwrap();
        let seed = ITEMS.iter().find(|d| d.kind == ItemKind::Seed).unwrap();
        // (place, item, count): a tool, a seed, a block, and the block again (a tie on everything
        // but the place).
        let start = [
            (0, fist.id, 1),
            (1, seed.id, 9),
            (2, dirt.id, 9),
            (3, dirt.id, 4),
        ];
        let order = |sort| {
            let mut entries = start;
            sort_entries(&mut entries, sort);
            entries.map(|(place, ..)| place)
        };
        // Slot: as the slots lie.
        assert_eq!(order(Sort::Slot), [0, 1, 2, 3]);
        // Type: chip order (Blocks, Seeds, Tools), slot order inside a chip.
        assert_eq!(order(Sort::Type), [2, 3, 1, 0]);
        // Name: A-Z, the same name by slot order.
        let mut by_name = start;
        by_name.sort_by(|a, b| {
            let n = |id| item(id).unwrap().name;
            n(a.1).cmp(n(b.1)).then(a.0.cmp(&b.0))
        });
        assert_eq!(order(Sort::Name), by_name.map(|(place, ..)| place));
        // Count: most first; 9 and 9 by name, then the 4, then the 1.
        let nine = if dirt.name < seed.name {
            [2, 1]
        } else {
            [1, 2]
        };
        assert_eq!(order(Sort::Count), [nine[0], nine[1], 3, 0]);
    }

    #[test]
    fn empty_chips_are_skipped_and_an_empty_grid_says_why() {
        let has = |chip: Chip| matches!(chip, Chip::Blocks | Chip::Tools);
        assert_eq!(step_chip(Chip::All, 1, has), Chip::Blocks);
        assert_eq!(step_chip(Chip::Blocks, 1, has), Chip::Tools);
        assert_eq!(
            step_chip(Chip::Tools, 1, has),
            Chip::All,
            "wraps past the empty ones"
        );
        assert_eq!(step_chip(Chip::All, -1, has), Chip::Tools);
        assert_eq!(step_chip(Chip::All, 2, has), Chip::Tools);
        assert_eq!(step_chip(Chip::Seeds, 1, |_| false), Chip::All);
        assert_eq!(
            empty_message(Chip::Seeds, ""),
            Some("No Seeds in your bag".to_string())
        );
        assert_eq!(
            empty_message(Chip::Seeds, " stone "),
            Some("No items match \"stone\"".to_string())
        );
        assert_eq!(empty_message(Chip::All, "  "), None);
    }

    #[test]
    fn the_hotbar_row_is_never_filtered_or_sorted() {
        let seeds: Vec<ItemId> = ITEMS
            .iter()
            .filter(|d| d.kind == ItemKind::Seed)
            .map(|d| d.id)
            .take(2)
            .collect();
        let (seed, other_seed) = (seeds[0], seeds[1]);
        // Hotbar: dirt, a seed. Grid: the fist, another seed.
        let slots = [(DIRT, 3), (seed, 2), (FIST, 1), (other_seed, 9)];
        let carried: Vec<usize> = (0..slots.len()).collect();
        let hotbar = 2;
        let mut view = InventoryView::default();
        let (all, free) = view.cells(&slots, &carried, hotbar);
        assert!(free, "unfiltered: the free slots follow");
        assert!(all.iter().all(|&p| p >= hotbar), "{all:?}");
        view.chip = Chip::Seeds;
        let (seeds, free) = view.cells(&slots, &carried, hotbar);
        assert!(!free);
        assert!(seeds.iter().all(|&p| p >= hotbar), "{seeds:?}");
        assert_eq!(seeds, vec![3], "the hotbar's seed is not in the grid");
        view.chip = Chip::All;
        view.sort = Sort::Count;
        let (sorted, _) = view.cells(&slots, &carried, hotbar);
        assert_eq!(
            sorted,
            vec![3, 2],
            "most first, the hotbar's 3 dirt left out"
        );
    }

    #[test]
    fn sort_cycles_and_round_trips_through_its_key() {
        assert_eq!(
            Sort::default(),
            Sort::Slot,
            "today's slot order is the default"
        );
        assert_eq!(Sort::Slot.next(), Sort::Type);
        assert_eq!(Sort::Type.next(), Sort::Name);
        assert_eq!(Sort::Name.next(), Sort::Count);
        assert_eq!(Sort::Count.next(), Sort::Slot);
        for sort in [Sort::Slot, Sort::Type, Sort::Name, Sort::Count] {
            assert_eq!(Sort::parse(sort.key()), Some(sort));
        }
        assert_eq!(Sort::parse("sideways"), None);
        assert_eq!(Sort::Count.label(), "Sort: Count");
    }
}
