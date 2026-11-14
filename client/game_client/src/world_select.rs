//! The World menu (docs/knowledge/ux/world-select.md): a name field with a server-backed status
//! line, Go / Create, Recent (settings), Active now (server) and Random world. Opened by the main
//! door (`interact.rs`), the top-bar World button, M, and a refused join (`net::JoinRefused`).
//! Entering a world restarts the client on it.

use bevy::app::AppExit;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_protocol::{WORLD_CAPACITY, WorldStatus, world_name_allowed};
use game_shared::worldgen::MAX_WORLD_NAME_LEN;

use crate::audio::Sfx;
use crate::chat::{OpenWorldMenu, WarpTo};
use crate::hud::{
    HUD_MARGIN, INACTIVE_TAB_COLOR, KIT_SCALE, OpenPanel, SLOT_GAP, TOOLTIP_IMAGE, TOOLTIP_INSET,
    TextFocus, ink, key_name, label, sliced, spawn_centre_panel, tab_node,
};
use crate::net::{AUTH_TOKEN_ENV, JoinRefused, Net, Refusal};
use crate::settings::Settings;
use crate::state::{ActionMsg, Game, Phase};

/// Opens and closes the menu (free: see the bindings in hud, interact, player, camera, chat).
pub(crate) const WORLD_KEY: KeyCode = KeyCode::KeyM;
const CLOSE_KEY: KeyCode = KeyCode::Escape;

// Re-exec: the variables the new process reads (net.rs).
const WORLD_ENV: &str = "GAME_WORLD";
/// The world to fall back to when the join is refused (net.rs).
const PREV_WORLD_ENV: &str = "GAME_PREV_WORLD";
/// Set when entering a world the menu said was new: shows the claim hint once.
const NEW_WORLD_ENV: &str = "GAME_NEW_WORLD";

const PANEL_WIDTH: f32 = 640.0;
const FIELD_IMAGE: &str = "ui/lobby/input.png";
const FIELD_INSET: f32 = 8.0;
const FIELD_HEIGHT: f32 = 44.0;
const ROW_HEIGHT: f32 = 44.0;
const FIELD_TEXT_COLOR: Color = Color::srgb(0.96, 0.88, 0.7);
const PLACEHOLDER_COLOR: Color = Color::srgb(0.6, 0.55, 0.5);
const FULL_ROW_ALPHA: f32 = 0.4;
const DISABLED_GO_ALPHA: f32 = 0.5;

const PLACEHOLDER: &str = "Enter world name";
const CARET: &str = "_";
/// The status lookup waits this long after typing stops (lookups are rate-limited).
const LOOKUP_DEBOUNCE_SECS: f32 = 0.3;
/// No answer by then: the server is unreachable.
const LOOKUP_TIMEOUT_SECS: f32 = 3.0;
/// The fresh token for the restart: waited for this long, then the current one is used.
const TOKEN_TIMEOUT_SECS: f32 = 2.0;
/// Go's label while the token is on its way.
const SWITCHING_TEXT: &str = "Switching...";
pub(crate) const MAX_RECENT: usize = 5;
/// Random world: an active one with this many players (not empty, not nearly full).
const RANDOM_PLAYERS: std::ops::RangeInclusive<u8> = 1..=60;

const UNREACHABLE_TEXT: &str = "Couldn't reach the server. Try again";
const NOT_ALLOWED_TEXT: &str = "That name isn't allowed";

const HINT_SECS: f32 = 10.0;
const HINT_TOP: f32 = 120.0;
const HINT_PADDING: f32 = 12.0;
/// The store and almanac keys, for the hint text (hud.rs `STORE_KEY`, almanac.rs).
const STORE_HINT_KEY: KeyCode = KeyCode::KeyB;

/// The field: upper-case letters and digits only, at most `MAX_WORLD_NAME_LEN`.
fn typed_name(field: &str, typed: &str) -> String {
    let mut name = field.to_string();
    for c in typed.chars().filter(char::is_ascii_alphanumeric) {
        if name.len() < MAX_WORLD_NAME_LEN {
            name.push(c.to_ascii_uppercase());
        }
    }
    name
}

