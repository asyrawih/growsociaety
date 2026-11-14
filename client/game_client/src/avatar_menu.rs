//! The avatar menu (docs/knowledge/ux/avatar-menu.md): right-click, wrench-click or a 400 ms
//! long-press on a player (routed by `interact.rs`) opens a small box beside them. Other players:
//! Trade, Whisper, Mute, Report, and Kick / Ban from world when the world lets you. Yourself:
//! Wardrobe, Go home, Almanac. A view: every row sends an existing action, message or command.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use game_protocol::ReportReason;
use game_shared::commands::Role;
use game_shared::physics::{HITBOX_HEIGHT, HITBOX_WIDTH};
use game_shared::rules::{Action, PlayerId};

use crate::audio::Sfx;
use crate::chat::OpenChat;
use crate::hud::{INK_COLOR, Notice, OpenPanel, TOOLTIP_IMAGE, TOOLTIP_INSET, label, sliced};
use crate::net::Net;
use crate::player::{HomeCooldown, asset_root, offline_name};
use crate::state::{ActionMsg, Game, Phase};
use crate::trade_ui::RequestTrade;

/// Opens the menu on `player` (yourself included); `touch` makes the rows taller.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct OpenAvatarMenu {
    pub player: PlayerId,
    pub touch: bool,
}

/// Opens the chat line with this text typed in (Whisper: `/msg <name> `). Read by `chat.rs`
/// (growsociaety-1f); until it is, Whisper opens an empty chat line.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ChatDraft(pub String);

/// The hit box is this much larger than the body's on each side, world px.
const HIT_PAD: f32 = 4.0;
const ROW_HEIGHT: f32 = 32.0;
const TOUCH_ROW_HEIGHT: f32 = 44.0;
const MENU_WIDTH: f32 = 240.0;
const MENU_PADDING: f32 = 10.0;
/// Between the player's hitbox and the box, screen px.
const MENU_GAP: f32 = 12.0;
/// Kept this far inside the window edges.
const EDGE_MARGIN: f32 = 8.0;
const FLAG_SIZE: Vec2 = Vec2::new(16.0, 11.0);
const ROW_PADDING: f32 = 6.0;
const DIVIDER_HEIGHT: f32 = 2.0;
const DIVIDER_COLOR: Color = Color::srgba(0.29, 0.17, 0.08, 0.35);
/// Parchment highlight under the hovered row.
const HOVER_COLOR: Color = Color::srgba(0.29, 0.17, 0.08, 0.15);
/// The ban duration waiting for its confirming second click: `Ban 2h?` in red.
const ARMED_TEXT: Color = Color::srgb(0.7, 0.1, 0.08);
/// The second click must come this soon (the store's "Buy?" pattern).
pub const BAN_CONFIRM_SECS: f32 = 3.0;
/// The keyboard focus frame.
const FOCUS_WIDTH: f32 = 1.0;
/// While the menu is open these move the focus and choose; the player does not move or jump.
const FOCUS_UP: KeyCode = KeyCode::ArrowUp;
const FOCUS_DOWN: KeyCode = KeyCode::ArrowDown;
const FOCUS_CHOOSE: KeyCode = KeyCode::Enter;
const GREY_TEXT: Color = Color::srgba(0.29, 0.17, 0.08, 0.45);
/// After the target leaves, the menu stays (greyed) this long.
const LEFT_CLOSE_SECS: f32 = 1.5;
/// Above the HUD panels.
const MENU_Z: i32 = i32::MAX - 10;

const STAFF_CANT_BE_MUTED: &str = "Staff can't be muted";

/// `/wban` lengths offered inline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BanLength {
    Min30,
    Hours2,
    Days7,
}

pub const BAN_LENGTHS: [BanLength; 3] = [BanLength::Min30, BanLength::Hours2, BanLength::Days7];

impl BanLength {
    /// As `/wban` takes it, and as the chip shows it.
    pub fn arg(self) -> &'static str {
        match self {
            BanLength::Min30 => "30m",
            BanLength::Hours2 => "2h",
            BanLength::Days7 => "7d",
        }
    }
}

/// A row of the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Row {
    Trade,
    Whisper,
    Mute,
    Report,
    Kick,
    Ban,
    Wardrobe,
    GoHome,
    Almanac,
}

/// What you may do in this world: owner / access list of the world claim, staff role; and
/// whether the target is staff out of your reach (world powers never reach staff, and staff never
/// act on an equal or higher role).
#[derive(Clone, Copy, Default, Debug)]
pub struct Authority {
    pub owner: bool,
    pub access: bool,
    pub staff: bool,
    pub target_protected: bool,
}

