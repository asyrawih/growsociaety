//! Window icon on Windows and Linux (assets/README.md, "Game icon"). macOS ignores a window's icon;
//! there it comes from the app bundle's `icon.icns` (release packaging).

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::NonSendMarker;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::winit::WINIT_WINDOWS;

const ICON: &str = "icon/icon_64.png";

pub struct IconPlugin;

impl Plugin for IconPlugin {
    fn build(&self, app: &mut App) {
        if !cfg!(target_os = "macos") {
            app.add_systems(Update, set_icon);
        }
    }
}

/// The icon as winit wants it: RGBA8 pixels and their size.
fn load_icon() -> Option<winit::window::Icon> {
    let path = crate::player::asset_root().join(ICON);
    let bytes = std::fs::read(&path).ok()?;
    let image = Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::MAIN_WORLD,
    )
    .ok()?
    .convert(TextureFormat::Rgba8UnormSrgb)?;
    let (width, height) = (image.width(), image.height());
    winit::window::Icon::from_rgba(image.data?, width, height).ok()
}

/// Runs on the main thread (winit requires it) until the window exists, then sets the icon once.
fn set_icon(_main_thread: NonSendMarker, mut done: Local<bool>) {
    if *done {
        return;
    }
    WINIT_WINDOWS.with_borrow(|windows| {
        if windows.windows.is_empty() {
            return;
        }
        *done = true;
        let Some(icon) = load_icon() else {
            warn!("window icon {ICON} could not be loaded");
            return;
        };
        for window in windows.windows.values() {
            window.set_window_icon(Some(icon.clone()));
        }
    });
}