/// The status line for `name`.
fn status_text(name: &str, status: WorldStatus) -> String {
    match status {
        WorldStatus::Open { players: 1 } => format!("{name} · 1 player"),
        WorldStatus::Open { players } => format!("{name} · {players} players"),
        WorldStatus::New => format!("{name} doesn't exist yet: entering it creates it"),
        WorldStatus::Full => format!("{name} is full ({WORLD_CAPACITY}/{WORLD_CAPACITY})"),
        WorldStatus::NotAllowed => NOT_ALLOWED_TEXT.to_string(),
        WorldStatus::LimitReached => limit_text(),
    }
}

fn limit_text() -> String {
    format!(
        "You've opened {} new worlds this hour. Visit an existing one or try again later",
        game_protocol::NEW_WORLDS_PER_HOUR
    )
}

/// Why entering `name` failed (the toast), or `None` when it may go.
fn refusal_text(name: &str, status: WorldStatus) -> Option<String> {
    match status {
        WorldStatus::Open { .. } | WorldStatus::New => None,
        WorldStatus::Full => Some(format!(
            "{name} is full ({WORLD_CAPACITY}/{WORLD_CAPACITY}). Try another world"
        )),
        WorldStatus::NotAllowed => Some(NOT_ALLOWED_TEXT.to_string()),
        WorldStatus::LimitReached => Some(limit_text()),
    }
}

/// `Go`, or `Create` for a new name.
fn go_label(status: Option<WorldStatus>) -> &'static str {
    if status == Some(WorldStatus::New) {
        "Create"
    } else {
        "Go"
    }
}

/// Recent worlds after entering `world`: newest first, no repeats, at most `MAX_RECENT`.
pub(crate) fn push_recent(recent: &[String], world: &str) -> Vec<String> {
    let rest = recent.iter().filter(|name| *name != world).cloned();
    std::iter::once(world.to_string())
        .chain(rest)
        .take(MAX_RECENT)
        .collect()
}

/// A random active world with `RANDOM_PLAYERS` players, other than the current one. `roll` in 0..1.
fn random_world(active: &[(String, u8)], current: &str, roll: f32) -> Option<String> {
    let pool: Vec<&String> = active
        .iter()
        .filter(|(name, players)| RANDOM_PLAYERS.contains(players) && name != current)
        .map(|(name, _)| name)
        .collect();
    let index =
        ((roll.clamp(0.0, 0.999_999) * pool.len() as f32) as usize).min(pool.len().checked_sub(1)?);
    Some(pool[index].clone())
}

/// What a Recent row says about `name`'s player count.
fn recent_count(name: &str, current: (&str, usize), active: &[(String, u8)]) -> String {
    if name == current.0 {
        return match current.1 {
            0 | 1 => "just you".to_string(),
            n => format!("{n}/{WORLD_CAPACITY}"),
        };
    }
    match active.iter().find(|(world, _)| world == name) {
        Some((_, players)) if *players > 0 => format!("{players}/{WORLD_CAPACITY}"),
        _ => "empty".to_string(),
    }
}

/// The variables the restarted client gets on top of the current ones (which carry the server
/// and the rest through): the world, where to fall back to, whether it is new (`None` = removed),
/// and the fresh token when there is one (none or empty, a dev login: the current env stays).
fn world_env(
    world: &str,
    current: &str,
    new: bool,
    token: Option<&str>,
) -> Vec<(&'static str, Option<String>)> {
    let mut env = vec![
        (WORLD_ENV, Some(world.to_string())),
        (PREV_WORLD_ENV, Some(current.to_string())),
        (NEW_WORLD_ENV, new.then(|| "1".to_string())),
    ];
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        env.push((AUTH_TOKEN_ENV, Some(token.to_string())));
    }
    env
}

