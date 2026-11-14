//! Login / register screen, shown before an online game (assets/README.md, "Lobby").
//!
//! It is its own small Bevy app. Login and register go over the game's own QUIC port: a short
//! `Hello` with `Auth::Login` / `Auth::Register` checks the account (the reply is `Welcome` or a
//! `Kicked` reason shown in the panel). `Welcome` carries a signed 15-minute token; the connection
//! closes, the game starts as a new process with `GAME_AUTH_TOKEN` set (so its plugins start online
//! exactly as with that env variable), and the lobby exits. Offline and dev runs (`GAME_TOKEN` /
//! `GAME_USER` set) never see it. ponytail: the window closes and a new one opens; move the game
//! behind an `AppState` if that flash matters.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use bevy::app::AppExit;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_protocol::{
    Auth, ClientMsg, MAX_SERVER_MSG, PROTOCOL_VERSION, SERVER_NAME, ServerMsg, read_msg,
    valid_name, valid_password, write_msg,
};

use crate::hud::{INK_COLOR, sliced};
use crate::net::{AUTH_TOKEN_ENV, PASSWORD_ENV, USER_ENV};

/// How long one login attempt may take.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
/// The remembered name, in the settings folder.
const REMEMBER_FILE: &str = "lobby_name.txt";

const FONT: &str = "fonts/growsociaety_pixel.ttf";
const BACKGROUND: &str = "ui/lobby/background.png";
const PANEL: &str = "ui/panel.png";
const INPUT: &str = "ui/lobby/input.png";
const BGM: &str = "audio/bgm/lobby.ogg";
const CLICK: &str = "audio/sfx/ui_click.ogg";
const DENIED: &str = "audio/sfx/denied.ogg";

const PANEL_INSET: f32 = 18.0;
const INPUT_INSET: f32 = 8.0;
const BUTTON_INSET: f32 = 10.0;
const WIDTH: f32 = 520.0;
const FIELD_HEIGHT: f32 = 60.0;
const BUTTON_SIZE: Vec2 = Vec2::new(210.0, 64.0);
const ICON: f32 = 40.0;
const BIG: f32 = 32.0;
const SMALL: f32 = 16.0;
const LOGO: f32 = 64.0;
const FIELD_TEXT: Color = Color::srgb(0.93, 0.89, 0.8);
const PLACEHOLDER: Color = Color::srgb(0.55, 0.52, 0.48);
const ERROR: Color = Color::srgb(0.7, 0.12, 0.08);
const LOGO_COLOR: Color = Color::srgb(0.98, 0.85, 0.45);

/// True when the game should start at the lobby: online, and no credentials in the environment.
pub fn wanted() -> bool {
    let set = |var| std::env::var_os(var).is_some();
    crate::net::server_address().is_some()
        && ![AUTH_TOKEN_ENV, "GAME_TOKEN", "GAME_USER"]
            .into_iter()
            .any(set)
}

/// Runs the lobby window; on a successful login, restarts the game with the token.
pub fn run(plugins: impl PluginGroup) {
    let mut app = App::new();
    app.add_plugins(plugins)
        .add_plugins((LobbyPlugin, crate::screenshot::DebugScreenshotPlugin));
    app.run();
}

/// Starts the game as a new process with the token. It must start before the lobby exits:
/// on macOS winit ends the whole process when its event loop stops, so nothing after
/// `app.run()` would run.
fn start_game(token: &str) -> std::io::Result<()> {
    std::process::Command::new(std::env::current_exe()?)
        .args(std::env::args_os().skip(1))
        .env_remove(USER_ENV)
        .env_remove(PASSWORD_ENV)
        .env(AUTH_TOKEN_ENV, token)
        .spawn()
        .map(drop)
}

struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        let name = crate::settings::config_dir()
            .and_then(|dir| std::fs::read_to_string(dir.join(REMEMBER_FILE)).ok())
            .map(|name| name.trim().to_owned())
            .unwrap_or_default();
        app.insert_resource(Form {
            remember: !name.is_empty(),
            name,
            ..default()
        })
        .add_systems(Startup, spawn)
        .add_systems(Update, (click, type_keys, finish, show).chain());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Screen {
    #[default]
    Login,
    Register,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Name,
    Password,
    Confirm,
}