/// The rows for a target: a short menu for yourself; social rows for others, plus the world
/// powers the server would allow (`/kick`: world access, `/wban`: world owner; staff both).
pub fn rows(is_self: bool, me: Authority) -> Vec<Row> {
    if is_self {
        return vec![Row::Wardrobe, Row::GoHome, Row::Almanac];
    }
    let mut rows = vec![Row::Trade, Row::Whisper, Row::Mute, Row::Report];
    if me.target_protected {
        return rows;
    }
    if me.owner || me.access || me.staff {
        rows.push(Row::Kick);
    }
    if me.owner || me.staff {
        rows.push(Row::Ban);
    }
    rows
}

/// A ban length armed until a time (`Time::elapsed_secs`).
pub type Armed = Option<(BanLength, f32)>;

/// A click on a ban length at `now`: the first arms it for `BAN_CONFIRM_SECS`, a second on the
/// same one within that sends. Returns the new armed state and the length to send.
pub fn ban_click(armed: Armed, clicked: BanLength, now: f32) -> (Armed, Option<BanLength>) {
    match armed {
        Some((length, until)) if length == clicked && now <= until => (None, Some(clicked)),
        _ => (Some((clicked, now + BAN_CONFIRM_SECS)), None),
    }
}

/// The armed length still on show at `now`.
pub fn armed_at(armed: Armed, now: f32) -> Option<BanLength> {
    armed
        .filter(|&(_, until)| now <= until)
        .map(|(length, _)| length)
}

/// Whether `point` (world px) is on a body centred at `centre`, with `HIT_PAD` to spare.
pub fn hits(centre: Vec2, point: Vec2) -> bool {
    let half = Vec2::new(HITBOX_WIDTH, HITBOX_HEIGHT) / 2.0 + HIT_PAD;
    let d = (point - centre).abs();
    d.x <= half.x && d.y <= half.y
}

/// The player under `point` (world px): others before yourself, then the nearest.
pub fn player_at(game: &Game, point: Vec2) -> Option<PlayerId> {
    game.0
        .players
        .values()
        .filter(|p| hits(Vec2::from(p.body.pos), point))
        .min_by(|a, b| {
            let key = |p: &game_shared::rules::Player| {
                (p.id == game.1, Vec2::from(p.body.pos).distance(point))
            };
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })
        .map(|p| p.id)
}

/// Your world authority from the mirrored world claim and your staff role, against `target`
/// (whose role is `theirs`).
fn authority(game: &Game, role: Role, theirs: Option<Role>) -> Authority {
    let world = &game.0.world;
    let claim = world.world_claim.and_then(|key| world.claim(key));
    Authority {
        owner: claim.is_some_and(|c| c.owner == game.1),
        access: claim.is_some_and(|c| c.access.contains(&game.1)),
        staff: role >= Role::Moderator,
        target_protected: theirs.is_some_and(|t| t >= Role::Moderator && role <= t),
    }
}

/// Whether a player with this role is staff (then Mute is greyed: staff can't be ignored).
fn is_staff(role: Option<Role>) -> bool {
    role.is_some_and(|r| r >= Role::Moderator)
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum MenuButton {
    Row(Row),
    Reason(ReportReason),
    Ban(BanLength),
}

#[derive(Resource, Default)]
struct Menu {
    target: Option<PlayerId>,
    touch: bool,
    /// Their name when the menu opened, for the `left` header.
    name: String,
    /// When they left the world.
    left_at: Option<f32>,
    armed: Armed,
    /// The Report picker replaces the rows.
    reporting: bool,
    /// Index of the keyboard-focused button, in the order they are drawn.
    focus: usize,
}

/// A button the keyboard can reach: its place in the drawing order.
#[derive(Component)]
struct Focus(usize);

/// Up / Down / Enter taken from the game while the menu is open.
#[derive(Resource, Default)]
struct MenuKeys {
    step: i32,
    choose: bool,
}

#[derive(Component)]
struct MenuRoot;

/// What the menu shows; rebuilt when it changes.
#[derive(Clone, PartialEq, Default)]
struct Shown {
    target: Option<PlayerId>,
    header: String,
    level: Option<u16>,
    flag: Option<String>,
    rows: Vec<Row>,
    muted: bool,
    staff: bool,
    armed: Option<BanLength>,
    reporting: bool,
    left: bool,
    touch: bool,
}

pub struct AvatarMenuPlugin;

impl Plugin for AvatarMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<OpenAvatarMenu>()
            .add_message::<ChatDraft>()
            .init_resource::<Menu>()
            .init_resource::<MenuKeys>()
            // Before the player's `FixedUpdate` would jump on Up (as the store does).
            .add_systems(
                RunFixedMainLoop,
                take_menu_keys.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
            )
            .add_systems(Update, use_menu.in_set(Phase::Input))
            .add_systems(
                Update,
                (build_menu, place_menu, hover_rows)
                    .chain()
                    .in_set(Phase::View),
            );
    }
}