/// Restarts the client on `world`: starts the new process, then ends this one. Not `exec`:
/// replacing the image skips the QUIC close, so the server keeps the account online until its idle
/// timeout and refuses the new process ("That account is already online"). The child must start
/// before the exit: on macOS winit ends the whole process when its event loop stops (lobby.rs).
// ponytail: re-start; swap the world mirror in place later (Leave + Join on the same connection).
fn enter_world(
    world: &str,
    current: &str,
    new: bool,
    token: Option<&str>,
    exit: &mut MessageWriter<AppExit>,
) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let env = world_env(world, current, new, token);
    // The keys only: the values hold the login token.
    let keys: Vec<_> = env.iter().map(|(key, _)| *key).collect();
    info!("restarting on world {world}: {exe:?} {args:?}, setting {keys:?}");
    let mut command = std::process::Command::new(exe);
    command.args(args);
    for (key, value) in env {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    // The other `GAME_*` variables are inherited as they are.
    command.spawn()?;
    exit.write(AppExit::Success);
    Ok(())
}

/// The menu's state while the game runs.
#[derive(Resource, Default)]
struct WorldMenu {
    field: String,
    focused: bool,
    /// When the field last changed (`Time::elapsed_secs`), until its lookup is sent.
    edited_at: Option<f32>,
    /// The name asked about and when.
    asked: Option<(String, f32)>,
    /// The newest answer: (name, status).
    status: Option<(String, WorldStatus)>,
    active: Vec<(String, u8)>,
    /// A refusal or failure to show in the status line instead.
    error: Option<String>,
    /// Going: (world, new, when the fresh token was asked for), until it comes or times out.
    switching: Option<(String, bool, f32)>,
    /// `/warp <name>` (the name is in `field`): when its status was asked for, until it answers.
    warp: Option<f32>,
}

impl WorldMenu {
    /// The status of the name in the field, if known.
    fn field_status(&self) -> Option<WorldStatus> {
        if !self.field.is_empty() && !world_name_allowed(&self.field) {
            return Some(WorldStatus::NotAllowed);
        }
        self.status
            .as_ref()
            .filter(|(name, _)| *name == self.field)
            .map(|&(_, status)| status)
    }
}

#[derive(Component)]
struct NameField;
#[derive(Component)]
struct NameText;
#[derive(Component)]
struct GoButton;
#[derive(Component)]
struct GoLabel;
#[derive(Component)]
struct StatusLine;
#[derive(Component)]
struct RecentList;
#[derive(Component)]
struct ActiveList;
#[derive(Component)]
struct RandomButton;
/// A row that enters its world; `false` when full (greyed, no click).
#[derive(Component)]
struct WorldRow(String, bool);
#[derive(Component)]
struct NewWorldHint {
    until: f32,
}

pub struct WorldSelectPlugin;

impl Plugin for WorldSelectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldMenu>()
            .add_systems(Startup, (spawn_menu, remember_world, spawn_hint))
            .add_systems(
                RunFixedMainLoop,
                take_menu_keys.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(
                Update,
                (toggle_menu, on_refused, on_warp, click_menu)
                    .chain()
                    .in_set(Phase::Input),
            )
            .add_systems(
                Update,
                (talk_to_server, refresh_menu, expire_hint)
                    .chain()
                    .in_set(Phase::View),
            );
        #[cfg(debug_assertions)]
        app.add_systems(Update, debug_warp.before(Phase::Input));
    }
}

/// Debug builds: `GAME_DEBUG_WARP=NAME` sends `/warp NAME` once, 5 s after start, to test the
/// world switch without typing. The restarted game inherits it and ignores it (already there).
#[cfg(debug_assertions)]
fn debug_warp(time: Res<Time>, mut done: Local<bool>, mut warps: MessageWriter<WarpTo>) {
    if *done || time.elapsed_secs() < 5.0 {
        return;
    }
    *done = true;
    if let Ok(name) = std::env::var("GAME_DEBUG_WARP") {
        warps.write(WarpTo(name.to_ascii_uppercase()));
    }
}

