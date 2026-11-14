//! The Almanac (docs/knowledge/ux/almanac.md): an in-game reference book. J toggles it, F1 opens
//! it on Basics. Every recipe and item page is generated from the shared tables (`items::ITEMS`,
//! `farming::RECIPES`, `crafting::RECIPES`, `claims::CLAIMS`, `equip_price`, `mods`), so it can't
//! go stale; only the Basics text is written here. Every item name on a page links to its page.
//! The page is rebuilt only when the page, the search text or the open panel changes.

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_shared::claims::claim_kind;
use game_shared::crafting::{self, Recipe};
use game_shared::farming::{self, grow_seconds};
use game_shared::items::{
    Collision, EMPTY, ITEMS, ItemDef, ItemId, ItemKind, STARTER_OUTFIT, Slot, WORKBENCH,
    equip_price, item, seed_of, seed_parent,
};
use game_shared::mods::mods_of;
use game_shared::worldgen::GENERATED;

use crate::hud::{
    CRAFT_PANEL_KEY, FIT_PANEL_TOP, FitWindow, HUD_MARGIN, INK_COLOR, OpenPanel, SEP, SLOT_GAP,
    duration_text, ink, key_name, kind_line, label, scroll_grid, sliced, spawn_centre_panel,
    spawn_mods, tab_node, title_case,
};
use crate::settings::BINDINGS;
use crate::state::Phase;
use crate::world_view::item_icon;

/// Toggles the almanac. Shown in the top-bar tooltip (`hud.rs`).
pub(crate) const ALMANAC_KEY: KeyCode = KeyCode::KeyJ;
/// Opens it on Basics.
const BASICS_KEY: KeyCode = KeyCode::F1;
const BACK_KEY: KeyCode = KeyCode::Backspace;
const HISTORY: usize = 20;
const MAX_QUERY: usize = 30;

const WIDTH: f32 = 900.0;
const SECTIONS_WIDTH: f32 = 170.0;
const LINE_ICON: f32 = 24.0;
const HEADER_ICON: f32 = 48.0;
const FIELD_IMAGE: &str = "ui/lobby/input.png";
const FIELD_INSET: f32 = 8.0;
const FIELD_HEIGHT: f32 = 40.0;
const LINK_COLOR: Color = Color::srgb(0.13, 0.32, 0.55);
const LINK_HOVER_COLOR: Color = Color::srgb(0.55, 0.2, 0.08);
const LIGHT_TEXT: Color = Color::srgb(0.96, 0.88, 0.7);
const MUTED_TEXT: Color = Color::srgb(0.55, 0.5, 0.45);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Basics,
    Basic(usize),
    Splicing,
    Crafting,
    Items,
    Controls,
    Item(ItemId),
    Search,
}

/// The section list, top to bottom.
const SECTIONS: [(Page, &str); 5] = [
    (Page::Basics, "Basics"),
    (Page::Splicing, "Splicing"),
    (Page::Crafting, "Crafting"),
    (Page::Items, "Items"),
    (Page::Controls, "Controls"),
];

#[derive(Resource)]
struct Almanac {
    page: Page,
    back: Vec<Page>,
    query: String,
    /// The search field has the keyboard.
    focused: bool,
}

impl Almanac {
    fn go(&mut self, page: Page) {
        if page != self.page {
            if self.back.len() == HISTORY {
                self.back.remove(0);
            }
            self.back.push(self.page);
            self.page = page;
        }
    }

    fn go_back(&mut self) {
        if let Some(page) = self.back.pop() {
            self.page = page;
        }
    }
}

/// One piece of a page line.
#[derive(Clone, PartialEq, Debug)]
enum Piece {
    Text(String),
    /// Smaller, muted text.
    Note(String),
    Link(String, Page),
    /// An item's icon and name, linking to its page.
    Item(ItemId),
    /// The item's mods with their icons.
    Mods(ItemId),
    /// Back to the previous page.
    Back,
}

type Line = Vec<Piece>;

#[derive(Component)]
struct Root;
#[derive(Component)]
struct LinkTo(Page);
#[derive(Component)]
struct SearchField;

pub struct AlmanacPlugin;