fn take_menu_keys(
    open: Res<OpenPanel>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut out: ResMut<MenuKeys>,
) {
    if *open != OpenPanel::AvatarMenu {
        return;
    }
    out.step += i32::from(keys.just_pressed(FOCUS_DOWN)) - i32::from(keys.just_pressed(FOCUS_UP));
    out.choose |= keys.just_pressed(FOCUS_CHOOSE);
    for code in [FOCUS_UP, FOCUS_DOWN, FOCUS_CHOOSE] {
        if keys.pressed(code) {
            keys.reset(code);
        }
    }
}

fn name_of(net: Option<&Net>, id: PlayerId) -> String {
    match net {
        Some(net) => net
            .name(id)
            .map_or_else(|| format!("Player{}", id.0), str::to_owned),
        None => offline_name(),
    }
}

/// Clicks on the rows, clicks outside, opening, and the "left the world" close.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn use_menu(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    game: Res<Game>,
    mut net: Option<ResMut<Net>>,
    cooldown: Res<HomeCooldown>,
    mut open: ResMut<OpenPanel>,
    mut menu: ResMut<Menu>,
    mut opens: MessageReader<OpenAvatarMenu>,
    buttons: Query<(&MenuButton, &Hovered, Option<&Focus>)>,
    boxes: Query<&Hovered, With<MenuRoot>>,
    mut keys: ResMut<MenuKeys>,
    (mut trades, mut drafts, mut chats): (
        MessageWriter<RequestTrade>,
        MessageWriter<ChatDraft>,
        MessageWriter<OpenChat>,
    ),
    (mut actions, mut notices, mut sfx): (
        MessageWriter<ActionMsg>,
        MessageWriter<Notice>,
        MessageWriter<Sfx>,
    ),
) {
    let now = time.elapsed_secs();
    let MenuKeys { step, choose } = std::mem::take(&mut *keys);
    // Another panel took over (or Esc closed it).
    if menu.target.is_some() && *open != OpenPanel::AvatarMenu {
        *menu = Menu::default();
    }
    if let Some(target) = menu.target {
        let pressed = mouse.get_just_pressed().next().is_some() || touches.any_just_pressed();
        let left = !game.0.players.contains_key(&target);
        if left && menu.left_at.is_none() {
            menu.left_at = Some(now);
        }
        let reachable = buttons.iter().filter(|(_, _, f)| f.is_some()).count();
        if step != 0 && reachable > 0 {
            let at = menu.focus as i32 + step;
            menu.focus = at.rem_euclid(reachable as i32) as usize;
        }
        let focused = buttons
            .iter()
            .find(|(_, _, f)| f.is_some_and(|f| f.0 == menu.focus))
            .map(|(b, _, _)| *b);
        let hovered = buttons.iter().find(|(_, h, _)| h.get()).map(|(b, _, _)| *b);
        let chosen = match (mouse.just_pressed(MouseButton::Left), choose) {
            (true, _) => hovered,
            (false, true) => focused,
            _ => None,
        };
        let mut close = menu.left_at.is_some_and(|at| now - at >= LEFT_CLOSE_SECS);
        if pressed && !boxes.iter().any(Hovered::get) {
            close = true;
        } else if let (Some(button), false) = (chosen, left) {
            sfx.write(Sfx::UiClick);
            let name = menu.name.clone();
            // A chat command to send once the row is handled.
            let mut line: Option<String> = None;
            let mut say = |text: String| line = Some(text);
            close = true;
            match button {
                MenuButton::Row(Row::Trade) => {
                    trades.write(RequestTrade(target));
                }
                MenuButton::Row(Row::Whisper) => {
                    drafts.write(ChatDraft(format!("/msg {name} ")));
                    chats.write(OpenChat);
                }
                MenuButton::Row(Row::Mute) => {
                    let ignoring = net.as_deref().is_some_and(|n| n.is_ignoring(target));
                    let verb = if ignoring { "unignore" } else { "ignore" };
                    say(format!("/{verb} {name}"));
                }
                MenuButton::Row(Row::Report) => {
                    menu.reporting = true;
                    menu.focus = 0;
                    close = false;
                }
                MenuButton::Reason(reason) => {
                    if let Some(net) = net.as_deref_mut() {
                        net.report(target, reason, None);
                    }
                }
                MenuButton::Row(Row::Kick) => say(format!("/kick {name}")),
                MenuButton::Row(Row::Ban) => close = false,
                MenuButton::Ban(length) => {
                    let (armed, send) = ban_click(menu.armed, length, now);
                    menu.armed = armed;
                    match send {
                        Some(length) => say(format!("/wban {name} {}", length.arg())),
                        None => close = false,
                    }
                }
                MenuButton::Row(Row::Wardrobe) => {
                    *open = OpenPanel::Wardrobe;
                    *menu = Menu::default();
                    close = false;
                }
                MenuButton::Row(Row::Almanac) => {
                    *open = OpenPanel::Almanac;
                    *menu = Menu::default();
                    close = false;
                }
                MenuButton::Row(Row::GoHome) => {
                    if cooldown.0 > 0.0 {
                        notices.write(Notice(format!(
                            "You can go home again in {}s",
                            cooldown.0.ceil() as u32
                        )));
                    } else {
                        actions.write(ActionMsg(Action::Respawn));
                    }
                }
            }
            if let (Some(line), Some(net)) = (line, net.as_deref_mut()) {
                net.chat(line);
            }
        }
        if close {
            *menu = Menu::default();
            if *open == OpenPanel::AvatarMenu {
                *open = OpenPanel::None;
            }
        }
    }
    if let Some(&OpenAvatarMenu { player, touch }) = opens.read().last() {
        *menu = Menu {
            target: Some(player),
            touch,
            name: name_of(net.as_deref(), player),
            ..Menu::default()
        };
        *open = OpenPanel::AvatarMenu;
    }
}