fn spawn_menu(mut commands: Commands, assets: Res<AssetServer>) {
    spawn_centre_panel(
        &mut commands,
        &assets,
        OpenPanel::WorldSelect,
        PANEL_WIDTH,
        |panel| {
            panel.spawn(ink("Worlds"));
            let bar = Node {
                column_gap: px(HUD_MARGIN),
                align_items: AlignItems::Center,
                ..default()
            };
            panel.spawn((bar, Hovered::default())).with_children(|bar| {
                let field = Node {
                    flex_grow: 1.0,
                    height: px(FIELD_HEIGHT),
                    padding: UiRect::horizontal(px(FIELD_INSET * KIT_SCALE / 2.0)),
                    align_items: AlignItems::Center,
                    ..default()
                };
                bar.spawn((
                    field,
                    sliced(&assets, FIELD_IMAGE, FIELD_INSET),
                    NameField,
                    Hovered::default(),
                ))
                .with_child((
                    label(PLACEHOLDER),
                    TextColor(PLACEHOLDER_COLOR),
                    NameText,
                ));
                bar.spawn((tab_node(&assets, true), GoButton, Hovered::default()))
                    .with_child((label("Go"), GoLabel));
            });
            panel.spawn((ink(""), StatusLine));
            let columns = Node {
                column_gap: px(2.0 * HUD_MARGIN),
                ..default()
            };
            let column = || Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                flex_basis: px(0),
                row_gap: px(SLOT_GAP),
                ..default()
            };
            panel
                .spawn((columns, Hovered::default()))
                .with_children(|columns| {
                    columns.spawn(column()).with_children(|recent| {
                        recent.spawn(ink("Recent"));
                        recent.spawn((column(), RecentList));
                    });
                    columns.spawn(column()).with_children(|active| {
                        active.spawn(ink("Active now"));
                        active.spawn((column(), ActiveList));
                        active
                            .spawn((tab_node(&assets, true), RandomButton, Hovered::default()))
                            .with_child(label("Random world"));
                    });
                });
        },
    );
}

/// The world we are in goes to the front of Recent.
fn remember_world(game: Res<Game>, mut settings: ResMut<Settings>) {
    let name = &game.0.world.name;
    let recent = push_recent(&settings.recent_worlds, name);
    if settings.recent_worlds != recent {
        settings.recent_worlds = recent;
    }
}

/// Entering a world the menu said was new: one hint, gone after `HINT_SECS` or on any action.
fn spawn_hint(mut commands: Commands, assets: Res<AssetServer>, time: Res<Time>) {
    if std::env::var(NEW_WORLD_ENV).as_deref() != Ok("1") {
        return;
    }
    let text = format!(
        "This world is new. Buy a World Claim ({}) to make it yours · Learn more ({})",
        key_name(STORE_HINT_KEY),
        key_name(crate::almanac::ALMANAC_KEY)
    );
    let node = Node {
        position_type: PositionType::Absolute,
        top: px(HINT_TOP),
        left: percent(50),
        max_width: px(PANEL_WIDTH),
        margin: UiRect::left(px(-PANEL_WIDTH / 2.0)),
        padding: UiRect::all(px(HINT_PADDING)),
        ..default()
    };
    commands
        .spawn((
            node,
            sliced(&assets, TOOLTIP_IMAGE, TOOLTIP_INSET),
            Hovered::default(),
            NewWorldHint {
                until: time.elapsed_secs() + HINT_SECS,
            },
        ))
        .with_child(ink(text));
}

fn expire_hint(
    mut commands: Commands,
    time: Res<Time>,
    mut actions: MessageReader<ActionMsg>,
    hints: Query<(Entity, &NewWorldHint)>,
) {
    let acted = actions.read().count() > 0;
    for (entity, hint) in &hints {
        if acted || time.elapsed_secs() >= hint.until {
            commands.entity(entity).despawn();
        }
    }
}

/// Typing into the focused field (letters and digits, upper-cased); Enter goes, Esc closes. The
/// keyboard is the field's while the menu is open with it focused.
fn take_menu_keys(
    open: Res<OpenPanel>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut menu: ResMut<WorldMenu>,
) {
    let typed: Vec<KeyboardInput> = typed.read().cloned().collect();
    if *open != OpenPanel::WorldSelect || !menu.focused {
        return;
    }
    for key in typed.iter().filter(|k| k.state == ButtonState::Pressed) {
        if !keys.pressed(key.key_code) {
            continue;
        }
        match &key.logical_key {
            Key::Backspace => {
                menu.field.pop();
                menu.edited_at = Some(f32::NAN);
            }
            Key::Character(text) => {
                let name = typed_name(&menu.field, text);
                if name != menu.field {
                    menu.field = name;
                    menu.edited_at = Some(f32::NAN);
                }
            }
            _ => {}
        }
    }
    // Enter and Esc stay for `click_menu` / hud's Esc (closes); everything else is the field's.
    let keep: Vec<KeyCode> = [KeyCode::Enter, KeyCode::NumpadEnter, CLOSE_KEY]
        .into_iter()
        .filter(|&code| keys.just_pressed(code))
        .collect();
    keys.reset_all();
    for code in keep {
        keys.press(code);
    }
}