impl Plugin for AlmanacPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Almanac {
            page: Page::Basics,
            back: Vec::new(),
            query: String::new(),
            focused: false,
        })
        .add_systems(Startup, spawn_panel)
        .add_systems(PreUpdate, type_search.after(InputSystems))
        .add_systems(Update, (keys, clicks).in_set(Phase::Input))
        .add_systems(Update, (rebuild, hover_links).in_set(Phase::View));
    }
}

fn spawn_panel(mut commands: Commands, assets: Res<AssetServer>) {
    let root = Node {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        min_height: px(0),
        row_gap: px(HUD_MARGIN),
        ..default()
    };
    let panel = spawn_centre_panel(&mut commands, &assets, OpenPanel::Almanac, WIDTH, |p| {
        p.spawn((root, Root, Hovered::default()));
    });
    commands
        .entity(panel)
        .insert(FitWindow { top: FIT_PANEL_TOP });
}

/// While the search field has the keyboard: typing, Backspace, Enter (open the first result) and
/// Esc (clear, then let go). The keyboard is then cleared so nothing else reacts (the same trick
/// as `chat.rs`); Enter is kept from the chat line by `TextFocus` (`claim_ui.rs`).
fn type_search(
    open: Res<OpenPanel>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut almanac: ResMut<Almanac>,
) {
    if *open != OpenPanel::Almanac || !almanac.focused {
        typed.clear();
        return;
    }
    let mut query = almanac.query.clone();
    let mut enter = false;
    let mut escape = false;
    for key in typed.read().filter(|k| k.state == ButtonState::Pressed) {
        match &key.logical_key {
            Key::Character(s) => query.extend(s.chars().filter(|c| !c.is_control())),
            Key::Space => query.push(' '),
            Key::Backspace => {
                query.pop();
            }
            Key::Enter => enter = true,
            Key::Escape => escape = true,
            _ => {}
        }
    }
    keys.reset_all();
    query = query.chars().take(MAX_QUERY).collect();
    if escape {
        if query.is_empty() {
            almanac.focused = false;
        }
        query.clear();
    }
    if enter {
        if let Some(first) = search(&query)
            .into_iter()
            .find_map(|line| first_link(&line))
        {
            query.clear();
            almanac.focused = false;
            set_query(&mut almanac, query);
            almanac.go(first);
            return;
        }
    }
    set_query(&mut almanac, query);
}

/// Writes the query (only when it changed) and moves to or away from the Search page.
fn set_query(almanac: &mut Almanac, query: String) {
    if almanac.query == query {
        return;
    }
    let searching = !query.is_empty();
    almanac.query = query;
    if searching && almanac.page != Page::Search {
        almanac.go(Page::Search);
    } else if !searching && almanac.page == Page::Search {
        almanac.go_back();
    }
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut open: ResMut<OpenPanel>,
    mut almanac: ResMut<Almanac>,
) {
    if keys.just_pressed(BASICS_KEY) {
        open.set_if_neq(OpenPanel::Almanac);
        almanac.go(Page::Basics);
    } else if keys.just_pressed(ALMANAC_KEY) {
        *open = if *open == OpenPanel::Almanac {
            OpenPanel::None
        } else {
            OpenPanel::Almanac
        };
    }
    if *open != OpenPanel::Almanac {
        if almanac.focused {
            almanac.focused = false;
        }
        return;
    }
    if keys.just_pressed(BACK_KEY) || mouse.just_pressed(MouseButton::Back) {
        almanac.go_back();
    }
}

fn clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    open: Res<OpenPanel>,
    links: Query<(&LinkTo, &Hovered)>,
    back: Query<&Hovered, With<BackLink>>,
    field: Query<&Hovered, With<SearchField>>,
    mut almanac: ResMut<Almanac>,
) {
    if *open != OpenPanel::Almanac || !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let on_field = field.iter().any(Hovered::get);
    if almanac.focused != on_field {
        almanac.focused = on_field;
    }
    if back.iter().any(Hovered::get) {
        almanac.go_back();
    } else if let Some((LinkTo(page), _)) = links.iter().find(|(_, h)| h.get()) {
        let page = *page;
        if almanac.page == Page::Search {
            // Leaving the results: the search is done, and Back skips it.
            almanac.query.clear();
            almanac.go_back();
        }
        almanac.go(page);
    }
}

