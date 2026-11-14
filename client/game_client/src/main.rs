use bevy::prelude::*;

mod almanac;
mod audio;
mod avatar_menu;
mod camera;
mod chat;
mod chat_bubbles;
mod claim_ui;
mod craft_ui;
mod farming_ui;
mod foliage;
mod hints;
mod hud;
mod icon;
mod interact;
mod inventory_filter;
mod levels_ui;
mod lobby;
mod net;
mod player;
mod respawn_ui;
mod screenshot;
mod settings;
mod state;
mod trade_ui;
mod vfx;
#[cfg(feature = "voice")]
mod voice;
mod wardrobe;
mod world_select;
mod world_view;

/// Where `assets/` is, relative to Bevy's base path (`CARGO_MANIFEST_DIR` under `cargo run`, else
/// the executable's directory): the repo root in development, next to the binary in a zip, or in
/// `Contents/Resources` of the macOS `.app`.
const ASSET_DIRS: [&str; 3] = ["../../assets", "assets", "../Resources/assets"];

/// The first of `ASSET_DIRS` that exists, decided once.
pub fn asset_dir() -> &'static str {
    static DIR: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = bevy::asset::io::file::FileAssetReader::get_base_path();
        ASSET_DIRS
            .into_iter()
            .find(|dir| base.join(dir).is_dir())
            .unwrap_or(ASSET_DIRS[0])
    })
}

/// Engine plugins, shared by the lobby window and the game.
fn engine_plugins() -> impl PluginGroup {
    let window = Window {
        title: "Growsociety".into(),
        // A screenshot run must not take the keyboard away from whoever is using the machine.
        focused: screenshot::screenshot_path().is_none(),
        ..default()
    };
    DefaultPlugins
        .set(ImagePlugin::default_nearest())
        .set(AssetPlugin {
            file_path: asset_dir().into(),
            ..default()
        })
        .set(WindowPlugin {
            primary_window: Some(window),
            ..default()
        })
        .set(bevy::log::LogPlugin {
            custom_layer: log_file,
            ..default()
        })
}

/// The log also goes to a file: started from Finder or by the lobby, stdout goes nowhere.
/// `~/Library/Logs/Growsociety/client.log` on macOS, else `client.log` in the settings dir.
/// Appended to, so a world switch (the game re-starts itself) keeps the old process's lines; each
/// session starts with a separator line. Emptied once it passes `LOG_CAP` bytes.
/// Bytes. ponytail: one file, emptied when full; rotate to `client.old.log` if history matters.
const LOG_CAP: u64 = 2 * 1024 * 1024;

/// `~/Library/Logs/Growsociety/client.log` on macOS, else `client.log` in the settings dir.
fn log_path() -> Option<std::path::PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| std::path::PathBuf::from(home).join("Library/Logs/Growsociety"))
    } else {
        settings::config_dir()
    }?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("client.log"))
}

/// A panic also lands in the log file (message and location), then the default hook runs. A game
/// started by the lobby or a world switch has no terminal to show it.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(mut file) = log_path().and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        }) {
            use std::io::Write;
            let _ = writeln!(file, "PANIC: {info}");
        }
        default(info);
    }));
}

fn log_file(_: &mut App) -> Option<bevy::log::BoxedLayer> {
    let path = log_path()?;
    let full = std::fs::metadata(&path).is_ok_and(|meta| meta.len() > LOG_CAP);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(!full)
        .write(true)
        .truncate(full)
        .open(&path)
        .ok()?;
    let world = std::env::var("GAME_WORLD").unwrap_or_else(|_| "START".into());
    use std::io::Write;
    let _ = writeln!(
        file,
        "--- session start pid {}, world {world} ---",
        std::process::id()
    );
    let layer = bevy::log::tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file));
    Some(Box::new(layer))
}

fn main() {
    log_panics();
    // livekit's wss:// (rustls without a provider) needs a process-wide one; ring is already used by quinn.
    let _ = rustls::crypto::ring::default_provider().install_default();
    // Online without credentials: log in first; the lobby restarts the game with a token.
    if lobby::wanted() {
        lobby::run(engine_plugins());
        return;
    }
    let mut app = App::new();
    app.add_plugins(engine_plugins()).add_plugins((
        // Before `StatePlugin`: online, the world comes from the server.
        net::NetPlugin,
        state::StatePlugin,
        world_view::WorldViewPlugin,
        foliage::FoliagePlugin,
        player::PlayerPlugin,
        interact::InteractPlugin,
        wardrobe::WardrobePlugin,
        hud::HudPlugin,
        // Before `VoicePlugin`, which reads and may adjust the settings.
        settings::SettingsPlugin,
        camera::CameraPlugin,
        chat::ChatPlugin,
        screenshot::DebugScreenshotPlugin,
    ));
    app.add_plugins((
        trade_ui::TradeUiPlugin,
        levels_ui::LevelsUiPlugin,
        craft_ui::CraftUiPlugin,
        claim_ui::ClaimUiPlugin,
        hints::HintsPlugin,
        respawn_ui::RespawnUiPlugin,
        farming_ui::FarmingUiPlugin,
    ));
    app.add_plugins((audio::AudioPlugin, icon::IconPlugin));
    app.add_plugins((
        vfx::VfxPlugin,
        inventory_filter::InventoryFilterPlugin,
        almanac::AlmanacPlugin,
        world_select::WorldSelectPlugin,
        chat_bubbles::ChatBubblesPlugin,
        avatar_menu::AvatarMenuPlugin,
    ));
    // After `NetPlugin` (voice is online only) and `HudPlugin` (its panel uses the HUD's).
    #[cfg(feature = "voice")]
    app.add_plugins(voice::VoicePlugin);
    app.run();
}