/// M opens and closes the menu (not while a text field has the keyboard); opening focuses the field.
fn toggle_menu(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<TextFocus>,
    mut open: ResMut<OpenPanel>,
    mut menu: ResMut<WorldMenu>,
) {
    if keys.just_pressed(WORLD_KEY) && !focus.0 {
        *open = if *open == OpenPanel::WorldSelect {
            OpenPanel::None
        } else {
            OpenPanel::WorldSelect
        };
    }
    // A refusal shown on opening (`on_refused`, `/warp`) stays until the menu closes or the name changes.
    if open.is_changed() {
        if *open == OpenPanel::WorldSelect {
            menu.focused = true;
        } else {
            menu.error = None;
        }
    }
}

/// A join the server refused (the client fell back to the previous world): say why, open the menu.
fn on_refused(
    mut commands: Commands,
    refused: Option<Res<JoinRefused>>,
    mut open: ResMut<OpenPanel>,
    mut menu: ResMut<WorldMenu>,
    mut notices: MessageWriter<crate::hud::Notice>,
    mut sfx: MessageWriter<Sfx>,
) {
    let Some(refused) = refused else { return };
    let text = match &refused.reason {
        Refusal::Status(status) => refusal_text(&refused.name, *status)
            .unwrap_or_else(|| status_text(&refused.name, *status)),
        Refusal::Kicked(reason) => reason.clone(),
    };
    notices.write(crate::hud::Notice(text.clone()));
    sfx.write(Sfx::Denied);
    menu.field = refused.name.clone();
    menu.error = Some(text);
    open.set_if_neq(OpenPanel::WorldSelect);
    commands.remove_resource::<JoinRefused>();
}

/// `/warp <name>` does what Go does for that name: its status first (`talk_to_server` goes on, or
/// opens the menu with the refusal); offline it goes at once. A bare `/warp` opens the menu.
fn on_warp(
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    game: Res<Game>,
    net: Option<ResMut<Net>>,
    mut warps: MessageReader<WarpTo>,
    mut opens: MessageReader<OpenWorldMenu>,
    mut open: ResMut<OpenPanel>,
    mut menu: ResMut<WorldMenu>,
) {
    if opens.read().count() > 0 {
        open.set_if_neq(OpenPanel::WorldSelect);
    }
    let Some(WarpTo(name)) = warps.read().last() else {
        return;
    };
    if menu.switching.is_some() {
        return;
    }
    menu.field = name.clone();
    menu.error = None;
    menu.edited_at = None;
    let current = &game.0.world.name;
    if *name == *current {
        return;
    }
    match net {
        Some(mut net) => {
            net.world_status(name.clone());
            menu.asked = None;
            menu.warp = Some(time.elapsed_secs());
        }
        None => {
            if let Err(error) = enter_world(name, current, false, None, &mut exit) {
                warn!("could not restart on world {name}: {error}");
            }
        }
    }
}

