//! QC tool: `GAME_SCREENSHOT=/path/out.png cargo run -p game_client` saves one frame shortly after
//! startup. In that mode the window opens unfocused and all keyboard and mouse input is ignored, so
//! the run neither steals keystrokes nor is steered by them.

use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use game_shared::commands::Admin;
use std::ffi::OsString;

use crate::state::{AdminMsg, Game};

const SCREENSHOT_ENV: &str = "GAME_SCREENSHOT";
/// Seconds after startup; long enough for the images to load.
const SCREENSHOT_DELAY: f32 = 3.0;

/// `GAME_SCREENSHOT_UI=inventory|wardrobe|store|store-<tab>|settings-<tab>` (tab label in lower
/// case, e.g. `store-pickaxes`, `settings-voice`) opens that panel at startup, so it can be captured.
/// `trade` and `trade-prompt` open an offline trade with a stand-in partner (`trade_ui.rs`).
const SCREENSHOT_UI_ENV: &str = "GAME_SCREENSHOT_UI";

/// `GAME_SCREENSHOT_FILL=1` (screenshot runs, offline only) gives the local player everything
/// through the admin `/giveall` path, so the scrolling grids can be captured full.
const SCREENSHOT_FILL_ENV: &str = "GAME_SCREENSHOT_FILL";

/// The panel a screenshot run asked for, if any.
pub fn ui_request() -> Option<String> {
    screenshot_path()?;
    std::env::var(SCREENSHOT_UI_ENV).ok()
}

/// Where to save the frame, if this is a screenshot run.
pub fn screenshot_path() -> Option<OsString> {
    std::env::var_os(SCREENSHOT_ENV)
}

#[derive(Resource)]
struct ScreenshotPath(OsString);

pub struct DebugScreenshotPlugin;

impl Plugin for DebugScreenshotPlugin {
    fn build(&self, app: &mut App) {
        if let Some(path) = screenshot_path() {
            app.insert_resource(ScreenshotPath(path))
                .add_systems(PreUpdate, ignore_input.after(InputSystems))
                .add_systems(Startup, fill_inventory)
                .add_systems(Update, take_screenshot);
        }
    }
}

fn fill_inventory(
    game: Res<Game>,
    net: Option<Res<crate::net::Net>>,
    mut admins: MessageWriter<AdminMsg>,
) {
    if net.is_none() && std::env::var_os(SCREENSHOT_FILL_ENV).is_some() {
        admins.write(AdminMsg(Admin::GiveAll { to: game.1 }));
    }
}

fn ignore_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    keys.reset_all();
    mouse.reset_all();
}

fn take_screenshot(
    mut commands: Commands,
    time: Res<Time>,
    path: Res<ScreenshotPath>,
    mut taken: Local<bool>,
) {
    if !*taken && time.elapsed_secs() >= SCREENSHOT_DELAY {
        *taken = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.0.clone()));
    }
}
