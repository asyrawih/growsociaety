//! Chat line (docs/knowledge/13, "Chat line and command input"). Enter opens it, Enter sends, Esc
//! cancels. Online the server cleans the text and echoes it to everyone, the speaker included;
//! offline the line is only shown locally. Recent lines fade from the bottom-left corner.
//!
//! While the line is open, the keyboard belongs to it: `ButtonInput<KeyCode>` is cleared right
//! after Bevy fills it, so movement, hotbar and drop keys do nothing (the same trick as
//! `screenshot.rs`). The mouse keeps working.

use std::collections::VecDeque;

use bevy::input::ButtonState;
use bevy::input::InputSystems;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use game_protocol::{ChatKind, MAX_CHAT_LEN, clean_chat};
use game_shared::commands::{
    self, Admin, COMMANDS, Command, Requirement, Role, Spec, Target, parse, required,
};
use game_shared::items::ITEMS;
use game_shared::rules::PlayerId;

use crate::net::Net;
use crate::player::offline_name;
use crate::state::{AdminMsg, Game, Phase, SystemLine};

/// Lines kept for the open chat view.
const HISTORY: usize = 50;
/// Lines shown while the chat line is closed, and for how long after they arrive.
const RECENT: usize = 6;
const RECENT_SECS: f32 = 10.0;
const FONT_SIZE: f32 = 14.0;
const MARGIN: f32 = 8.0;
/// Above the hotbar.
const BOTTOM: f32 = 72.0;
const WIDTH: f32 = 420.0;
const BACKGROUND: Color = Color::srgba(0.0, 0.0, 0.0, 0.45);
const SYSTEM_COLOR: Color = Color::srgb(1.0, 0.85, 0.4);
const PRIVATE_COLOR: Color = Color::srgb(0.75, 0.6, 1.0);
const HINT_COLOR: Color = Color::srgb(0.8, 0.85, 0.9);
/// Launcher rows shown at once.
const MAX_HINTS: usize = 8;

struct Line {
    text: String,
    kind: ChatKind,
    /// `Time::elapsed_secs` when it arrived.
    at: f32,
}

#[derive(Resource, Default)]
struct Chat {
    open: bool,
    draft: String,
    lines: VecDeque<Line>,
}

impl Chat {
    fn push(&mut self, text: String, kind: ChatKind, at: f32) {
        if self.lines.len() == HISTORY {
            self.lines.pop_front();
        }
        self.lines.push_back(Line { text, kind, at });
    }
}

#[derive(Component)]
struct ChatLog;

#[derive(Component)]
struct ChatInput;

/// A chat line from another player arrived (World or Private), e.g. for a sound.
#[derive(Message, Clone, Copy)]
#[allow(dead_code)] // the reader is audio.rs (growsociaety-56), not built yet
pub struct ChatReceived {
    pub from: PlayerId,
    pub kind: ChatKind,
}

/// Opens the chat line, as Enter does (e.g. from a HUD button).
#[derive(Message, Clone, Copy, Default)]
pub struct OpenChat;

/// The command launcher: matching commands while the draft starts with `/`.
#[derive(Component)]
struct ChatHints;

type InputBox = (With<ChatInput>, Without<ChatHints>);
type LauncherBox = (With<ChatHints>, Without<ChatInput>);

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Chat>()
            .add_message::<OpenChat>()
            .add_message::<ChatReceived>()
            .add_message::<WarpTo>()
            .add_message::<OpenWorldMenu>()
            .add_systems(Startup, spawn_chat)
            .add_systems(PreUpdate, type_chat.after(InputSystems))
            .add_systems(
                Update,
                (receive_chat, draw_chat, draw_input)
                    .chain()
                    .after(Phase::Apply),
            );
    }
}