/// Go / Create, rows, Random, Enter; a click on the field focuses it.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn click_menu(
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    net: Option<ResMut<Net>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    game: Res<Game>,
    mut open: ResMut<OpenPanel>,
    mut menu: ResMut<WorldMenu>,
    field: Single<&Hovered, With<NameField>>,
    go: Single<&Hovered, With<GoButton>>,
    random: Single<&Hovered, With<RandomButton>>,
    rows: Query<(&WorldRow, &Hovered)>,
    mut sfx: MessageWriter<Sfx>,
) {
    if *open != OpenPanel::WorldSelect {
        menu.focused = false;
        return;
    }
    if menu.switching.is_some() {
        return;
    }
    let click = mouse.just_pressed(MouseButton::Left);
    if click {
        menu.focused = field.get();
    }
    let current = game.0.world.name.clone();
    let enter_pressed =
        menu.focused && keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
    let target = if (click && go.get()) || enter_pressed {
        let status = menu.field_status();
        match status.and_then(|s| refusal_text(&menu.field, s)) {
            Some(text) => {
                menu.error = Some(text);
                sfx.write(Sfx::Denied);
                None
            }
            None if menu.field.is_empty() => None,
            None => Some((menu.field.clone(), status == Some(WorldStatus::New))),
        }
    } else if click && random.get() {
        random_world(&menu.active, &current, rand_roll()).map(|name| (name, false))
    } else if click {
        rows.iter()
            .find(|(row, hovered)| hovered.get() && row.1)
            .map(|(row, _)| (row.0.clone(), false))
    } else {
        None
    };
    let Some((world, new)) = target else { return };
    sfx.write(Sfx::UiClick);
    if world == current {
        *open = OpenPanel::None;
        return;
    }
    // Online: a fresh token first (`talk_to_server` restarts when it comes); offline: at once.
    match net {
        Some(mut net) => {
            net.refresh_token();
            menu.switching = Some((world, new, time.elapsed_secs()));
        }
        None => {
            if let Err(error) = enter_world(&world, &current, new, None, &mut exit) {
                warn!("could not restart on world {world}: {error}");
                menu.error = Some(UNREACHABLE_TEXT.to_string());
            }
        }
    }
}

/// 0..1 from the clock: good enough to pick a random world.
fn rand_roll() -> f32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    (nanos % 1_000_000) as f32 / 1_000_000.0
}

/// Asks for the world list when the menu opens and the field's status `LOOKUP_DEBOUNCE_SECS` after
/// typing stops; takes the answers.
fn talk_to_server(
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    game: Res<Game>,
    mut open: ResMut<OpenPanel>,
    net: Option<ResMut<Net>>,
    mut menu: ResMut<WorldMenu>,
) {
    let Some(mut net) = net else { return };
    let now = time.elapsed_secs();
    if let Some((world, new, asked)) = menu.switching.clone() {
        let token = net.take_token();
        if token.is_some() || now - asked >= TOKEN_TIMEOUT_SECS {
            menu.switching = None;
            let current = &game.0.world.name;
            match enter_world(&world, current, new, token.as_deref(), &mut exit) {
                // Free the account now: the new process logs in with it (net.rs retries briefly).
                Ok(()) => net.close(),
                Err(error) => {
                    warn!("could not restart on world {world}: {error}");
                    menu.error = Some(UNREACHABLE_TEXT.to_string());
                }
            }
        }
    }
    if open.is_changed() && *open == OpenPanel::WorldSelect {
        net.list_worlds();
    }
    if let Some(worlds) = net.take_worlds() {
        menu.active = worlds;
    }
    if let Some(answer) = net.take_world_status() {
        // A `/warp` waiting on this name: go, or show why not in the menu.
        if menu.warp.is_some() && answer.0 == menu.field {
            menu.warp = None;
            match refusal_text(&answer.0, answer.1) {
                Some(text) => {
                    menu.error = Some(text);
                    open.set_if_neq(OpenPanel::WorldSelect);
                }
                None => {
                    net.refresh_token();
                    let new = answer.1 == WorldStatus::New;
                    menu.switching = Some((answer.0.clone(), new, now));
                }
            }
        }
        menu.status = Some(answer);
        menu.asked = None;
    }
    // A field edit stamps NaN (the key system has no clock); start the wait here.
    if menu.edited_at.is_some_and(f32::is_nan) {
        menu.edited_at = Some(now);
        menu.error = None;
    }
    let due = menu
        .edited_at
        .is_some_and(|at| now - at >= LOOKUP_DEBOUNCE_SECS);
    if due {
        menu.edited_at = None;
        if !menu.field.is_empty() && world_name_allowed(&menu.field) {
            net.world_status(menu.field.clone());
            menu.asked = Some((menu.field.clone(), now));
        }
    }
    if menu.warp.is_some_and(|at| now - at >= LOOKUP_TIMEOUT_SECS) {
        menu.warp = None;
        menu.error = Some(UNREACHABLE_TEXT.to_string());
        open.set_if_neq(OpenPanel::WorldSelect);
    }
    let late = menu
        .asked
        .as_ref()
        .is_some_and(|(_, at)| now - at >= LOOKUP_TIMEOUT_SECS);
    if late {
        menu.asked = None;
        menu.error = Some(UNREACHABLE_TEXT.to_string());
    }
}