impl Screen {
    fn fields(self) -> &'static [Field] {
        match self {
            Screen::Login => &[Field::Name, Field::Password],
            Screen::Register => &[Field::Name, Field::Password, Field::Confirm],
        }
    }
}

#[derive(Resource, Default)]
struct Form {
    screen: Screen,
    name: String,
    password: String,
    confirm: String,
    focus: usize,
    show_password: bool,
    remember: bool,
    error: String,
    /// The answer of the request in flight.
    pending: Option<Mutex<Receiver<Result<String, String>>>>,
}

impl Form {
    fn value(&mut self, field: Field) -> &mut String {
        match field {
            Field::Name => &mut self.name,
            Field::Password => &mut self.password,
            Field::Confirm => &mut self.confirm,
        }
    }

    fn focused(&self) -> Field {
        self.screen.fields()[self.focus.min(self.screen.fields().len() - 1)]
    }

    /// Local checks first, so the server only sees plausible requests.
    fn check(&self) -> Result<(), &'static str> {
        if !valid_name(&self.name) {
            return Err("Names are 1-16 letters, digits or _");
        }
        // Login only needs a password; the length rule is for new accounts (demo ones are shorter).
        if self.password.is_empty() {
            return Err("Enter your password");
        }
        if self.screen == Screen::Register && !valid_password(&self.password) {
            return Err("Passwords are 8-128 characters");
        }
        if self.screen == Screen::Register && self.password != self.confirm {
            return Err("Passwords do not match");
        }
        Ok(())
    }
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Control {
    Field(Field),
    Submit,
    Switch,
    Eye,
    Remember,
}

