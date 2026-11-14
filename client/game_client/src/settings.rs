//! The settings window (top-bar gear; N opens it on the Voice tab) and the one local settings file,
//! `settings.txt` in the OS config folder (`key=value` lines). Nothing here goes to the game server.
//! Art: `assets/ui/settings/` (assets/README.md, "Top bar and settings"). The Voice tab's status
//! line and player list are added by `voice/panel.rs`.

use std::path::{Path, PathBuf};

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};

use crate::audio::Sfx;
use crate::camera::{SetZoom, ZOOM_LEVELS};
use crate::hud::{
    HUD_MARGIN, HudText, INK_COLOR, KIT_SCALE, OpenPanel, SLOT_GAP, ink, label, sliced,
    spawn_centre_panel, tab_node,
};
use crate::player::Player;
use crate::player::{Look, rig::SKIN_RAMPS};
use crate::screenshot;
use crate::state::Phase;

const APP_DIR: &str = "growsociaety";
const FILE: &str = "settings.txt";
/// Written by the voice code before this file existed; read once if `settings.txt` is missing.
const OLD_VOICE_FILE: &str = "voice.txt";
pub const MAX_VOLUME: u8 = 100;
const DEFAULT_VOLUME: u8 = 80;
/// There is no translation system yet: English is the only language.
const LANGUAGES: [&str; 1] = ["English"];
const MIC_DEFAULT: &str = "System default";
/// Opens the settings on the Voice tab (free: see the bindings in hud, interact, player, camera, chat).
pub(crate) const VOICE_KEY: KeyCode = KeyCode::KeyN;

/// Everything the player can adjust, saved on this machine.
#[derive(Resource, Clone, PartialEq, Debug)]
pub struct Settings {
    /// 0..=`MAX_VOLUME`, played at by `audio.rs`.
    pub music: u8,
    pub sfx: u8,
    /// Applied to every voice speaker.
    pub voice_volume: u8,
    /// Voice chat on. Off until the player turns it on (doc 12: no microphone opens by surprise).
    pub voice: bool,
    /// Open mic; otherwise push-to-talk (V). One of the two is always the mode.
    pub open_mic: bool,
    pub self_muted: bool,
    /// Input device by name; `None` is the system default.
    pub mic: Option<String>,
    pub fullscreen: bool,
    /// Index into `ZOOM_LEVELS` the camera starts at.
    pub zoom: usize,
    /// Index into `SKIN_RAMPS`, written to the local player's `Look`.
    pub skin: usize,
    /// Index into `LANGUAGES`.
    pub language: usize,
    /// First-run hints already finished, 0..=3; `hints.rs`.
    pub hints: u8,
    /// The claim tip ("hold the wrench and click your claim") was shown once; `claim_ui.rs`.
    pub claim_tip_seen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            music: DEFAULT_VOLUME,
            sfx: DEFAULT_VOLUME,
            voice_volume: DEFAULT_VOLUME,
            voice: false,
            open_mic: false,
            self_muted: false,
            mic: None,
            fullscreen: false,
            zoom: 0,
            skin: 0,
            language: 0,
            hints: 0,
            claim_tip_seen: false,
        }
    }
}

fn volume(value: &str) -> Option<u8> {
    Some(
        value
            .parse::<f32>()
            .ok()?
            .clamp(0.0, MAX_VOLUME as f32)
            .round() as u8,
    )
}