type LinkText = (Or<(With<LinkTo>, With<BackLink>)>, Changed<Hovered>);

fn hover_links(mut links: Query<(&Hovered, &mut TextColor), LinkText>) {
    for (hovered, mut colour) in &mut links {
        colour.0 = if hovered.get() {
            LINK_HOVER_COLOR
        } else {
            LINK_COLOR
        };
    }
}

fn rebuild(
    mut commands: Commands,
    assets: Res<AssetServer>,
    open: Res<OpenPanel>,
    almanac: Res<Almanac>,
    root: Single<Entity, With<Root>>,
) {
    if *open != OpenPanel::Almanac || !(open.is_changed() || almanac.is_changed()) {
        return;
    }
    let lines = page_lines(almanac.page, &almanac.query);
    let section = section_of(almanac.page, almanac.back.last().copied());
    let field = Node {
        height: px(FIELD_HEIGHT),
        padding: UiRect::horizontal(px(2.0 * FIELD_INSET)),
        align_items: AlignItems::Center,
        ..default()
    };
    let field_text = match (almanac.query.is_empty(), almanac.focused) {
        (true, false) => "Search items, recipes and basics".to_string(),
        (_, true) => format!("{}_", almanac.query),
        (false, false) => almanac.query.clone(),
    };
    let field_colour = if almanac.query.is_empty() && !almanac.focused {
        MUTED_TEXT
    } else {
        LIGHT_TEXT
    };
    commands
        .entity(*root)
        .despawn_children()
        .with_children(|root| {
            root.spawn((
                field,
                sliced(&assets, FIELD_IMAGE, FIELD_INSET),
                SearchField,
                Hovered::default(),
            ))
            .with_child((label(field_text), TextColor(field_colour)));
            let body = Node {
                flex_grow: 1.0,
                min_height: px(0),
                column_gap: px(2.0 * HUD_MARGIN),
                ..default()
            };
            root.spawn((body, Hovered::default()))
                .with_children(|body| {
                    let list = Node {
                        width: px(SECTIONS_WIDTH),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(SLOT_GAP),
                        ..default()
                    };
                    body.spawn(list).with_children(|list| {
                        for (page, name) in SECTIONS {
                            let (mut node, image) = tab_node(&assets, Some(page) == section);
                            node.width = percent(100);
                            list.spawn((node, image, LinkTo(page), Hovered::default()))
                                .with_child((label(name), TextColor(LIGHT_TEXT)));
                        }
                    });
                    let column = Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        min_height: px(0),
                        ..default()
                    };
                    body.spawn(column).with_children(|column| {
                        let page = scroll_grid(column, ());
                        column.commands().entity(page).with_children(|page| {
                            if !almanac.back.is_empty() {
                                spawn_line(page, &assets, &[Piece::Back]);
                            }
                            for line in &lines {
                                spawn_line(page, &assets, line);
                            }
                        });
                    });
                });
        });
}

/// The highlighted section: the page's own, or for an item page or the results, where it came
/// from.
fn section_of(page: Page, previous: Option<Page>) -> Option<Page> {
    match page {
        Page::Basic(_) => Some(Page::Basics),
        Page::Item(_) => Some(Page::Items),
        Page::Search => previous.and_then(|p| section_of(p, None)),
        page => Some(page),
    }
}

fn spawn_line(parent: &mut ChildSpawnerCommands, assets: &AssetServer, line: &[Piece]) {
    let node = Node {
        width: percent(100),
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        column_gap: px(6.0),
        row_gap: px(2.0),
        margin: UiRect::bottom(px(SLOT_GAP)),
        ..default()
    };
    parent.spawn(node).with_children(|row| {
        for piece in line {
            match piece {
                Piece::Text(text) => {
                    row.spawn(ink(text.clone()));
                }
                Piece::Note(text) => {
                    row.spawn((label(text.clone()), TextColor(MUTED_TEXT)));
                }
                Piece::Back => {
                    row.spawn((
                        label("Back"),
                        TextColor(LINK_COLOR),
                        BackLink,
                        Hovered::default(),
                    ));
                }
                Piece::Link(text, page) => {
                    row.spawn((
                        label(text.clone()),
                        TextColor(LINK_COLOR),
                        LinkTo(*page),
                        Hovered::default(),
                    ));
                }
                Piece::Item(id) => {
                    let size = if line.len() == 2 && matches!(line[1], Piece::Note(_)) {
                        HEADER_ICON
                    } else {
                        LINE_ICON
                    };
                    spawn_item(row, assets, *id, size);
                }
                Piece::Mods(id) => spawn_mods(row, assets, *id, INK_COLOR),
            }
        }
    });
}