fn spawn_chat(mut commands: Commands) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: px(MARGIN),
            bottom: px(BOTTOM),
            width: px(WIDTH),
            flex_direction: FlexDirection::Column,
            row_gap: px(2.0),
            ..default()
        })
        .with_children(|chat| {
            chat.spawn((
                ChatLog,
                Text::new(""),
                TextFont::from_font_size(FONT_SIZE),
                TextShadow::default(),
            ));
            chat.spawn((
                ChatHints,
                Text::new(""),
                TextFont::from_font_size(FONT_SIZE - 2.0),
                TextColor(HINT_COLOR),
                Node {
                    padding: UiRect::axes(px(4.0), px(2.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(BACKGROUND),
            ));
            chat.spawn((
                ChatInput,
                Text::new(""),
                TextFont::from_font_size(FONT_SIZE),
                Node {
                    padding: UiRect::axes(px(4.0), px(2.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(BACKGROUND),
            ));
        });
}

/// `/warp <name>`: go to that world (upper-case, already allowed by the deny-list). Read by the
/// world menu (`world_select.rs`), which switches the same way as its Go button.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct WarpTo(pub String);

/// A bare `/warp`: open the World menu (`world_select.rs`).
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct OpenWorldMenu;

/// docs/knowledge/ux/world-select.md.
const NAME_NOT_ALLOWED: &str = "That name isn't allowed";

/// Commands the client runs itself, listed in the launcher and `/help` next to the server's.
const CLIENT_COMMANDS: &[Spec] = &[Spec {
    name: "warp",
    usage: "/warp [world]",
    summary: "Go to a world, or open the World menu",
    needs: Requirement::Anyone,
}];

/// `/help` additions for `CLIENT_COMMANDS`: a bare `/help` still goes to the server (the line is
/// shown as well); `/help warp` is answered here only. `(line, also send)`.
fn client_help(text: &str) -> Option<(String, bool)> {
    let mut words = text.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("/help") {
        return None;
    }
    match words.next() {
        None => {
            let names: Vec<String> = CLIENT_COMMANDS
                .iter()
                .map(|c| format!("/{}", c.name))
                .collect();
            Some((format!("Also: {}", names.join(" ")), true))
        }
        Some(topic) => CLIENT_COMMANDS
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(topic.trim_start_matches('/')))
            .map(|c| (format!("{} - {}", c.usage, c.summary), false)),
    }
}

/// `/warp` handled here, never sent: `None` if the line is not a warp, else the world to go to
/// (`None` = open the menu) or the refusal to show.
fn warp(text: &str) -> Option<Result<Option<String>, &'static str>> {
    let mut words = text.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("/warp") {
        return None;
    }
    Some(match (words.next(), words.next()) {
        (None, _) => Ok(None),
        (Some(name), None) if game_protocol::world_name_allowed(name) => {
            Ok(Some(name.to_ascii_uppercase()))
        }
        _ => Err(NAME_NOT_ALLOWED),
    })
}

/// What pressing Enter on `draft` does. `None`: nothing to send.
enum Submit {
    /// Send to the server (online) or show (offline).
    Say(String),
    /// A `/command`. The one hook for the command parser of docs/knowledge/13 (C1).
    Command(String),
}

fn submit(draft: &str) -> Option<Submit> {
    let text = clean_chat(draft)?;
    Some(if text.starts_with('/') {
        Submit::Command(text)
    } else {
        Submit::Say(text)
    })
}

/// Offline the local player is Admin in debug builds only (docs/knowledge/13, C-D4).
fn offline_role() -> Role {
    if cfg!(debug_assertions) {
        Role::Owner
    } else {
        Role::Player
    }
}

/// Which launcher rows to show: staff commands only to that staff role (the server's answer at login
/// online, `offline_role` offline). World commands are listed for everyone; the server checks
/// world ownership when they run.
fn visible(spec: &Spec, role: Role) -> bool {
    match spec.needs {
        Requirement::Role(needed) => role >= needed,
        _ => true,
    }
}

/// The role that decides which commands are listed.
fn my_role(net: Option<&Net>) -> Role {
    net.map_or_else(offline_role, Net::role)
}

/// Launcher rows for `draft`: commands whose name starts with what is typed after `/`; once a
/// name is complete and followed by a space, only that command.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Arg {
    Player,
    /// An item name; it may have spaces ("small claim"), so it runs to the end of the line.
    Item,
    Other,
}

/// What each argument of a command is, for completion.
fn args_of(command: &str) -> &'static [Arg] {
    match command {
        "spawn" => &[Arg::Item],
        "give" | "take" => &[Arg::Player, Arg::Item],
        "setblock" => &[Arg::Other, Arg::Other, Arg::Item],
        "msg" | "ignore" | "unignore" | "kick" | "pull" | "wban" | "wunban" | "trade" | "gems"
        | "giveall" | "clearinv" => &[Arg::Player],
        _ => &[],
    }
}

/// The argument being typed in `draft`, if it can be completed: where it starts (byte offset),
/// what it is, and what has been typed of it.
fn typing_arg(draft: &str) -> Option<(usize, Arg, &str)> {
    let body = draft.strip_prefix('/')?;
    let command = body.split_whitespace().next()?;
    let mut start = 1 + command.len();
    // Byte offsets of the arguments typed so far; a trailing space starts a new, empty one.
    let mut tokens: Vec<usize> = Vec::new();
    let mut in_token = false;
    for (i, c) in draft[start..].char_indices() {
        if c.is_whitespace() {
            in_token = false;
        } else if !in_token {
            in_token = true;
            tokens.push(start + i);
        }
    }
    if tokens.is_empty() && !draft[start..].starts_with(char::is_whitespace) {
        return None; // still typing the command name
    }
    if !in_token {
        tokens.push(draft.len());
    }
    let index = tokens.len() - 1;
    let kinds = args_of(&command.to_ascii_lowercase());
    // An item swallows the rest of the line.
    if let Some(item) = kinds.iter().position(|&k| k == Arg::Item)
        && index >= item
    {
        start = tokens[item];
        return Some((start, Arg::Item, &draft[start..]));
    }
    match kinds.get(index) {
        Some(&kind) if kind != Arg::Other => Some((tokens[index], kind, &draft[tokens[index]..])),
        _ => None,
    }
}

/// Names that complete `typed`: prefix matches first, then any containing it, case-insensitive.
fn complete<'a>(typed: &str, candidates: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let typed = typed.to_lowercase();
    let mut prefix = Vec::new();
    let mut inside = Vec::new();
    for name in candidates {
        let lower = name.to_lowercase();
        if lower.starts_with(&typed) {
            prefix.push(name);
        } else if lower.contains(&typed) {
            inside.push(name);
        }
    }
    prefix.sort_unstable();
    inside.sort_unstable();
    prefix.extend(inside);
    prefix
}