fn index(value: &str, len: usize) -> Option<usize> {
    value.parse::<usize>().ok().filter(|&i| i < len)
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            let (key, value) = (key.trim(), value.trim());
            let yes = value == "true";
            match key {
                "music" => s.music = volume(value).unwrap_or(s.music),
                "sfx" => s.sfx = volume(value).unwrap_or(s.sfx),
                "voice_volume" => s.voice_volume = volume(value).unwrap_or(s.voice_volume),
                "voice" => s.voice = yes,
                "open_mic" => s.open_mic = yes,
                "self_muted" => s.self_muted = yes,
                "mic" if !value.is_empty() => s.mic = Some(value.to_owned()),
                "fullscreen" => s.fullscreen = yes,
                "zoom" => s.zoom = index(value, ZOOM_LEVELS.len()).unwrap_or(s.zoom),
                "skin" => s.skin = index(value, SKIN_RAMPS.len()).unwrap_or(s.skin),
                "hints" => s.hints = value.parse().unwrap_or(s.hints),
                "claim_tip_seen" => s.claim_tip_seen = yes,
                "language" => {
                    s.language = LANGUAGES.iter().position(|&l| l == value).unwrap_or(0);
                }
                _ => {}
            }
        }
        s
    }

    /// The old `voice.txt` (enabled, open_mic, self_muted, volume 0..1, device); the rest default.
    pub fn from_voice_txt(text: &str) -> Self {
        let mut s = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            let (key, value) = (key.trim(), value.trim());
            let yes = value == "true";
            match key {
                "enabled" => s.voice = yes,
                "open_mic" => s.open_mic = yes,
                "self_muted" => s.self_muted = yes,
                "volume" => {
                    if let Ok(v) = value.parse::<f32>() {
                        s.voice_volume = (v.clamp(0.0, 1.0) * MAX_VOLUME as f32).round() as u8;
                    }
                }
                "device" if !value.is_empty() => s.mic = Some(value.to_owned()),
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        format!(
            "music={}\nsfx={}\nvoice_volume={}\nvoice={}\nopen_mic={}\nself_muted={}\nmic={}\n\
             fullscreen={}\nzoom={}\nskin={}\nlanguage={}\nhints={}\nclaim_tip_seen={}\n",
            self.music,
            self.sfx,
            self.voice_volume,
            self.voice,
            self.open_mic,
            self.self_muted,
            self.mic.as_deref().unwrap_or(""),
            self.fullscreen,
            self.zoom,
            self.skin,
            LANGUAGES[self.language.min(LANGUAGES.len() - 1)],
            self.hints,
            self.claim_tip_seen,
        )
    }

    /// `settings.txt` in `dir`, else the old `voice.txt` (then `true`: save it as `settings.txt`),
    /// else the defaults.
    pub fn load_from(dir: &Path) -> (Self, bool) {
        if let Ok(text) = std::fs::read_to_string(dir.join(FILE)) {
            return (Self::parse(&text), false);
        }
        match std::fs::read_to_string(dir.join(OLD_VOICE_FILE)) {
            Ok(text) => (Self::from_voice_txt(&text), true),
            Err(_) => (Self::default(), false),
        }
    }

    pub fn save_to(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(FILE), self.to_text())
    }

    fn save(&self) {
        let Some(dir) = config_dir() else { return };
        if let Err(e) = self.save_to(&dir) {
            warn!("settings not saved to {}: {e}", dir.display());
        }
    }

    fn slider(&self, field: Field) -> u8 {
        match field {
            Field::Music => self.music,
            Field::Sfx => self.sfx,
            _ => self.voice_volume,
        }
    }

    fn set_slider(&mut self, field: Field, value: u8) {
        match field {
            Field::Music => self.music = value,
            Field::Sfx => self.sfx = value,
            _ => self.voice_volume = value,
        }
    }

    fn toggle(&self, field: Field) -> bool {
        match field {
            Field::Voice => self.voice,
            Field::PushToTalk => !self.open_mic,
            Field::OpenMic => self.open_mic,
            Field::SelfMute => self.self_muted,
            _ => self.fullscreen,
        }
    }

    /// Push-to-talk and open mic are one choice: turning either on turns the other off, and
    /// turning either off turns the other on.
    pub fn set_toggle(&mut self, field: Field, on: bool) {
        match field {
            Field::Voice => self.voice = on,
            Field::PushToTalk => self.open_mic = !on,
            Field::OpenMic => self.open_mic = on,
            Field::SelfMute => self.self_muted = on,
            _ => self.fullscreen = on,
        }
    }

    fn choice(&self, field: Field, mics: &[String]) -> usize {
        match field {
            Field::Zoom => self.zoom,
            Field::Skin => self.skin,
            Field::Language => self.language,
            _ => self
                .mic
                .as_ref()
                .and_then(|m| mics.iter().position(|d| d == m))
                .map_or(0, |i| i + 1),
        }
    }

    fn choose(&mut self, field: Field, choice: usize, mics: &[String]) {
        match field {
            Field::Zoom => self.zoom = choice.min(ZOOM_LEVELS.len() - 1),
            Field::Skin => self.skin = choice.min(SKIN_RAMPS.len() - 1),
            Field::Language => self.language = choice.min(LANGUAGES.len() - 1),
            _ => self.mic = choice.checked_sub(1).and_then(|i| mics.get(i)).cloned(),
        }
    }
}