#[derive(Component)]
struct BackLink;

/// An item's icon and name, both linking to its page.
fn spawn_item(row: &mut ChildSpawnerCommands, assets: &AssetServer, id: ItemId, size: f32) {
    let Some(def) = item(id) else { return };
    row.spawn(Node {
        column_gap: px(SLOT_GAP),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|entry| {
        entry.spawn((
            item_icon(assets, def.sprite).node(),
            Node {
                width: px(size),
                height: px(size),
                ..default()
            },
            LinkTo(Page::Item(id)),
            Hovered::default(),
        ));
        entry.spawn((
            label(name(id)),
            TextColor(LINK_COLOR),
            LinkTo(Page::Item(id)),
            Hovered::default(),
        ));
    });
}

// --- Page content: pure functions of the shared tables ---------------------------------------

fn name(id: ItemId) -> String {
    item(id).map_or_else(|| "?".to_string(), |def| title_case(def.name))
}

/// The keys of a `settings::BINDINGS` action, as the Keys tab shows them.
fn keys_for(action: &str) -> &'static str {
    BINDINGS
        .iter()
        .find(|(a, _)| *a == action)
        .map_or("?", |(_, keys)| keys)
}

/// Hand-written Basics: title, text, and the page it points to.
fn basics() -> Vec<(&'static str, String, Option<(&'static str, Page)>)> {
    let craft = key_name(CRAFT_PANEL_KEY);
    vec![
        (
            "Moving",
            format!(
                "Walk with {}. Jump with {}; hold it to jump higher. You can stand on blocks and platforms.",
                keys_for("Move"),
                keys_for("Jump"),
            ),
            Some(("All controls", Page::Controls)),
        ),
        (
            "Punching",
            format!(
                "Hold {} on a block in reach to hit it until it breaks. A broken block drops itself, its seed or nothing, and sometimes gems. Walk over a drop to pick it up.",
                keys_for("Punch / place"),
            ),
            Some(("All items", Page::Items)),
        ),
        (
            "Placing",
            format!(
                "Pick a block in the hotbar ({}) and click an empty tile in reach. Backgrounds sit behind you and never block the way.",
                keys_for("Select hotbar slot"),
            ),
            None,
        ),
        (
            "Planting",
            "Pick a seed and click a free tile with solid ground below it. The tree grows on its own, even while you are away. Rarer items grow slower.".to_string(),
            Some(("Grow times", Page::Splicing)),
        ),
        (
            "Harvesting",
            "Punch a fully grown tree to harvest it. It gives a few of its block and sometimes a seed.".to_string(),
            None,
        ),
        (
            "Splicing",
            "Plant a second, different seed on a young tree. If the two seeds make a known pair, the tree turns into a new item's tree.".to_string(),
            Some(("All splices", Page::Splicing)),
        ),
        (
            "Claims",
            format!(
                "Place a claim to protect the tiles around it: only you and your friends can build there. Use the wrench on your claim to add friends or make it public. Claims are sold in the store ({}).",
                keys_for("Store"),
            ),
            Some(("Claim items", Page::Item(game_shared::items::SMALL_CLAIM))),
        ),
        (
            "Trading",
            "Trade with another player to swap items safely. Both sides lock their offer, look it over, then confirm.".to_string(),
            None,
        ),
        (
            "Respawning",
            "Touching lava sends you back to the world's main door. Your items stay with you.".to_string(),
            None,
        ),
        (
            "Ladders & platforms",
            "Climb a ladder with W and S. Jump up through a platform and stand on it; press S on one to drop down.".to_string(),
            None,
        ),
        (
            "Crafting",
            format!(
                "Crafting turns items into new ones. Some recipes work anywhere, others need a Workbench nearby. Press {craft} to open the craft panel."
            ),
            Some(("All recipes", Page::Crafting)),
        ),
    ]
}