/// Completions for the argument under the cursor: item names, or players in the world.
fn arg_hints(draft: &str, players: &[String]) -> Option<(usize, Vec<String>)> {
    let (start, kind, typed) = typing_arg(draft)?;
    let found: Vec<&str> = match kind {
        Arg::Item => complete(typed, ITEMS.iter().map(|def| def.name)),
        Arg::Player => complete(typed, players.iter().map(String::as_str)),
        Arg::Other => return None,
    };
    Some((start, found.into_iter().map(str::to_owned).collect()))
}

/// The names `<player>` can complete to: the world's players online, yourself offline.
fn player_names(net: Option<&Net>) -> Vec<String> {
    match net {
        Some(net) => net.names().map(str::to_owned).collect(),
        None => vec![offline_name()],
    }
}

fn hints(draft: &str, role: Role) -> Vec<&'static Spec> {
    let Some(body) = draft.strip_prefix('/') else {
        return Vec::new();
    };
    let typed = body.split_whitespace().next().unwrap_or("");
    let complete = body.len() > typed.len();
    COMMANDS
        .iter()
        .chain(CLIENT_COMMANDS)
        .filter(|spec| visible(spec, role))
        .filter(|spec| {
            if complete {
                spec.name.eq_ignore_ascii_case(typed)
            } else {
                spec.name.starts_with(&typed.to_ascii_lowercase())
            }
        })
        .collect()
}

/// A command typed with no server: `Ok` is an admin action for `apply_actions`, `Err` a reply line.
fn offline_command(text: &str, me: PlayerId) -> Result<Admin, String> {
    let command = parse(text).map_err(|error| error.to_string())?;
    let my_name = offline_name();
    match &command {
        Command::Help { topic } => {
            return Err(commands::help(topic.as_deref(), |spec| {
                visible(spec, offline_role())
            }));
        }
        Command::Who => return Err(format!("Players here: {my_name}")),
        _ => {}
    }
    match required(&command) {
        Requirement::Role(needed) if offline_role() >= needed => {}
        Requirement::Role(_) => {
            return Err("Admin commands work offline in debug builds only".into());
        }
        _ => return Err(format!("/{} needs a server", command.name())),
    }
    let resolve = |target: &Target| match target {
        Target::Name(name) => name.eq_ignore_ascii_case(&my_name).then_some(me),
        Target::Id(id) => (*id == me).then_some(me),
    };
    command
        .admin(resolve)
        .ok_or_else(|| "No such player here".into())
}