/// `~/Library/Application Support` (macOS), `%APPDATA%` (Windows), else `$XDG_CONFIG_HOME` or
/// `~/.config`; then `growsociaety`.
pub(crate) fn config_dir() -> Option<PathBuf> {
    let env = |name| std::env::var_os(name).map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        env("HOME").map(|home| home.join("Library/Application Support"))
    } else if cfg!(windows) {
        env("APPDATA")
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|home| home.join(".config")))
    };
    Some(base?.join(APP_DIR))
}

/// Slider value (0..=`MAX_VOLUME`) for a pointer at `x` over a track starting at `left`, `width` wide.
pub fn slider_value(x: f32, left: f32, width: f32) -> u8 {
    if width <= 0.0 {
        return 0;
    }
    (((x - left) / width).clamp(0.0, 1.0) * MAX_VOLUME as f32).round() as u8
}

/// Present: never write the settings file (screenshot runs, the voice test mode).
#[derive(Resource)]
pub struct Unsaved;

/// Microphones the voice thread reported, for the Microphone dropdown.
#[derive(Resource, Default)]
pub struct MicDevices(pub Vec<String>);

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    #[default]
    Audio,
    Voice,
    Display,
    Keys,
    Account,
}

/// The tabs, left to right: label and icon (`ui/settings/icon_<name>.png`).
const TABS: [(SettingsTab, &str, &str); 5] = [
    (SettingsTab::Audio, "Audio", "audio"),
    (SettingsTab::Voice, "Voice", "voice"),
    (SettingsTab::Display, "Display", "display"),
    (SettingsTab::Keys, "Keys", "controls"),
    (SettingsTab::Account, "Account", "account"),
];

/// Read-only list for the Keys tab. ponytail: copied from the key constants in hud, interact,
/// player, camera, chat and voice; derive it from them when rebinding arrives.
const BINDINGS: [(&str, &str); 13] = [
    ("Move", "A / D or Left / Right"),
    ("Jump", "Space, W or Up"),
    ("Punch / place", "Left mouse (hold)"),
    ("Select hotbar slot", "1 - 8"),
    ("Drop one", "Q"),
    ("Trash one", "X twice"),
    ("Inventory / wardrobe", "E or I"),
    ("Store", "B"),
    ("Voice settings", "N"),
    ("Push-to-talk", "Hold V"),
    ("Chat", "Enter"),
    ("Close window", "Esc"),
    ("Zoom", "Mouse wheel or + / -"),
];

/// A setting a control shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Music,
    Sfx,
    VoiceVolume,
    Voice,
    PushToTalk,
    OpenMic,
    SelfMute,
    Mic,
    Fullscreen,
    Zoom,
    Skin,
    Language,
}

/// What a dropdown offers, in order.
fn options(field: Field, mics: &[String]) -> Vec<String> {
    match field {
        Field::Zoom => ZOOM_LEVELS.iter().map(|z| format!("{z}x")).collect(),
        Field::Skin => SKIN_RAMPS
            .iter()
            .map(|(name, _)| capitalised(name))
            .collect(),
        Field::Language => LANGUAGES.iter().map(|&l| l.to_owned()).collect(),
        _ => std::iter::once(MIC_DEFAULT.to_owned())
            .chain(mics.iter().cloned())
            .collect(),
    }
}

fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The dropdown whose list is open.
#[derive(Resource, Default)]
pub struct OpenDropdown(pub Option<Field>);

#[derive(Component)]
struct TabButton(SettingsTab);
/// A tab's page; `voice/panel.rs` adds to the Voice one.
#[derive(Component)]
pub struct TabBody(pub SettingsTab);
/// A slider track (the knob is its child).
#[derive(Component)]
struct Slider(Field);
#[derive(Component)]
struct Knob(Field);
#[derive(Component)]
struct Toggle(Field);
#[derive(Component)]
struct Dropdown(Field);
/// "Show tips again" on the Keys tab: restarts the first-run hints (`hints.rs`).
#[derive(Component)]
struct ResetHints;
#[derive(Component)]
struct DropdownList;
#[derive(Component)]
struct DropdownOption(Field, usize);
/// A slider's number or a dropdown's current choice.
#[derive(Component)]
struct ValueText(Field);