/// The field, Go's label and state, the status line and the two lists follow the menu.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn refresh_menu(
    mut commands: Commands,
    game: Res<Game>,
    settings: Res<Settings>,
    menu: Res<WorldMenu>,
    mut name: Single<(&mut Text, &mut TextColor), With<NameText>>,
    mut go_label_text: Single<&mut Text, (With<GoLabel>, Without<NameText>)>,
    mut go_art: Single<&mut ImageNode, With<GoButton>>,
    mut status: Single<&mut Text, (With<StatusLine>, Without<NameText>, Without<GoLabel>)>,
    recent_list: Single<Entity, With<RecentList>>,
    active_list: Single<Entity, With<ActiveList>>,
    mut random: Single<&mut Node, With<RandomButton>>,
    mut shown: Local<Option<(Vec<String>, Vec<(String, u8)>, usize)>>,
) {
    if !menu.is_changed() && !settings.is_changed() && !game.is_changed() {
        return;
    }
    let (text, colour) = match (menu.field.is_empty(), menu.focused) {
        (true, false) => (PLACEHOLDER.to_string(), PLACEHOLDER_COLOR),
        (_, true) => (format!("{}{CARET}", menu.field), FIELD_TEXT_COLOR),
        (false, false) => (menu.field.clone(), FIELD_TEXT_COLOR),
    };
    let (name_text, name_colour) = &mut *name;
    if name_text.0 != text {
        name_text.0 = text;
    }
    if name_colour.0 != colour {
        name_colour.0 = colour;
    }
    let field_status = menu.field_status();
    let label_text = if menu.switching.is_some() {
        SWITCHING_TEXT
    } else {
        go_label(field_status)
    };
    if go_label_text.0 != label_text {
        go_label_text.0 = label_text.to_string();
    }
    let disabled = menu.field.is_empty()
        || field_status.is_some_and(|s| refusal_text(&menu.field, s).is_some());
    let tint = if disabled {
        INACTIVE_TAB_COLOR.with_alpha(DISABLED_GO_ALPHA)
    } else {
        Color::WHITE
    };
    if go_art.color != tint {
        go_art.color = tint;
    }
    let line = match (&menu.error, field_status) {
        (Some(error), _) => error.clone(),
        (None, Some(s)) => status_text(&menu.field, s),
        (None, None) => String::new(),
    };
    if status.0 != line {
        status.0 = line;
    }
    let current = game.0.world.name.as_str();
    let players = game.0.players.len();
    let random_shown = random_world(&menu.active, current, 0.0).is_some();
    let display = if random_shown {
        Display::Flex
    } else {
        Display::None
    };
    if random.display != display {
        random.display = display;
    }
    let view = (settings.recent_worlds.clone(), menu.active.clone(), players);
    if shown.as_ref() == Some(&view) {
        return;
    }
    let row = || Node {
        min_height: px(ROW_HEIGHT),
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        ..default()
    };
    commands
        .entity(*recent_list)
        .despawn_children()
        .with_children(|list| {
            for world in &view.0 {
                let count = recent_count(world, (current, players), &view.1);
                list.spawn((row(), WorldRow(world.clone(), true), Hovered::default()))
                    .with_children(|row| {
                        row.spawn(ink(world.clone()));
                        row.spawn(ink(count));
                    });
            }
        });
    commands
        .entity(*active_list)
        .despawn_children()
        .with_children(|list| {
            for (world, players) in &view.1 {
                let open = *players < WORLD_CAPACITY;
                let alpha = if open { 1.0 } else { FULL_ROW_ALPHA };
                list.spawn((row(), WorldRow(world.clone(), open), Hovered::default()))
                    .with_children(|row| {
                        let colour = crate::hud::INK_COLOR.with_alpha(alpha);
                        row.spawn((label(world.clone()), TextColor(colour)));
                        row.spawn((
                            label(format!("{players}/{WORLD_CAPACITY}")),
                            TextColor(colour),
                        ));
                    });
            }
        });
    *shown = Some(view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_upper_cases_and_takes_letters_and_digits_only() {
        assert_eq!(typed_name("", "myFarm 2!"), "MYFARM2");
        let long = typed_name("", &"a".repeat(40));
        assert_eq!(long.len(), MAX_WORLD_NAME_LEN);
        assert_eq!(typed_name("AB", "é-c"), "ABC");
        let field_status = |field: &str| {
            WorldMenu {
                field: field.into(),
                ..default()
            }
            .field_status()
        };
        assert_eq!(
            field_status("ADMIN"),
            Some(WorldStatus::NotAllowed),
            "reserved"
        );
        assert_eq!(
            field_status("START"),
            None,
            "the hub is a world like any other"
        );
    }

    #[test]
    fn statuses_read_as_the_spec_says() {
        let open = WorldStatus::Open { players: 12 };
        assert_eq!(status_text("START", open), "START · 12 players");
        assert_eq!(
            status_text("MYFARM", WorldStatus::New),
            "MYFARM doesn't exist yet: entering it creates it"
        );
        assert_eq!(
            status_text("START", WorldStatus::Full),
            "START is full (64/64)"
        );
        assert_eq!(
            status_text("X", WorldStatus::NotAllowed),
            "That name isn't allowed"
        );
        assert_eq!(
            refusal_text("START", WorldStatus::Full).as_deref(),
            Some("START is full (64/64). Try another world")
        );
        assert_eq!(
            refusal_text("A", WorldStatus::LimitReached).as_deref(),
            Some("You've opened 10 new worlds this hour. Visit an existing one or try again later")
        );
        assert_eq!(refusal_text("A", open), None);
        assert_eq!(go_label(Some(WorldStatus::New)), "Create");
        assert_eq!(go_label(Some(open)), "Go");
        assert_eq!(go_label(None), "Go");
    }

    #[test]
    fn recent_keeps_five_newest_first_without_repeats() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let recent = names(&["A", "B", "C", "D", "E"]);
        assert_eq!(push_recent(&recent, "F"), names(&["F", "A", "B", "C", "D"]));
        assert_eq!(push_recent(&recent, "C"), names(&["C", "A", "B", "D", "E"]));
        assert_eq!(push_recent(&[], "A"), names(&["A"]));
        let active = vec![("B".to_string(), 3)];
        assert_eq!(recent_count("A", ("A", 1), &active), "just you");
        assert_eq!(recent_count("B", ("A", 1), &active), "3/64");
        assert_eq!(recent_count("C", ("A", 1), &active), "empty");
    }

    #[test]
    fn random_picks_a_world_with_1_to_60_players_other_than_ours() {
        let active = vec![
            ("EMPTY".to_string(), 0),
            ("BUSY".to_string(), 61),
            ("HERE".to_string(), 5),
            ("OK".to_string(), 30),
            ("ALSO".to_string(), 60),
        ];
        for roll in [0.0, 0.3, 0.6, 0.99, 1.0] {
            let pick = random_world(&active, "HERE", roll).unwrap();
            assert!(pick == "OK" || pick == "ALSO", "{pick}");
        }
        assert_eq!(random_world(&active, "HERE", 0.0).as_deref(), Some("OK"));
        assert_eq!(random_world(&active, "HERE", 0.99).as_deref(), Some("ALSO"));
        assert_eq!(random_world(&active[..3], "HERE", 0.5), None);
    }

    #[test]
    fn the_restart_carries_the_world_the_fallback_and_the_new_flag() {
        let env = world_env("MYFARM", "START", true, Some("fresh"));
        assert_eq!(
            env,
            vec![
                (WORLD_ENV, Some("MYFARM".to_string())),
                (PREV_WORLD_ENV, Some("START".to_string())),
                (NEW_WORLD_ENV, Some("1".to_string())),
                (AUTH_TOKEN_ENV, Some("fresh".to_string())),
            ]
        );
        // Not new: the flag is removed. No token, or a dev login's empty one: the env stays.
        let plain = world_env("A", "B", false, None);
        assert_eq!(plain[2], (NEW_WORLD_ENV, None));
        assert_eq!(plain.len(), 3);
        assert_eq!(world_env("A", "B", false, Some("")).len(), 3);
    }
}