/// Reads typed keys into the draft while open, and keeps them from reaching the game.
#[allow(clippy::too_many_arguments)]
fn type_chat(
    time: Res<Time>,
    mut chat: ResMut<Chat>,
    mut typed: MessageReader<KeyboardInput>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut net: Option<ResMut<Net>>,
    game: Res<Game>,
    mut admins: MessageWriter<AdminMsg>,
    mut open: MessageReader<OpenChat>,
    mut warps: MessageWriter<WarpTo>,
    mut menus: MessageWriter<OpenWorldMenu>,
    mut said: MessageWriter<crate::chat_bubbles::SaidInWorld>,
    mut drafts: MessageReader<crate::avatar_menu::ChatDraft>,
    text_focus: Option<Res<crate::hud::TextFocus>>,
) {
    let was_open = chat.open;
    // A HUD text field (e.g. the claim panel's name box) owns Enter while it has focus.
    let field_focused = text_focus.is_some_and(|focus| focus.0);
    if open.read().count() > 0 {
        chat.open = true;
    }
    // "Whisper" in the avatar menu: chat opens with `/msg Name ` typed.
    // A focused HUD text field keeps the keyboard; the draft is dropped then.
    if let Some(draft) = drafts.read().last()
        && !field_focused
    {
        chat.open = true;
        chat.draft = draft.0.clone();
    }
    for key in typed.read().filter(|k| k.state == ButtonState::Pressed) {
        if !chat.open {
            // Only a fresh Enter opens it; the press is swallowed below.
            if key.logical_key == Key::Enter && !key.repeat && !field_focused {
                chat.open = true;
            }
            continue;
        }
        match &key.logical_key {
            Key::Enter => {
                let draft = std::mem::take(&mut chat.draft);
                chat.open = false;
                let now = time.elapsed_secs();
                let warped = clean_chat(&draft).and_then(|text| warp(&text));
                match (warped, net.is_some()) {
                    (Some(Ok(Some(world))), true) => {
                        warps.write(WarpTo(world));
                        continue;
                    }
                    (Some(Ok(None)), true) => {
                        menus.write(OpenWorldMenu);
                        continue;
                    }
                    (Some(Ok(_)), false) => {
                        chat.push("/warp needs the server".into(), ChatKind::System, now);
                        continue;
                    }
                    (Some(Err(reply)), _) => {
                        chat.push(reply.into(), ChatKind::System, now);
                        continue;
                    }
                    (None, _) => {}
                }
                if let Some((line, also_send)) = clean_chat(&draft).and_then(|t| client_help(&t)) {
                    chat.push(line, ChatKind::System, now);
                    if !also_send {
                        continue;
                    }
                }
                match (submit(&draft), net.as_deref_mut()) {
                    (Some(Submit::Say(text) | Submit::Command(text)), Some(net)) => net.chat(text),
                    (Some(Submit::Say(text)), None) => {
                        let from = game.1;
                        said.write(crate::chat_bubbles::SaidInWorld {
                            from,
                            text: text.clone(),
                        });
                        chat.push(format!("You: {text}"), ChatKind::World, now)
                    }
                    (Some(Submit::Command(text)), None) => match offline_command(&text, game.1) {
                        Ok(admin) => {
                            admins.write(AdminMsg(admin));
                        }
                        Err(reply) => chat.push(reply, ChatKind::System, now),
                    },
                    (None, _) => {}
                }
            }
            Key::Escape => {
                chat.open = false;
                chat.draft.clear();
            }
            Key::Backspace => {
                chat.draft.pop();
            }
            // Completes the command name from the first launcher row.
            // Completes the argument being typed, else the command name, from the first row.
            Key::Tab => {
                let players = player_names(net.as_deref());
                if let Some((start, found)) = arg_hints(&chat.draft, &players) {
                    if let Some(first) = found.first() {
                        chat.draft = format!("{}{first} ", &chat.draft[..start]);
                    }
                } else if let Some(spec) = hints(&chat.draft, my_role(net.as_deref())).first() {
                    chat.draft = format!("/{} ", spec.name);
                }
            }
            _ => {
                if let Some(text) = &key.text {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        if chat.draft.len() + c.len_utf8() <= MAX_CHAT_LEN {
                            chat.draft.push(c);
                        }
                    }
                }
            }
        }
    }
    if chat.open || was_open {
        keys.reset_all();
    }
}