// The settings kit, drawn at `KIT_SCALE` like the rest of the HUD.
/// Narrow enough that the close button clears the top bar on a 1280 px window.
const WIDTH: f32 = 620.0;
/// Tabs a little tighter than the HUD's, so five fit.
const TAB_PADDING: f32 = 6.0;
const TITLE_SIZE: f32 = 32.0;
const TAB_ICON: f32 = 32.0;
const DIVIDER_IMAGE: &str = "ui/settings/divider.png";
const DIVIDER_HEIGHT: f32 = 17.0 * KIT_SCALE;
const DIVIDER_INSET: f32 = 8.0;
const TRACK_IMAGE: &str = "ui/settings/slider_track.png";
const TRACK_INSET: f32 = 7.0;
const TRACK_WIDTH: f32 = 260.0;
const TRACK_HEIGHT: f32 = 16.0 * KIT_SCALE;
const KNOB_IMAGE: &str = "ui/settings/slider_knob.png";
const KNOB_SIZE: f32 = 25.0 * KIT_SCALE;
const VALUE_WIDTH: f32 = 40.0;
const TOGGLE_ON: &str = "ui/settings/toggle_on.png";
const TOGGLE_OFF: &str = "ui/settings/toggle_off.png";
const TOGGLE_SIZE: Vec2 = Vec2::new(57.0 * KIT_SCALE, 29.0 * KIT_SCALE);
const DROPDOWN_IMAGE: &str = "ui/settings/dropdown.png";
const DROPDOWN_INSET: f32 = 10.0;
/// Track plus number: the control column, so sliders, toggles and dropdowns line up on the right.
const CONTROL_WIDTH: f32 = TRACK_WIDTH + SLOT_GAP * 2.0 + VALUE_WIDTH;
const DROPDOWN_HEIGHT: f32 = 29.0 * KIT_SCALE;
/// Room for the arrow on the dropdown's right end.
const DROPDOWN_ARROW: f32 = 28.0 * KIT_SCALE;
const OPTION_HEIGHT: f32 = 36.0;
const ROW_MIN_HEIGHT: f32 = 44.0;
/// Text on the dark dropdown.
const LIGHT_TEXT: Color = Color::srgb(0.96, 0.88, 0.7);
const CHOSEN_TEXT: Color = Color::srgb(1.0, 0.8, 0.3);
const LIST_COLOR: Color = Color::srgb(0.22, 0.14, 0.09);
/// Above every other HUD node.
const LIST_Z: i32 = 10;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        // A screenshot run shows the defaults and writes nothing; it may ask for a tab
        // (`GAME_SCREENSHOT_UI=settings-voice`).
        let request = screenshot::ui_request().unwrap_or_default();
        let tab = TABS
            .iter()
            .find(|(_, name, _)| request.strip_prefix("settings-") == Some(&name.to_lowercase()))
            .map_or(SettingsTab::Audio, |&(tab, _, _)| tab);
        if screenshot::screenshot_path().is_some() {
            app.insert_resource(Settings::default())
                .insert_resource(Unsaved);
        } else {
            let (settings, migrated) = config_dir().map_or((Settings::default(), false), |dir| {
                Settings::load_from(&dir)
            });
            if migrated {
                settings.save();
            }
            app.insert_resource(settings);
        }
        app.insert_resource(tab)
            .init_resource::<MicDevices>()
            .init_resource::<OpenDropdown>()
            .add_systems(Startup, spawn_settings)
            .add_systems(Update, (voice_key, click_settings).in_set(Phase::Input))
            .add_systems(
                Update,
                (
                    show_tab,
                    show_dropdown,
                    refresh_controls,
                    apply_settings,
                    save_settings.run_if(not(resource_exists::<Unsaved>)),
                )
                    .in_set(Phase::View),
            );
    }
}

fn row() -> Node {
    Node {
        min_height: px(ROW_MIN_HEIGHT),
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        ..default()
    }
}

/// The right-hand control column of a row.
fn control() -> Node {
    Node {
        width: px(CONTROL_WIDTH),
        align_items: AlignItems::Center,
        column_gap: px(SLOT_GAP * 2.0),
        ..default()
    }
}

fn slider_row(page: &mut ChildSpawnerCommands, assets: &AssetServer, name: &str, field: Field) {
    page.spawn(row()).with_children(|r| {
        r.spawn(ink(name));
        r.spawn(control()).with_children(|c| {
            let track = Node {
                width: px(TRACK_WIDTH),
                height: px(TRACK_HEIGHT),
                ..default()
            };
            let knob = Node {
                position_type: PositionType::Absolute,
                width: px(KNOB_SIZE),
                height: px(KNOB_SIZE),
                top: px((TRACK_HEIGHT - KNOB_SIZE) / 2.0),
                margin: UiRect::left(px(-KNOB_SIZE / 2.0)),
                ..default()
            };
            c.spawn((
                track,
                sliced(assets, TRACK_IMAGE, TRACK_INSET),
                Slider(field),
                Hovered::default(),
            ))
            .with_child((knob, ImageNode::new(assets.load(KNOB_IMAGE)), Knob(field)));
            c.spawn((ink(""), ValueText(field)));
        });
    });
}