fn title(text: &str) -> Line {
    vec![Piece::Text(text.to_string())]
}

fn page_lines(page: Page, query: &str) -> Vec<Line> {
    match page {
        Page::Basics => {
            let mut lines = vec![title("Basics")];
            lines.extend(
                basics()
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, ..))| vec![Piece::Link(name.to_string(), Page::Basic(i))]),
            );
            lines
        }
        Page::Basic(i) => {
            let Some((name, text, link)) = basics().into_iter().nth(i) else {
                return Vec::new();
            };
            let mut lines = vec![title(name), vec![Piece::Text(text)]];
            if let Some((label, page)) = link {
                lines.push(vec![Piece::Link(label.to_string(), page)]);
            }
            lines
        }
        Page::Splicing => {
            let mut lines = vec![title(
                "Splicing: plant the second seed on a young tree of the first",
            )];
            lines.extend(splices().into_iter().map(splice_line));
            lines
        }
        Page::Crafting => {
            let mut lines = vec![title("Crafting")];
            lines.extend(crafting::RECIPES.iter().map(craft_line));
            lines
        }
        Page::Items => {
            let mut lines = vec![title("Items")];
            let mut kind = None;
            for def in listed_items() {
                let group = kind_name(def.kind);
                if kind != Some(group) {
                    kind = Some(group);
                    lines.push(vec![Piece::Note(group.to_string())]);
                }
                lines.push(vec![Piece::Item(def.id)]);
            }
            lines
        }
        Page::Controls => {
            let mut lines = vec![title("Controls")];
            lines.extend(BINDINGS.iter().map(|(action, keys)| {
                vec![
                    Piece::Text(format!("{action}:")),
                    Piece::Note(keys.to_string()),
                ]
            }));
            lines.push(vec![
                Piece::Text("Almanac:".to_string()),
                Piece::Note(format!(
                    "{} or {}",
                    key_name(ALMANAC_KEY),
                    key_name(BASICS_KEY)
                )),
            ]);
            lines
        }
        Page::Item(id) => item_lines(id),
        Page::Search => {
            let results = search(query);
            if results.is_empty() {
                vec![vec![Piece::Note(format!("Nothing found for \"{query}\""))]]
            } else {
                results
            }
        }
    }
}

/// Every item a player can meet, grouped by kind (the order of `kind_rank`).
fn listed_items() -> Vec<&'static ItemDef> {
    let mut items: Vec<_> = ITEMS.iter().filter(|def| def.id != EMPTY).collect();
    items.sort_by_key(|def| (kind_rank(def.kind), def.id.0));
    items
}

fn kind_rank(kind: ItemKind) -> u8 {
    match kind {
        ItemKind::Block => 0,
        ItemKind::Background => 1,
        ItemKind::Seed => 2,
        ItemKind::Resource => 3,
        ItemKind::Claim => 4,
        ItemKind::Tool => 5,
        ItemKind::Equip(_) => 6,
    }
}

fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Block => "Block",
        ItemKind::Background => "Background",
        ItemKind::Seed => "Seed",
        ItemKind::Resource => "Resource",
        ItemKind::Claim => "Claim",
        ItemKind::Tool => "Tool",
        ItemKind::Equip(Slot::Back) => "Wings",
        ItemKind::Equip(Slot::Hand) => "Pickaxe",
        ItemKind::Equip(_) => "Clothing",
    }
}

/// `farming::RECIPES`, rarest result last.
fn splices() -> Vec<(ItemId, ItemId, ItemId)> {
    let mut all = farming::RECIPES.to_vec();
    all.sort_by_key(|&(_, _, c)| item(c).map_or(0, |def| def.rarity));
    all
}