fn name(net: &Net, from: PlayerId) -> String {
    net.name(from)
        .map_or_else(|| format!("Player{}", from.0), str::to_owned)
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per message stream
fn receive_chat(
    time: Res<Time>,
    mut chat: ResMut<Chat>,
    net: Option<ResMut<Net>>,
    mut lines: MessageReader<SystemLine>,
    mut received: MessageWriter<ChatReceived>,
    mut said: MessageWriter<crate::chat_bubbles::SaidInWorld>,
    mut notices: MessageWriter<crate::hud::Notice>,
    game: Res<Game>,
) {
    let now = time.elapsed_secs();
    for SystemLine(text) in lines.read() {
        chat.push(text.clone(), ChatKind::System, now);
    }
    let Some(mut net) = net else { return };
    for (from, text, kind) in net.take_chat() {
        let line = match kind {
            ChatKind::World => format!("{}: {text}", name(&net, from)),
            ChatKind::Private => format!("[from {}] {text}", name(&net, from)),
            ChatKind::System => text.clone(),
        };
        if kind != ChatKind::System && from != game.1 {
            received.write(ChatReceived { from, kind });
        }
        // A bubble over the speaker, our own lines included (the server echoes them back).
        if kind == ChatKind::World {
            let text = text.clone();
            said.write(crate::chat_bubbles::SaidInWorld { from, text });
        }
        // The answer to a report from the avatar menu is a toast too.
        if kind == ChatKind::System && game_protocol::REPORT_ANSWERS.contains(&text.as_str()) {
            notices.write(crate::hud::Notice(text.clone()));
        }
        chat.push(line, kind, now);
    }
}

fn draw_chat(
    time: Res<Time>,
    chat: Res<Chat>,
    mut drawn: Local<Vec<(String, ChatKind)>>,
    log: Single<Entity, With<ChatLog>>,
    mut commands: Commands,
) {
    let now = time.elapsed_secs();
    let shown: Vec<(String, ChatKind)> = if chat.open {
        chat.lines
            .iter()
            .map(|l| (l.text.clone(), l.kind))
            .collect()
    } else {
        let recent: Vec<_> = chat
            .lines
            .iter()
            .filter(|l| now - l.at < RECENT_SECS)
            .map(|l| (l.text.clone(), l.kind))
            .collect();
        recent[recent.len().saturating_sub(RECENT)..].to_vec()
    };
    // Rebuilt only when what is visible changes (a new line, one fading out, opening).
    if *drawn != shown {
        // One span per line, so system lines can have their own colour.
        commands.entity(*log).despawn_children();
        for (i, (text, kind)) in shown.iter().enumerate() {
            let color = match kind {
                ChatKind::World => Color::WHITE,
                ChatKind::System => SYSTEM_COLOR,
                ChatKind::Private => PRIVATE_COLOR,
            };
            commands.entity(*log).with_child((
                TextSpan::new(if i == 0 {
                    text.clone()
                } else {
                    format!("\n{text}")
                }),
                TextFont::from_font_size(FONT_SIZE),
                TextColor(color),
            ));
        }
        *drawn = shown;
    }
}

/// The input line and the launcher under it.
fn draw_input(
    chat: Res<Chat>,
    net: Option<Res<Net>>,
    input: Single<(&mut Text, &mut Node), InputBox>,
    launcher: Single<(&mut Text, &mut Node), LauncherBox>,
) {
    if !chat.is_changed() {
        return;
    }
    let (mut text, mut node) = input.into_inner();
    let display = if chat.open {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != display {
        node.display = display;
    }
    if chat.open {
        text.0 = format!("> {}_", chat.draft);
    }

    let players = player_names(net.as_deref());
    let rows: Vec<String> = match (chat.open, arg_hints(&chat.draft, &players)) {
        (false, _) => Vec::new(),
        (true, Some((_, found))) => found,
        (true, None) => hints(&chat.draft, my_role(net.as_deref()))
            .iter()
            .map(|spec| format!("{}  - {}", spec.usage, spec.summary))
            .collect(),
    };
    let (mut list, mut list_node) = launcher.into_inner();
    let display = if rows.is_empty() {
        Display::None
    } else {
        Display::Flex
    };
    if list_node.display != display {
        list_node.display = display;
    }
    let shown: Vec<&str> = rows.iter().take(MAX_HINTS).map(String::as_str).collect();
    list.0 = shown.join("\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_complete_to_items_and_players() {
        let players = vec!["Budi".to_owned(), "Ana".to_owned()];
        let (start, items) = arg_hints("/spawn pick", &players).expect("item argument");
        assert_eq!(start, "/spawn ".len());
        assert!(
            !items.is_empty() && items.iter().all(|n| n.contains("pick")),
            "{items:?}"
        );
        // Prefix matches come before matches inside the name.
        let (_, seeds) = arg_hints("/spawn dirt", &players).expect("item argument");
        assert_eq!(seeds.first().map(String::as_str), Some("dirt"));
        // Multi-word items keep completing past a space.
        let (_, claims) = arg_hints("/give Budi small cl", &players).expect("item");
        assert_eq!(claims.first().map(String::as_str), Some("small claim"));
        // The player argument of /give, then nothing for /setblock's coordinates.
        assert_eq!(
            arg_hints("/give b", &players).map(|r| r.1),
            Some(vec!["Budi".to_owned()])
        );
        assert_eq!(arg_hints("/msg ", &players).map(|r| r.1.len()), Some(2));
        assert!(arg_hints("/setblock 3", &players).is_none());
        assert!(
            arg_hints("/spaw", &players).is_none(),
            "still the command name"
        );
        assert!(arg_hints("hello", &players).is_none());
    }

    #[test]
    fn slash_lines_go_to_the_command_hook() {
        assert!(matches!(submit("  hello "), Some(Submit::Say(t)) if t == "hello"));
        assert!(matches!(submit("/who"), Some(Submit::Command(t)) if t == "/who"));
        assert!(submit(" \t ").is_none());
    }

    #[test]
    fn launcher_filters_by_typed_prefix() {
        let names = |draft| {
            hints(draft, Role::Owner)
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names("hello"), Vec::<&str>::new());
        assert_eq!(names("/w"), ["who", "wban", "wunban", "warp"]);
        assert_eq!(names("/give Budi"), ["give"]);
        assert_eq!(names("/").len(), COMMANDS.len() + CLIENT_COMMANDS.len());
        // A player is never shown staff commands.
        let player = |draft| {
            hints(draft, Role::Player)
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };
        assert!(
            player("/").iter().all(|n| ![
                "give",
                "take",
                "spawn",
                "gems",
                "setblock",
                "clearblock"
            ]
            .contains(n))
        );
        assert_eq!(player("/s"), Vec::<&str>::new());
    }

    #[test]
    fn offline_commands_resolve_only_the_local_player() {
        let me = PlayerId(1);
        let name = offline_name();
        let spawn = offline_command("/spawn dirt 5", me);
        if cfg!(debug_assertions) {
            assert!(
                matches!(spawn, Ok(Admin::Spawn { count: 5, .. })),
                "{spawn:?}"
            );
            let give = offline_command(&format!("/give {name} dirt 3 test"), me);
            assert!(matches!(give, Ok(Admin::Give { to, count: 3, .. }) if to == me));
            assert!(offline_command("/give Stranger dirt 3 test", me).is_err());
        } else {
            assert!(spawn.is_err());
        }
        assert!(matches!(offline_command("/kick Budi", me), Err(e) if e.contains("server")));
        assert!(offline_command("/help", me).is_err_and(|e| e.starts_with("Commands:")));
        assert!(offline_command("/dance", me).is_err_and(|e| e.contains("Unknown")));
    }

    #[test]
    fn warp_is_handled_here_and_checks_the_name() {
        assert_eq!(warp("/warp myfarm"), Some(Ok(Some("MYFARM".into()))));
        assert_eq!(warp("/WARP"), Some(Ok(None)), "bare: the menu");
        assert_eq!(warp("/warp admin"), Some(Err(NAME_NOT_ALLOWED)));
        assert_eq!(warp("/warp two words"), Some(Err(NAME_NOT_ALLOWED)));
        assert_eq!(warp("/warpzone"), None, "another command");
        assert_eq!(warp("hello"), None);
        assert_eq!(client_help("/help"), Some(("Also: /warp".into(), true)));
        assert_eq!(
            client_help("/help warp"),
            Some((
                "/warp [world] - Go to a world, or open the World menu".into(),
                false
            ))
        );
        assert_eq!(client_help("/help give"), None, "the server's");
    }

    #[test]
    fn history_is_bounded() {
        let mut chat = Chat::default();
        for i in 0..HISTORY + 5 {
            chat.push(i.to_string(), ChatKind::World, 0.0);
        }
        assert_eq!(chat.lines.len(), HISTORY);
        assert_eq!(chat.lines.front().map(|l| l.text.as_str()), Some("5"));
    }
}