fn toggle_row(page: &mut ChildSpawnerCommands, assets: &AssetServer, name: &str, field: Field) {
    page.spawn(row()).with_children(|r| {
        r.spawn(ink(name));
        r.spawn(control()).with_child((
            Node {
                width: px(TOGGLE_SIZE.x),
                height: px(TOGGLE_SIZE.y),
                ..default()
            },
            ImageNode::new(assets.load(TOGGLE_OFF)),
            Toggle(field),
            Hovered::default(),
        ));
    });
}

fn dropdown_row(page: &mut ChildSpawnerCommands, assets: &AssetServer, name: &str, field: Field) {
    page.spawn(row()).with_children(|r| {
        r.spawn(ink(name));
        r.spawn(control()).with_children(|c| {
            let node = Node {
                width: px(TRACK_WIDTH),
                height: px(DROPDOWN_HEIGHT),
                padding: UiRect::new(px(HUD_MARGIN * 2.0), px(DROPDOWN_ARROW), px(0), px(0)),
                align_items: AlignItems::Center,
                ..default()
            };
            c.spawn((
                node,
                sliced(assets, DROPDOWN_IMAGE, DROPDOWN_INSET),
                Dropdown(field),
                Hovered::default(),
            ))
            .with_child((label(""), TextColor(LIGHT_TEXT), ValueText(field)));
        });
    });
}

pub fn spawn_settings(mut commands: Commands, assets: Res<AssetServer>, tab: Res<SettingsTab>) {
    spawn_centre_panel(
        &mut commands,
        &assets,
        OpenPanel::Settings,
        WIDTH,
        |panel| {
            panel.spawn((
                Text::new("Settings"),
                TextFont::from_font_size(TITLE_SIZE),
                HudText,
                TextColor(INK_COLOR),
                Node {
                    align_self: AlignSelf::Center,
                    ..default()
                },
            ));
            let tabs = Node {
                column_gap: px(SLOT_GAP),
                justify_content: JustifyContent::Center,
                ..default()
            };
            panel.spawn(tabs).with_children(|tabs| {
                for (choice, name, icon) in TABS {
                    let (mut node, art) = tab_node(&assets, choice == *tab);
                    node.padding = UiRect::horizontal(px(TAB_PADDING));
                    node.column_gap = px(SLOT_GAP);
                    tabs.spawn((node, art, TabButton(choice), Hovered::default()))
                        .with_children(|t| {
                            t.spawn((
                                Node {
                                    width: px(TAB_ICON),
                                    height: px(TAB_ICON),
                                    ..default()
                                },
                                ImageNode::new(assets.load(format!("ui/settings/icon_{icon}.png"))),
                            ));
                            t.spawn(label(name));
                        });
                }
            });
            let mut divider = ImageNode::new(assets.load(DIVIDER_IMAGE));
            divider.image_mode = NodeImageMode::Sliced(TextureSlicer {
                border: BorderRect {
                    min_inset: Vec2::new(DIVIDER_INSET, 0.0),
                    max_inset: Vec2::new(DIVIDER_INSET, 0.0),
                },
                max_corner_scale: KIT_SCALE,
                ..default()
            });
            panel.spawn((
                Node {
                    height: px(DIVIDER_HEIGHT),
                    ..default()
                },
                divider,
            ));
            for (page, _, _) in TABS {
                let node = Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(SLOT_GAP),
                    display: if page == *tab {
                        Display::Flex
                    } else {
                        Display::None
                    },
                    ..default()
                };
                panel
                    .spawn((node, TabBody(page)))
                    .with_children(|p| match page {
                        SettingsTab::Audio => {
                            slider_row(p, &assets, "Music", Field::Music);
                            slider_row(p, &assets, "Sound effects", Field::Sfx);
                            slider_row(p, &assets, "Voice volume", Field::VoiceVolume);
                        }
                        SettingsTab::Voice => {
                            toggle_row(p, &assets, "Voice chat", Field::Voice);
                            toggle_row(p, &assets, "Push-to-talk (V)", Field::PushToTalk);
                            toggle_row(p, &assets, "Open mic", Field::OpenMic);
                            toggle_row(p, &assets, "Mute myself", Field::SelfMute);
                            dropdown_row(p, &assets, "Microphone", Field::Mic);
                            if !cfg!(feature = "voice") {
                                p.spawn(ink("This build has no voice chat"));
                            }
                        }
                        SettingsTab::Display => {
                            toggle_row(p, &assets, "Fullscreen", Field::Fullscreen);
                            dropdown_row(p, &assets, "Default zoom", Field::Zoom);
                        }
                        SettingsTab::Keys => {
                            for (action, keys) in BINDINGS {
                                p.spawn(Node {
                                    justify_content: JustifyContent::SpaceBetween,
                                    ..default()
                                })
                                .with_children(|r| {
                                    r.spawn(ink(action));
                                    r.spawn(ink(keys));
                                });
                            }
                            p.spawn((tab_node(&assets, true), ResetHints, Hovered::default()))
                                .with_child((label("Show tips again"), TextColor(LIGHT_TEXT)));
                        }
                        SettingsTab::Account => {
                            dropdown_row(p, &assets, "Skin tone", Field::Skin);
                            dropdown_row(p, &assets, "Language", Field::Language);
                        }
                    });
            }
        },
    );
}