fn splice_line((a, b, c): (ItemId, ItemId, ItemId)) -> Line {
    let mut line = vec![
        Piece::Item(a),
        Piece::Text("+".into()),
        Piece::Item(b),
        Piece::Text("=".into()),
        Piece::Item(c),
    ];
    if let Some(def) = item(c) {
        line.push(Piece::Note(format!(
            "grows in {}{SEP}rarity {}",
            duration_text(grow_seconds(def.rarity)),
            def.rarity
        )));
    }
    line
}

fn station_name(recipe: &Recipe) -> String {
    recipe.station.map_or_else(|| "Anywhere".to_string(), name)
}

fn craft_line(recipe: &Recipe) -> Line {
    let mut line = vec![Piece::Note(format!("{}:", station_name(recipe)))];
    for (i, &(id, count)) in recipe.inputs.iter().enumerate() {
        if i > 0 {
            line.push(Piece::Text("+".into()));
        }
        line.push(Piece::Text(count.to_string()));
        line.push(Piece::Item(id));
    }
    line.push(Piece::Text("=".into()));
    let (out, count) = recipe.output;
    if count > 1 {
        line.push(Piece::Text(count.to_string()));
    }
    line.push(Piece::Item(out));
    line
}

/// Every true way to get `id`, from the shared tables.
fn sources(id: ItemId) -> Vec<Piece> {
    let mut out = Vec::new();
    if GENERATED.contains(&id) {
        out.push(Piece::Text("Break it in any world".into()));
    }
    if let Some(seed) = seed_of(id) {
        out.push(Piece::Link("Farm its seed".into(), Page::Item(seed)));
    }
    if let Some(parent) = seed_parent(id) {
        out.push(Piece::Text("Break or harvest".into()));
        out.push(Piece::Item(parent));
    }
    for &(a, b, c) in farming::RECIPES {
        if c == id {
            out.push(Piece::Link(
                format!("Splice {} + {}", name(a), name(b)),
                Page::Splicing,
            ));
        }
    }
    for recipe in crafting::RECIPES {
        if recipe.output.0 == id {
            let at = match recipe.station {
                Some(station) => format!("Craft at a {}", name(station)),
                None => "Craft anywhere".to_string(),
            };
            out.push(Piece::Link(at, Page::Crafting));
        }
    }
    let price = claim_kind(id)
        .map(|kind| kind.price)
        .or_else(|| equip_price(id));
    if let Some(price) = price {
        out.push(Piece::Text(format!("Store{SEP}{price} gems")));
    }
    if STARTER_OUTFIT.contains(&id) {
        out.push(Piece::Text("Starter outfit".into()));
    }
    out
}

/// Recipes that take `id` as an input, as links to what they make.
fn uses(id: ItemId) -> Vec<Piece> {
    let mut out = Vec::new();
    for &(a, b, c) in farming::RECIPES {
        if a == id || b == id {
            out.push(Piece::Item(c));
        }
    }
    for recipe in crafting::RECIPES {
        if recipe.inputs.iter().any(|&(input, _)| input == id) {
            out.push(Piece::Item(recipe.output.0));
        }
    }
    out
}

/// A rule worth one line, where the item has one beyond its tooltip line.
fn rule(def: &ItemDef) -> Option<String> {
    match def.collision {
        Collision::Hazard => return Some("Touching it sends you home".to_string()),
        Collision::Platform => {
            return Some("Jump up through it and stand on it; press S to drop down".to_string());
        }
        _ => {}
    }
    if def.id == WORKBENCH {
        return Some("Recipes marked Workbench need one within reach".to_string());
    }
    kind_line(def, false)
}

fn item_lines(id: ItemId) -> Vec<Line> {
    let Some(def) = item(id) else {
        return Vec::new();
    };
    let mut lines = vec![vec![
        Piece::Item(id),
        Piece::Note(format!("{}{SEP}rarity {}", kind_name(def.kind), def.rarity)),
    ]];
    let mut labelled = |title: &str, mut pieces: Vec<Piece>| {
        if !pieces.is_empty() {
            pieces.insert(0, Piece::Text(format!("{title}:")));
            lines.push(pieces);
        }
    };
    labelled("How to get", sources(id));
    labelled("Used in", uses(id));
    if let Some(seed) = seed_of(id).and_then(item) {
        labelled(
            "Seed",
            vec![
                Piece::Item(seed.id),
                Piece::Note(format!(
                    "grows in {}",
                    duration_text(grow_seconds(def.rarity))
                )),
            ],
        );
    }
    if let Some(parent) = seed_parent(id) {
        labelled("Grows", vec![Piece::Item(parent)]);
    }
    if mods_of(id).next().is_some() {
        labelled("Mods", vec![Piece::Mods(id)]);
    }
    if let Some(rule) = rule(def) {
        lines.push(vec![Piece::Note(rule)]);
    }
    lines
}