/// A button image with `_hover` / `_pressed` versions.
#[derive(Component)]
struct Skin(&'static str);

#[derive(Component)]
struct ScreenRoot(Screen);

#[derive(Component)]
struct FieldText(Field);

#[derive(Component)]
struct ErrorText;

#[derive(Component)]
struct RememberBox;

fn spawn(mut commands: Commands, assets: Res<AssetServer>, form: Res<Form>) {
    let font = assets.load(FONT);
    let text = |size: f32| TextFont {
        font: FontSource::Handle(font.clone()),
        ..TextFont::from_font_size(size)
    };
    commands.spawn(Camera2d);
    // Screenshot runs are silent, like the game's.
    if crate::screenshot::screenshot_path().is_none() {
        use crate::settings::{Settings, config_dir};
        let level = config_dir()
            .map_or(Settings::default(), |dir| Settings::load_from(&dir).0)
            .music;
        commands.spawn(crate::audio::music_player(&assets, BGM, level));
    }
    let root = Node {
        width: percent(100),
        height: percent(100),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: px(24),
        ..default()
    };
    commands
        .spawn((root, ImageNode::new(assets.load(BACKGROUND))))
        .with_children(|root| {
            root.spawn((
                Text::new("GROWSOCIAETY"),
                text(LOGO),
                TextColor(LOGO_COLOR),
                TextShadow::default(),
            ));
            for screen in [Screen::Login, Screen::Register] {
                let panel = Node {
                    width: px(WIDTH),
                    padding: UiRect::all(px(32)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: px(14),
                    display: if screen == form.screen {
                        Display::Flex
                    } else {
                        Display::None
                    },
                    ..default()
                };
                root.spawn((
                    panel,
                    sliced(&assets, PANEL, PANEL_INSET),
                    ScreenRoot(screen),
                ))
                .with_children(|panel| {
                    let title = match screen {
                        Screen::Login => "Login",
                        Screen::Register => "Create Account",
                    };
                    panel.spawn((
                        Text::new(title),
                        text(BIG),
                        TextColor(INK_COLOR),
                        Node {
                            align_self: AlignSelf::Center,
                            ..default()
                        },
                    ));
                    for &field in screen.fields() {
                        spawn_field(panel, &assets, &text, field);
                    }
                    if screen == Screen::Login {
                        let row = Node {
                            column_gap: px(10),
                            align_items: AlignItems::Center,
                            ..default()
                        };
                        panel.spawn(row).with_children(|row| {
                            row.spawn((
                                Node {
                                    width: px(29),
                                    height: px(29),
                                    ..default()
                                },
                                ImageNode::new(assets.load(checkbox(form.remember))),
                                Control::Remember,
                                RememberBox,
                                Hovered::default(),
                            ));
                            row.spawn((Text::new("Remember me"), text(BIG), TextColor(INK_COLOR)));
                        });
                    }
                    panel.spawn((Text::new(""), text(SMALL), TextColor(ERROR), ErrorText));
                    let buttons = Node {
                        justify_content: JustifyContent::SpaceBetween,
                        ..default()
                    };
                    let (submit, switch) = match screen {
                        Screen::Login => ("Login", "Register"),
                        Screen::Register => ("Register", "Back"),
                    };
                    panel.spawn(buttons).with_children(|row| {
                        for (label, skin, control) in [
                            (submit, "ui/lobby/button_primary", Control::Submit),
                            (switch, "ui/lobby/button_secondary", Control::Switch),
                        ] {
                            let node = Node {
                                width: px(BUTTON_SIZE.x),
                                height: px(BUTTON_SIZE.y),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            };
                            row.spawn((
                                node,
                                sliced(&assets, png(skin), BUTTON_INSET),
                                Skin(skin),
                                control,
                                Hovered::default(),
                            ))
                            .with_child((
                                Text::new(label),
                                text(BIG),
                                TextColor(Color::WHITE),
                                TextShadow::default(),
                            ));
                        }
                    });
                });
            }
        });
}

fn spawn_field(
    panel: &mut ChildSpawnerCommands,
    assets: &AssetServer,
    text: &impl Fn(f32) -> TextFont,
    field: Field,
) {
    let node = Node {
        height: px(FIELD_HEIGHT),
        padding: UiRect::horizontal(px(16)),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::SpaceBetween,
        ..default()
    };
    panel
        .spawn((
            node,
            sliced(assets, INPUT, INPUT_INSET),
            Control::Field(field),
            Hovered::default(),
        ))
        .with_children(|input| {
            input.spawn((
                Text::new(""),
                text(BIG),
                TextColor(FIELD_TEXT),
                FieldText(field),
            ));
            if field == Field::Password {
                input.spawn((
                    Node {
                        width: px(ICON),
                        height: px(ICON),
                        ..default()
                    },
                    ImageNode::new(assets.load(png("ui/lobby/eye"))),
                    Skin("ui/lobby/eye"),
                    Control::Eye,
                    Hovered::default(),
                ));
            }
        });
}

/// `name` + `.png`, leaked once per distinct skin: the art paths are a handful of constants.
fn png(name: &'static str) -> &'static str {
    Box::leak(format!("{name}.png").into_boxed_str())
}

fn checkbox(on: bool) -> &'static str {
    if on {
        "ui/lobby/checkbox_on.png"
    } else {
        "ui/lobby/checkbox_off.png"
    }
}

fn click(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mouse: Res<ButtonInput<MouseButton>>,
    controls: Query<(&Control, &Hovered)>,
    mut form: ResMut<Form>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    // The eye sits inside a field: the innermost control wins.
    let mut hovered: Vec<Control> = controls
        .iter()
        .filter(|(_, h)| h.get())
        .map(|(c, _)| *c)
        .collect();
    hovered.sort_by_key(|c| matches!(c, Control::Field(_)));
    let Some(control) = hovered.first().copied() else {
        return;
    };
    match control {
        Control::Field(field) => {
            form.focus = form
                .screen
                .fields()
                .iter()
                .position(|f| *f == field)
                .unwrap_or(0);
        }
        Control::Eye => form.show_password = !form.show_password,
        Control::Remember => form.remember = !form.remember,
        Control::Switch => {
            form.screen = match form.screen {
                Screen::Login => Screen::Register,
                Screen::Register => Screen::Login,
            };
            form.focus = 0;
            form.error.clear();
        }
        Control::Submit => submit(&mut commands, &assets, &mut form),
    }
    if !matches!(control, Control::Field(_)) {
        commands.spawn((
            AudioPlayer::new(assets.load(CLICK)),
            PlaybackSettings::DESPAWN,
        ));
    }
}

fn type_keys(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut typed: MessageReader<KeyboardInput>,
    mut form: ResMut<Form>,
) {
    for key in typed.read().filter(|k| k.state == ButtonState::Pressed) {
        let field = form.focused();
        match &key.logical_key {
            Key::Enter => submit(&mut commands, &assets, &mut form),
            Key::Tab => form.focus = (form.focus + 1) % form.screen.fields().len(),
            Key::Backspace => {
                form.value(field).pop();
            }
            _ => {
                let Some(text) = &key.text else { continue };
                let limit = if field == Field::Name {
                    game_protocol::MAX_NAME_LEN
                } else {
                    game_protocol::MAX_PASSWORD_LEN
                };
                let value = form.value(field);
                for c in text.chars().filter(|c| !c.is_control()) {
                    if value.len() + c.len_utf8() <= limit {
                        value.push(c);
                    }
                }
            }
        }
    }
}

fn submit(commands: &mut Commands, assets: &AssetServer, form: &mut Form) {
    if form.pending.is_some() {
        return;
    }
    if let Err(error) = form.check() {
        refuse(commands, assets, form, error.into());
        return;
    }
    let Some(server) = crate::net::server_address() else {
        return;
    };
    let (name, password) = (form.name.clone(), form.password.clone());
    let auth = match form.screen {
        Screen::Login => Auth::Login { name, password },
        Screen::Register => Auth::Register { name, password },
    };
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(check_account(&server, auth));
    });
    form.pending = Some(Mutex::new(rx));
    form.error = "Connecting...".into();
}