/// Rebuilds the box when what it shows changes.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn build_menu(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    net: Option<Res<Net>>,
    menu: Res<Menu>,
    roots: Query<Entity, With<MenuRoot>>,
    mut shown: Local<Shown>,
    mut no_flag: Local<std::collections::HashSet<String>>,
) {
    let next = match menu.target {
        None => Shown::default(),
        Some(target) => {
            let role = net.as_deref().map_or(Role::Player, Net::role);
            let theirs = net.as_deref().and_then(|n| n.role_of(target));
            let left = menu.left_at.is_some();
            Shown {
                target: Some(target),
                header: if left {
                    format!("{} left", menu.name)
                } else {
                    name_of(net.as_deref(), target)
                },
                level: game.0.players.get(&target).map(|p| p.level),
                flag: net
                    .as_deref()
                    .and_then(|n| n.country(target))
                    .map(str::to_owned),
                rows: rows(target == game.1, authority(&game, role, theirs)),
                muted: net.as_deref().is_some_and(|n| n.is_ignoring(target)),
                staff: is_staff(theirs),
                armed: armed_at(menu.armed, time.elapsed_secs()),
                reporting: menu.reporting,
                left,
                touch: menu.touch,
            }
        }
    };
    if *shown == next {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    *shown = next;
    if shown.target.is_none() {
        return;
    }
    let s = shown.clone();
    let row_height = if s.touch {
        TOUCH_ROW_HEIGHT
    } else {
        ROW_HEIGHT
    };
    let row_node = || Node {
        height: px(row_height),
        padding: UiRect::horizontal(px(ROW_PADDING)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::SpaceBetween,
        column_gap: px(ROW_PADDING),
        ..default()
    };
    let flag = s.flag.clone().filter(|code| {
        if no_flag.contains(code) {
            return false;
        }
        let found = asset_root().join(format!("flags/{code}.png")).is_file();
        if !found {
            no_flag.insert(code.clone());
        }
        found
    });
    let text = |t: &str, enabled: bool| {
        (
            label(t),
            TextColor(if enabled { INK_COLOR } else { GREY_TEXT }),
        )
    };
    let next_focus = std::cell::Cell::new(0);
    let focus = || {
        let i = next_focus.get();
        next_focus.set(i + 1);
        (
            Focus(i),
            Outline::new(px(FOCUS_WIDTH), px(0.0), Color::NONE),
        )
    };
    let divider = || {
        (
            Node {
                height: px(DIVIDER_HEIGHT),
                margin: UiRect::vertical(px(ROW_PADDING / 2.0)),
                ..default()
            },
            BackgroundColor(DIVIDER_COLOR),
            Hovered::default(),
        )
    };
    commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: px(MENU_WIDTH),
                padding: UiRect::all(px(MENU_PADDING)),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            sliced(&assets, TOOLTIP_IMAGE, TOOLTIP_INSET),
            GlobalZIndex(MENU_Z),
            Hovered::default(),
        ))
        .with_children(|m| {
            // Header: flag, name, level.
            m.spawn((row_node(), Hovered::default()))
                .with_children(|h| {
                    h.spawn((
                        Node {
                            column_gap: px(ROW_PADDING),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        Hovered::default(),
                    ))
                    .with_children(|n| {
                        if let Some(code) = &flag {
                            n.spawn((
                                Node {
                                    width: px(FLAG_SIZE.x),
                                    height: px(FLAG_SIZE.y),
                                    ..default()
                                },
                                ImageNode::new(assets.load(format!("flags/{code}.png"))),
                                Hovered::default(),
                            ));
                        }
                        n.spawn((text(&s.header, !s.left), Hovered::default()));
                    });
                    if let Some(level) = s.level.filter(|_| !s.left) {
                        h.spawn((text(&format!("Lv {level}"), true), Hovered::default()));
                    }
                });
            m.spawn(divider());
            if s.reporting && !s.left {
                for (reason, name) in [
                    (ReportReason::Spam, "Spam"),
                    (ReportReason::Rude, "Rude"),
                    (ReportReason::Cheating, "Cheating"),
                ] {
                    m.spawn((
                        row_node(),
                        BackgroundColor(Color::NONE),
                        MenuButton::Reason(reason),
                        focus(),
                        Hovered::default(),
                    ))
                    .with_child((text(name, true), Hovered::default()));
                }
                return;
            }
            for (i, &row) in s.rows.iter().enumerate() {
                if row == Row::Kick && i > 0 {
                    m.spawn(divider());
                }
                let enabled = !s.left && !(row == Row::Mute && s.staff);
                let name = match row {
                    Row::Trade => "Trade",
                    Row::Whisper => "Whisper",
                    Row::Mute if s.muted => "Unmute",
                    Row::Mute => "Mute",
                    Row::Report => "Report",
                    Row::Kick => "Kick",
                    Row::Ban => "Ban from world",
                    Row::Wardrobe => "Wardrobe",
                    Row::GoHome => "Go home",
                    Row::Almanac => "Almanac",
                };
                let mut r = m.spawn((row_node(), BackgroundColor(Color::NONE), Hovered::default()));
                // The Ban row itself does nothing: its lengths are the buttons.
                if enabled && row != Row::Ban {
                    r.insert((MenuButton::Row(row), focus()));
                }
                r.with_children(|r| {
                    r.spawn((text(name, enabled), Hovered::default()));
                    if row == Row::Mute && s.staff {
                        r.spawn((text(STAFF_CANT_BE_MUTED, false), Hovered::default()));
                    }
                    if row == Row::Ban {
                        r.spawn((
                            Node {
                                column_gap: px(ROW_PADDING / 2.0),
                                ..default()
                            },
                            Hovered::default(),
                        ))
                        .with_children(|chips| {
                            for length in BAN_LENGTHS {
                                let mut chip = chips.spawn((
                                    Node {
                                        padding: UiRect::horizontal(px(ROW_PADDING / 2.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::NONE),
                                    Hovered::default(),
                                ));
                                if enabled {
                                    chip.insert((MenuButton::Ban(length), focus()));
                                }
                                if s.armed == Some(length) {
                                    let armed = format!("Ban {}?", length.arg());
                                    chip.with_child((
                                        label(armed),
                                        TextColor(ARMED_TEXT),
                                        Hovered::default(),
                                    ));
                                } else {
                                    chip.with_child((
                                        text(length.arg(), enabled),
                                        Hovered::default(),
                                    ));
                                }
                            }
                        });
                    }
                });
            }
        });
}

/// Beside the target every frame (it follows them), kept inside the window.
fn place_menu(
    game: Res<Game>,
    menu: Res<Menu>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut roots: Query<(&mut Node, &ComputedNode), With<MenuRoot>>,
    mut last: Local<Vec2>,
) {
    let Some(target) = menu.target else { return };
    let (camera, camera_transform) = *camera;
    if let Some(body) = game.0.players.get(&target).map(|p| p.body.pos) {
        let side = Vec3::new(
            body.0 + HITBOX_WIDTH / 2.0,
            body.1 + HITBOX_HEIGHT / 2.0,
            0.0,
        );
        if let Ok(at) = camera.world_to_viewport(camera_transform, side) {
            *last = at;
        }
    }
    for (mut node, computed) in &mut roots {
        let size = computed.size() * computed.inverse_scale_factor();
        let room = window.size() - size - EDGE_MARGIN;
        let x = (last.x + MENU_GAP).clamp(EDGE_MARGIN, room.x.max(EDGE_MARGIN));
        let y = last.y.clamp(EDGE_MARGIN, room.y.max(EDGE_MARGIN));
        if node.left != px(x) || node.top != px(y) {
            node.left = px(x);
            node.top = px(y);
        }
    }
}

/// Parchment highlight under the hovered row; the armed ban length keeps its colour.
fn hover_rows(
    menu: Res<Menu>,
    mut rows: Query<(&Hovered, &mut BackgroundColor, &Focus, &mut Outline), With<MenuButton>>,
) {
    for (hovered, mut background, focus, mut outline) in &mut rows {
        let color = if hovered.get() {
            HOVER_COLOR
        } else {
            Color::NONE
        };
        background.set_if_neq(BackgroundColor(color));
        let frame = if focus.0 == menu.focus {
            INK_COLOR
        } else {
            Color::NONE
        };
        if outline.color != frame {
            outline.color = frame;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_depend_on_the_target_and_world_powers() {
        let nobody = Authority::default();
        assert_eq!(
            rows(true, nobody),
            [Row::Wardrobe, Row::GoHome, Row::Almanac]
        );
        let owner_self = Authority {
            owner: true,
            ..nobody
        };
        assert_eq!(
            rows(true, owner_self),
            [Row::Wardrobe, Row::GoHome, Row::Almanac],
            "no social rows"
        );
        let social = [Row::Trade, Row::Whisper, Row::Mute, Row::Report];
        assert_eq!(rows(false, nobody), social);
        let access = Authority {
            access: true,
            ..nobody
        };
        assert_eq!(rows(false, access), [&social[..], &[Row::Kick]].concat());
        let full = [&social[..], &[Row::Kick, Row::Ban]].concat();
        assert_eq!(rows(false, owner_self), full);
        assert_eq!(
            rows(
                false,
                Authority {
                    staff: true,
                    ..nobody
                }
            ),
            full
        );
        // Staff out of your reach: no Kick or Ban, whatever the world says.
        let protected = Authority {
            target_protected: true,
            ..owner_self
        };
        assert_eq!(rows(false, protected), social);
        assert!(is_staff(Some(Role::Moderator)) && !is_staff(Some(Role::Player)));
        assert!(!is_staff(None));
    }

    #[test]
    fn a_ban_needs_a_second_click_on_the_same_length_within_3_s() {
        use BanLength::{Days7, Hours2};
        assert_eq!(BAN_LENGTHS.map(BanLength::arg), ["30m", "2h", "7d"]);
        let armed = Some((Hours2, BAN_CONFIRM_SECS));
        assert_eq!(ban_click(None, Hours2, 0.0), (armed, None));
        assert_eq!(armed_at(armed, 1.0), Some(Hours2));
        assert_eq!(armed_at(armed, BAN_CONFIRM_SECS + 0.1), None, "expired");
        assert_eq!(ban_click(armed, Hours2, 2.9), (None, Some(Hours2)));
        // Another length re-arms; too late re-arms the same one.
        assert_eq!(ban_click(armed, Days7, 1.0), (Some((Days7, 4.0)), None));
        assert_eq!(ban_click(armed, Hours2, 3.5), (Some((Hours2, 6.5)), None));
    }

    #[test]
    fn the_hit_box_is_a_little_larger_than_the_body() {
        let c = Vec2::new(100.0, 100.0);
        assert!(hits(c, c));
        assert!(hits(c, c + Vec2::new(HITBOX_WIDTH / 2.0 + HIT_PAD, 0.0)));
        assert!(!hits(
            c,
            c + Vec2::new(HITBOX_WIDTH / 2.0 + HIT_PAD + 1.0, 0.0)
        ));
        assert!(hits(c, c - Vec2::new(0.0, HITBOX_HEIGHT / 2.0 + HIT_PAD)));
        assert!(!hits(
            c,
            c + Vec2::new(0.0, HITBOX_HEIGHT / 2.0 + HIT_PAD + 1.0)
        ));
    }
}
