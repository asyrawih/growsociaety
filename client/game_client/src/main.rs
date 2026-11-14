use bevy::prelude::*;

mod audio;
mod camera;
mod chat;
mod claim_ui;
mod craft_ui;
mod farming_ui;
mod foliage;
mod hints;
mod hud;
mod icon;
mod interact;
mod levels_ui;
mod lobby;
mod net;
mod player;
mod respawn_ui;
mod screenshot;
mod settings;
mod state;
mod trade_ui;
#[cfg(feature = "voice")]
mod voice;
mod wardrobe;
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
}

fn main() {
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
    // After `NetPlugin` (voice is online only) and `HudPlugin` (its panel uses the HUD's).
    #[cfg(feature = "voice")]
    app.add_plugins(voice::VoicePlugin);
    app.run();
}