fn refuse(commands: &mut Commands, assets: &AssetServer, form: &mut Form, error: String) {
    form.error = error;
    commands.spawn((
        AudioPlayer::new(assets.load(DENIED)),
        PlaybackSettings::DESPAWN,
    ));
}

/// Picks up the answer of the request in flight.
fn finish(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut form: ResMut<Form>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(answer) = form
        .pending
        .as_ref()
        .and_then(|rx| rx.lock().ok()?.try_recv().ok())
    else {
        return;
    };
    form.pending = None;
    match answer {
        Ok(token) => {
            if let Some(dir) = crate::settings::config_dir() {
                let path = dir.join(REMEMBER_FILE);
                let _ = if form.remember {
                    std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(path, &form.name))
                } else {
                    std::fs::remove_file(path)
                };
            }
            match start_game(&token) {
                Ok(()) => {
                    exit.write(AppExit::Success);
                }
                Err(e) => refuse(
                    &mut commands,
                    &assets,
                    &mut form,
                    format!("Cannot start the game: {e}"),
                ),
            }
        }
        Err(error) => refuse(&mut commands, &assets, &mut form, error),
    }
}

/// Draws the form: which screen, field texts, caret, error, checkbox and button states.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system drawing every part of the form
fn show(
    form: Res<Form>,
    time: Res<Time>,
    assets: Res<AssetServer>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut screens: Query<(&ScreenRoot, &mut Node)>,
    mut fields: Query<(&FieldText, &mut Text, &mut TextColor), Without<ErrorText>>,
    mut errors: Query<&mut Text, With<ErrorText>>,
    mut boxes: Query<&mut ImageNode, (With<RememberBox>, Without<Skin>)>,
    mut skins: Query<(&Skin, &Hovered, &mut ImageNode)>,
) {
    for (root, mut node) in &mut screens {
        let display = if root.0 == form.screen {
            Display::Flex
        } else {
            Display::None
        };
        node.display = display;
    }
    let caret = ((time.elapsed_secs() * 2.0) as u32).is_multiple_of(2);
    let focused = form.focused();
    for (field, mut text, mut color) in &mut fields {
        let value = match field.0 {
            Field::Name => form.name.clone(),
            Field::Password if form.show_password => form.password.clone(),
            Field::Password => "*".repeat(form.password.chars().count()),
            Field::Confirm => "*".repeat(form.confirm.chars().count()),
        };
        let placeholder = match field.0 {
            Field::Name => "Username",
            Field::Password => "Password",
            Field::Confirm => "Confirm Password",
        };
        let shown = match (value.is_empty(), field.0 == focused && caret) {
            (true, false) => placeholder.to_owned(),
            (true, true) => "_".into(),
            (false, true) => format!("{value}_"),
            (false, false) => value.clone(),
        };
        color.0 = if value.is_empty() && shown != "_" {
            PLACEHOLDER
        } else {
            FIELD_TEXT
        };
        if text.0 != shown {
            text.0 = shown;
        }
    }
    for mut text in &mut errors {
        if text.0 != form.error {
            text.0.clone_from(&form.error);
        }
    }
    for mut image in &mut boxes {
        image.image = assets.load(checkbox(form.remember));
    }
    for (skin, hovered, mut image) in &mut skins {
        let state = match (hovered.get(), mouse.pressed(MouseButton::Left)) {
            (true, true) => "_pressed",
            (true, false) => "_hover",
            _ => "",
        };
        let path = format!("{}{state}.png", skin.0);
        if image.image.path().is_none_or(|p| p.to_string() != path) {
            image.image = assets.load(path);
        }
    }
}