/// N opens the settings on the Voice tab, or closes them when that tab is on show.
fn voice_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut open: ResMut<OpenPanel>,
    mut tab: ResMut<SettingsTab>,
) {
    if !keys.just_pressed(VOICE_KEY) {
        return;
    }
    if *open == OpenPanel::Settings && *tab == SettingsTab::Voice {
        *open = OpenPanel::None;
    } else {
        *open = OpenPanel::Settings;
        tab.set_if_neq(SettingsTab::Voice);
    }
}

fn hovered<'a, C: Component>(query: &'a Query<(&C, &Hovered)>) -> Option<&'a C> {
    query.iter().find(|(_, h)| h.get()).map(|(c, _)| c)
}

/// Clicks on tabs, toggles and dropdowns; pressing and dragging a slider.
#[allow(clippy::too_many_arguments)] // a Bevy system: five kinds of control and what they change
fn click_settings(
    mouse: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    open: Res<OpenPanel>,
    mics: Res<MicDevices>,
    tabs: Query<(&TabButton, &Hovered)>,
    toggles: Query<(&Toggle, &Hovered)>,
    dropdowns: Query<(&Dropdown, &Hovered)>,
    choices: Query<(&DropdownOption, &Hovered)>,
    sliders: Query<(&Slider, &Hovered, &ComputedNode, &UiGlobalTransform)>,
    resets: Query<&Hovered, With<ResetHints>>,
    mut tab: ResMut<SettingsTab>,
    mut dropdown: ResMut<OpenDropdown>,
    mut settings: ResMut<Settings>,
    mut dragging: Local<Option<Field>>,
    mut sfx: MessageWriter<Sfx>,
) {
    if *open != OpenPanel::Settings {
        if dropdown.0.is_some() {
            dropdown.0 = None;
        }
        *dragging = None;
        return;
    }
    let pointer = window.physical_cursor_position();
    let slide = |field: Field, settings: &mut Settings| {
        let track = sliders.iter().find(|(s, ..)| s.0 == field);
        if let (Some((_, _, node, transform)), Some(pointer)) = (track, pointer) {
            let left = transform.translation.x - node.size.x / 2.0;
            let value = slider_value(pointer.x, left, node.size.x);
            if settings.slider(field) != value {
                settings.set_slider(field, value);
            }
        }
    };
    if !mouse.pressed(MouseButton::Left) {
        *dragging = None;
    } else if let Some(field) = *dragging {
        slide(field, &mut settings);
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let on_control = hovered(&choices).is_some()
        || hovered(&dropdowns).is_some()
        || hovered(&toggles).is_some()
        || hovered(&tabs).is_some()
        || resets.iter().any(Hovered::get)
        || sliders.iter().any(|(_, h, ..)| h.get());
    if on_control {
        sfx.write(Sfx::UiClick);
    }
    if let Some(&DropdownOption(field, choice)) = hovered(&choices) {
        settings.choose(field, choice, &mics.0);
        dropdown.0 = None;
        return;
    }
    if let Some(&Dropdown(field)) = hovered(&dropdowns) {
        dropdown.0 = if dropdown.0 == Some(field) {
            None
        } else {
            Some(field)
        };
        return;
    }
    // A click anywhere else closes an open list (and still does what it is on).
    if dropdown.0.is_some() {
        dropdown.0 = None;
    }
    if let Some(&Toggle(field)) = hovered(&toggles) {
        let on = settings.toggle(field);
        settings.set_toggle(field, !on);
    } else if let Some(&TabButton(choice)) = hovered(&tabs) {
        tab.set_if_neq(choice);
    } else if resets.iter().any(Hovered::get) {
        settings.hints = 0;
    } else if let Some((slider, ..)) = sliders.iter().find(|(_, h, ..)| h.get()) {
        *dragging = Some(slider.0);
        slide(slider.0, &mut settings);
    }
}

fn show_tab(
    assets: Res<AssetServer>,
    tab: Res<SettingsTab>,
    mut bodies: Query<(&TabBody, &mut Node)>,
    mut buttons: Query<(&TabButton, &mut ImageNode)>,
) {
    if !tab.is_changed() {
        return;
    }
    for (body, mut node) in &mut bodies {
        node.display = if body.0 == *tab {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (button, mut image) in &mut buttons {
        *image = tab_node(&assets, button.0 == *tab).1;
    }
}

/// The open dropdown's list, below it and over everything else.
fn show_dropdown(
    mut commands: Commands,
    open: Res<OpenDropdown>,
    settings: Res<Settings>,
    mics: Res<MicDevices>,
    lists: Query<Entity, With<DropdownList>>,
    dropdowns: Query<(Entity, &Dropdown)>,
) {
    if !open.is_changed() {
        return;
    }
    for list in &lists {
        commands.entity(list).despawn();
    }
    let Some(field) = open.0 else { return };
    let Some((entity, _)) = dropdowns.iter().find(|(_, d)| d.0 == field) else {
        return;
    };
    let chosen = settings.choice(field, &mics.0);
    let list = (
        Node {
            position_type: PositionType::Absolute,
            top: percent(100),
            left: px(0),
            width: percent(100),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        BackgroundColor(LIST_COLOR),
        GlobalZIndex(LIST_Z),
        DropdownList,
    );
    commands.entity(entity).with_children(|parent| {
        parent.spawn(list).with_children(|list| {
            for (i, name) in options(field, &mics.0).into_iter().enumerate() {
                let color = if i == chosen { CHOSEN_TEXT } else { LIGHT_TEXT };
                list.spawn((
                    Node {
                        height: px(OPTION_HEIGHT),
                        padding: UiRect::horizontal(px(HUD_MARGIN * 2.0)),
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    DropdownOption(field, i),
                    Hovered::default(),
                ))
                .with_child((label(name), TextColor(color)));
            }
        });
    });
}

/// Knobs, numbers, toggle images and dropdown texts follow the settings.
fn refresh_controls(
    assets: Res<AssetServer>,
    settings: Res<Settings>,
    mics: Res<MicDevices>,
    mut knobs: Query<(&Knob, &mut Node)>,
    mut texts: Query<(&ValueText, &mut Text)>,
    mut toggles: Query<(&Toggle, &mut ImageNode)>,
) {
    if !settings.is_changed() && !mics.is_changed() {
        return;
    }
    for (knob, mut node) in &mut knobs {
        let left = percent(f32::from(settings.slider(knob.0)));
        if node.left != left {
            node.left = left;
        }
    }
    for (value, mut text) in &mut texts {
        let shown = match value.0 {
            Field::Music | Field::Sfx | Field::VoiceVolume => settings.slider(value.0).to_string(),
            field => options(field, &mics.0)
                .into_iter()
                .nth(settings.choice(field, &mics.0))
                .unwrap_or_default(),
        };
        if text.0 != shown {
            text.0 = shown;
        }
    }
    for (toggle, mut image) in &mut toggles {
        let file = if settings.toggle(toggle.0) {
            TOGGLE_ON
        } else {
            TOGGLE_OFF
        };
        let handle = assets.load(file);
        if image.image != handle {
            image.image = handle;
        }
    }
}

/// Window mode, camera zoom and the local player's skin follow the settings.
fn apply_settings(
    settings: Res<Settings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut looks: Query<&mut Look, With<Player>>,
    mut zoom: MessageWriter<SetZoom>,
    mut sent_zoom: Local<Option<usize>>,
) {
    if !settings.is_changed() {
        return;
    }
    let mode = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if window.mode != mode {
        window.mode = mode;
    }
    if *sent_zoom != Some(settings.zoom) {
        *sent_zoom = Some(settings.zoom);
        zoom.write(SetZoom(settings.zoom));
    }
    for mut look in &mut looks {
        if look.skin != settings.skin {
            look.skin = settings.skin;
        }
    }
}

fn save_settings(settings: Res<Settings>) {
    if settings.is_changed() && !settings.is_added() {
        settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("growsociaety-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn defaults_are_voice_off_push_to_talk_80_english_skin_0() {
        let s = Settings::parse("");
        assert_eq!(s, Settings::default());
        assert!(!s.voice && !s.open_mic && s.toggle(Field::PushToTalk));
        assert_eq!((s.music, s.sfx, s.voice_volume), (80, 80, 80));
        assert_eq!((s.skin, s.language, s.zoom), (0, 0, 0));
        assert!(s.to_text().contains("language=English"));
    }

    #[test]
    fn settings_file_roundtrip_and_bad_values_fall_back() {
        let s = Settings {
            music: 10,
            sfx: 0,
            voice_volume: 100,
            voice: true,
            open_mic: true,
            self_muted: true,
            mic: Some("USB Mic".into()),
            fullscreen: true,
            zoom: 2,
            skin: 3,
            language: 0,
            hints: 2,
            claim_tip_seen: true,
        };
        assert_eq!(Settings::parse(&s.to_text()), s);
        let dir = temp_dir("roundtrip");
        s.save_to(&dir).unwrap();
        assert_eq!(Settings::load_from(&dir), (s, false));
        let _ = std::fs::remove_dir_all(&dir);
        let odd = Settings::parse("music=900\nskin=99\nzoom=-1\nlanguage=Klingon\nmic=\ngarbage");
        assert_eq!(odd.music, MAX_VOLUME);
        assert_eq!((odd.skin, odd.zoom, odd.language), (0, 0, 0));
        assert!(odd.mic.is_none());
    }

    #[test]
    fn old_voice_txt_is_migrated_once() {
        let dir = temp_dir("migrate");
        std::fs::create_dir_all(&dir).unwrap();
        let old = "enabled=true\nopen_mic=true\nself_muted=false\nvolume=0.7\ndevice=USB Mic\n";
        std::fs::write(dir.join(OLD_VOICE_FILE), old).unwrap();
        let (s, migrated) = Settings::load_from(&dir);
        assert!(migrated);
        assert!(s.voice && s.open_mic && !s.self_muted);
        assert_eq!(s.voice_volume, 70);
        assert_eq!(s.mic.as_deref(), Some("USB Mic"));
        assert_eq!((s.music, s.sfx, s.skin), (80, 80, 0));
        // Once settings.txt exists it wins, even if voice.txt is still there.
        let changed = Settings { music: 5, ..s };
        changed.save_to(&dir).unwrap();
        assert_eq!(Settings::load_from(&dir), (changed, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_to_talk_and_open_mic_are_mutually_exclusive() {
        let mut s = Settings::default();
        for (field, on) in [
            (Field::OpenMic, true),
            (Field::PushToTalk, true),
            (Field::PushToTalk, false),
            (Field::OpenMic, false),
            (Field::OpenMic, true),
        ] {
            s.set_toggle(field, on);
            assert_eq!(s.toggle(field), on, "{field:?} {on}");
            assert_ne!(
                s.toggle(Field::PushToTalk),
                s.toggle(Field::OpenMic),
                "exactly one mode after {field:?} {on}"
            );
        }
    }

    #[test]
    fn slider_maps_pointer_x_to_0_100_clamped() {
        assert_eq!(slider_value(100.0, 100.0, 200.0), 0);
        assert_eq!(slider_value(200.0, 100.0, 200.0), 50);
        assert_eq!(slider_value(300.0, 100.0, 200.0), 100);
        assert_eq!(slider_value(-50.0, 100.0, 200.0), 0);
        assert_eq!(slider_value(9999.0, 100.0, 200.0), 100);
        assert_eq!(slider_value(150.0, 100.0, 0.0), 0);
    }

    #[test]
    fn microphone_choice_maps_to_the_device_list() {
        let mics = vec!["A".to_owned(), "B".to_owned()];
        let mut s = Settings::default();
        assert_eq!(s.choice(Field::Mic, &mics), 0);
        s.choose(Field::Mic, 2, &mics);
        assert_eq!(s.mic.as_deref(), Some("B"));
        assert_eq!(s.choice(Field::Mic, &mics), 2);
        assert_eq!(options(Field::Mic, &mics)[2], "B");
        s.choose(Field::Mic, 0, &mics);
        assert!(s.mic.is_none());
    }
}