/// Live, case-insensitive results, grouped Items · Recipes · Basics.
fn search(query: &str) -> Vec<Line> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let hit = |text: &str| text.to_lowercase().contains(&q);
    let items: Vec<Line> = listed_items()
        .into_iter()
        .filter(|def| hit(def.name))
        .map(|def| vec![Piece::Item(def.id)])
        .collect();
    let mut recipes: Vec<Line> = splices()
        .into_iter()
        .filter(|&(a, b, c)| [a, b, c].iter().any(|&id| hit(&name(id))))
        .map(splice_line)
        .collect();
    recipes.extend(
        crafting::RECIPES
            .iter()
            .filter(|r| hit(&name(r.output.0)) || r.inputs.iter().any(|&(id, _)| hit(&name(id))))
            .map(craft_line),
    );
    let basics: Vec<Line> = basics()
        .into_iter()
        .enumerate()
        .filter(|(_, (title, text, _))| hit(title) || hit(text))
        .map(|(i, (title, ..))| vec![Piece::Link(title.to_string(), Page::Basic(i))])
        .collect();
    let mut lines = Vec::new();
    for (group, found) in [("Items", items), ("Recipes", recipes), ("Basics", basics)] {
        if !found.is_empty() {
            lines.push(vec![Piece::Note(group.to_string())]);
            lines.extend(found);
        }
    }
    lines
}

/// The page the first link of a line goes to (Enter in the search field).
fn first_link(line: &Line) -> Option<Page> {
    line.iter().find_map(|piece| match piece {
        Piece::Link(_, page) => Some(*page),
        Piece::Item(id) => Some(Page::Item(*id)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::PLANKS;

    #[test]
    fn every_splice_result_lists_its_splice_as_a_source() {
        for &(a, b, c) in farming::RECIPES {
            let text = format!("Splice {} + {}", name(a), name(b));
            assert!(
                sources(c).contains(&Piece::Link(text, Page::Splicing)),
                "{c:?}"
            );
        }
    }

    #[test]
    fn every_craft_input_lists_the_output_under_used_in() {
        for recipe in crafting::RECIPES {
            for &(input, _) in recipe.inputs {
                assert!(uses(input).contains(&Piece::Item(recipe.output.0)));
            }
        }
    }

    #[test]
    fn items_page_lists_every_item_but_empty() {
        let lines = page_lines(Page::Items, "");
        let listed = lines
            .iter()
            .filter(|line| matches!(line.as_slice(), [Piece::Item(_)]))
            .count();
        assert_eq!(listed, ITEMS.iter().filter(|def| def.id != EMPTY).count());
    }

    #[test]
    fn search_finds_items_recipes_and_basics_case_insensitively() {
        let results = search("PLANK");
        assert_eq!(results.first(), Some(&vec![Piece::Note("Items".into())]));
        assert!(results.contains(&vec![Piece::Item(PLANKS)]));
        assert!(
            results
                .iter()
                .any(|l| l.first() == Some(&Piece::Note("Recipes".into())))
        );
        assert!(
            search("lava")
                .iter()
                .any(|l| first_link(l) == Some(Page::Basic(8)))
        );
        assert!(search("zzzz").is_empty());
    }

    #[test]
    fn history_keeps_the_last_twenty_pages() {
        let mut almanac = Almanac {
            page: Page::Basics,
            back: Vec::new(),
            query: String::new(),
            focused: false,
        };
        for i in 0..30 {
            almanac.go(Page::Basic(i));
        }
        assert_eq!(almanac.back.len(), HISTORY);
        almanac.go_back();
        assert_eq!(almanac.page, Page::Basic(28));
    }
}