fn unreachable<E>(_: E) -> String {
    "Cannot reach the server".into()
}

/// One `Hello` on the game port: the account's token on `Welcome`, the server's reason on `Kicked`.
/// The connection is closed cleanly, so the account is free again for the game's own login.
fn check_account(server: &str, auth: Auth) -> Result<String, String> {
    let (addr, config) = crate::net::quic_target(server).map_err(unreachable)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async move {
        let local = if addr.is_ipv6() {
            "[::]:0"
        } else {
            "0.0.0.0:0"
        };
        let mut endpoint = quinn::Endpoint::client(local.parse().map_err(|_| "bind address")?)
            .map_err(unreachable)?;
        endpoint.set_default_client_config(config);
        let attempt = async {
            let conn = endpoint
                .connect(addr, SERVER_NAME)
                .map_err(unreachable)?
                .await
                .map_err(unreachable)?;
            let (mut send, mut recv) = conn.open_bi().await.map_err(unreachable)?;
            let hello = ClientMsg::Hello {
                protocol: PROTOCOL_VERSION,
                auth,
                country: None,
            };
            write_msg(&mut send, &hello).await.map_err(unreachable)?;
            let answer = match read_msg::<ServerMsg>(&mut recv, MAX_SERVER_MSG).await {
                Ok(ServerMsg::Welcome { token, .. }) if !token.is_empty() => Ok(token),
                Ok(ServerMsg::Kicked { reason }) => Err(reason),
                _ => Err("The server sent a bad answer".to_owned()),
            };
            conn.close(0u32.into(), b"lobby");
            answer
        };
        let answer = tokio::time::timeout(LOGIN_TIMEOUT, attempt)
            .await
            .unwrap_or_else(|_| Err("The server did not answer".into()));
        endpoint.wait_idle().await;
        answer
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_checks_before_sending() {
        let mut form = Form {
            name: "demo1".into(),
            password: "short".into(),
            ..default()
        };
        assert!(form.check().is_ok(), "login takes any password");
        form.password.clear();
        assert!(form.check().is_err());
        form.screen = Screen::Register;
        form.password = "short".into();
        assert!(form.check().is_err(), "new accounts need 8+");
        form.password = "longenough".into();
        form.confirm = "different1".into();
        assert_eq!(form.check(), Err("Passwords do not match"));
        form.confirm = "longenough".into();
        assert!(form.check().is_ok());
        form.name = "bad name!".into();
        assert!(form.check().is_err());
    }

    /// Against a running server: `GAME_SERVER=127.0.0.1:7777 cargo test -p game_client
    /// --no-default-features -- --ignored account_`. Registers a fresh name, then logs in.
    #[test]
    #[ignore = "needs a world_server with a database"]
    fn account_checks_over_quic() {
        let server = crate::net::server_address().expect("GAME_SERVER");
        let name = format!("t{}", std::process::id() % 100_000);
        let auth = |register: bool, password: &str| {
            let (name, password) = (name.clone(), password.to_owned());
            if register {
                Auth::Register { name, password }
            } else {
                Auth::Login { name, password }
            }
        };
        assert!(check_account(&server, auth(true, "password1")).is_ok());
        assert_eq!(
            check_account(&server, auth(true, "password1")),
            Err("That name is taken".into())
        );
        // The probe released the account, so it can log in again right away.
        assert!(check_account(&server, auth(false, "password1")).is_ok_and(|t| !t.is_empty()));
        assert_eq!(
            check_account(&server, auth(false, "wrongpass")),
            Err("Wrong name or password".into())
        );
    }
}
