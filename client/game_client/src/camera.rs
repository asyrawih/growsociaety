//! Camera: follows the player with frame-rate independent smoothing, clamped to the world, pixel snapped.
//! Mouse wheel or +/- steps through whole-number zoom levels, a pure view setting. A window too big
//! for the world at some zoom never gets it (`min_zoom`), so the view never shows past the edges.

use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use game_shared::{TILE_SIZE, WORLD_HEIGHT, WORLD_WIDTH};

use crate::player::Player;

/// Per second. `pos += (target - pos) * (1 - exp(-CAMERA_STIFFNESS * dt))`.
const CAMERA_STIFFNESS: f32 = 10.0;

/// Screen px per world px. Whole numbers keep the pixel art crisp.
pub const ZOOM_LEVELS: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
/// Index into `ZOOM_LEVELS` at startup, and the zoom a screenshot run shows: 2x, where a 48 px
/// character reads well.
const DEFAULT_ZOOM: usize = 1;
const ZOOM_IN_KEYS: [KeyCode; 2] = [KeyCode::Equal, KeyCode::NumpadAdd];
const ZOOM_OUT_KEYS: [KeyCode; 2] = [KeyCode::Minus, KeyCode::NumpadSubtract];
/// Trackpads scroll in pixels; this many count as one wheel notch.
const PIXELS_PER_NOTCH: f32 = 50.0;

/// The zoom in use (screen px per world px), for things that keep a fixed on-screen size.
#[derive(Resource, Clone, Copy)]
pub struct ViewZoom(pub f32);

impl Default for ViewZoom {
    fn default() -> Self {
        ViewZoom(ZOOM_LEVELS[DEFAULT_ZOOM])
    }
}

/// Unrounded camera position (the `Transform` gets the pixel-snapped copy) and the chosen zoom level
/// index. The zoom actually used is never below `min_zoom` for the current window.
#[derive(Component)]
struct FollowCamera {
    pos: Vec2,
    zoom: usize,
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SetZoom>()
            .init_resource::<ViewZoom>()
            .add_systems(Startup, spawn_camera.after(crate::player::spawn_player))
            .add_systems(
                Update,
                (
                    zoom_camera.run_if(|| crate::screenshot::screenshot_path().is_none()),
                    follow_player,
                )
                    .chain()
                    .after(crate::state::Phase::View),
            );
    }
}

fn spawn_camera(mut commands: Commands, player: Single<&Player>) {
    commands.spawn((
        Camera2d,
        FollowCamera {
            pos: player.position,
            zoom: DEFAULT_ZOOM,
        },
    ));
}

/// The zoom level index after `notches` steps (positive = in), kept inside `min..ZOOM_LEVELS.len()`.
fn step_zoom(zoom: usize, notches: i32, min: usize) -> usize {
    (zoom as i32 + notches).clamp(min as i32, ZOOM_LEVELS.len() as i32 - 1) as usize
}

fn world_size() -> Vec2 {
    Vec2::new(WORLD_WIDTH as f32, WORLD_HEIGHT as f32) * TILE_SIZE
}

/// The smallest zoom index at which a `window` (logical px) sees no more than the world on both
/// axes; the largest one if even that is not enough.
fn min_zoom(window: Vec2) -> usize {
    let world = world_size();
    ZOOM_LEVELS
        .iter()
        .position(|&zoom| {
            let view = window / zoom;
            view.x <= world.x && view.y <= world.y
        })
        .unwrap_or(ZOOM_LEVELS.len() - 1)
}

/// Asks for a zoom level by index into `ZOOM_LEVELS` (e.g. from a settings panel); out-of-range
/// indices are clamped.
#[derive(Message, Clone, Copy)]
pub struct SetZoom(pub usize);

fn zoom_camera(
    mut requests: MessageReader<SetZoom>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut wheel: Local<f32>,
    window: Single<&Window>,
    ui: Query<&bevy::picking::hover::Hovered>,
    mut follow: Single<&mut FollowCamera>,
) {
    let min = min_zoom(window.size());

    // Over a HUD panel the wheel scrolls the panel, not the camera; +/- still zoom.
    if crate::hud::pointer_over_ui(&ui) {
        *wheel = 0.0;
    } else {
        *wheel += match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_NOTCH,
        };
    }
    let notches = wheel.trunc();
    *wheel -= notches;
    let keys = i32::from(keys.any_just_pressed(ZOOM_IN_KEYS))
        - i32::from(keys.any_just_pressed(ZOOM_OUT_KEYS));
    let base = requests
        .read()
        .last()
        .map_or(follow.zoom, |&SetZoom(level)| {
            step_zoom(0, level as i32, min)
        });
    let zoom = step_zoom(base, notches as i32 + keys, min);
    if zoom != follow.zoom {
        follow.zoom = zoom;
    }
}

/// Keeps a view of `view_len` inside `0..world_len`; centres it when the view is the larger one.
fn clamp_axis(pos: f32, view_len: f32, world_len: f32) -> f32 {
    let half = view_len / 2.0;
    if view_len >= world_len {
        world_len / 2.0
    } else {
        pos.clamp(half, world_len - half)
    }
}

fn follow_player(
    time: Res<Time>,
    window: Single<&Window>,
    player: Single<&Player>,
    camera: Single<(&mut FollowCamera, &mut Transform, &mut Projection)>,
    mut view_zoom: ResMut<ViewZoom>,
) {
    let (mut follow, mut transform, mut projection) = camera.into_inner();
    // `Window::size` is logical px (Retina-independent), the same unit the projection uses. The
    // minimum is re-read every frame, so a resize or fullscreen toggle applies at once; the chosen
    // level comes back when the window shrinks again.
    let window = window.size();
    let zoom = ZOOM_LEVELS[follow.zoom.max(min_zoom(window))];
    if view_zoom.0 != zoom {
        view_zoom.0 = zoom;
    }
    if let Projection::Orthographic(ortho) = &mut *projection
        && ortho.scale != 1.0 / zoom
    {
        ortho.scale = 1.0 / zoom;
    }
    let target = player.position;
    let pos =
        follow.pos + (target - follow.pos) * (1.0 - (-CAMERA_STIFFNESS * time.delta_secs()).exp());
    // Clamp every frame, not only on movement, so a window resize or zoom change is handled too.
    let (view, world) = (window / zoom, world_size());
    follow.pos = Vec2::new(
        clamp_axis(pos.x, view.x, world.x),
        clamp_axis(pos.y, view.y, world.y),
    );
    transform.translation = snap(follow.pos, zoom).extend(transform.translation.z);
}

/// Whole screen pixels only: avoids tile seams and shimmering.
fn snap(pos: Vec2, zoom: f32) -> Vec2 {
    (pos * zoom).round() / zoom
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_stays_inside_the_world_or_is_centred() {
        assert_eq!(clamp_axis(-50.0, 1280.0, 3200.0), 640.0);
        assert_eq!(clamp_axis(9999.0, 1280.0, 3200.0), 2560.0);
        assert_eq!(clamp_axis(1000.0, 1280.0, 3200.0), 1000.0);
        assert_eq!(clamp_axis(123.0, 4000.0, 3200.0), 1600.0);
    }

    #[test]
    fn zoomed_view_is_window_over_zoom() {
        // 1280x720 window at 4x shows 320x180 world px.
        let view = Vec2::new(1280.0, 720.0) / ZOOM_LEVELS[3];
        assert_eq!(clamp_axis(-50.0, view.x, 3200.0), 160.0);
        assert_eq!(clamp_axis(9999.0, view.x, 3200.0), 3040.0);
        // At 2x a 1600-wide world fits a 3200 px window: centred.
        assert_eq!(clamp_axis(10.0, 3200.0 / ZOOM_LEVELS[1], 1200.0), 600.0);
    }

    #[test]
    fn zoom_steps_stay_in_range() {
        assert_eq!(ZOOM_LEVELS[DEFAULT_ZOOM], 2.0);
        assert_eq!(step_zoom(0, -1, 0), 0);
        assert_eq!(step_zoom(0, 2, 0), 2);
        assert_eq!(step_zoom(3, 5, 0), ZOOM_LEVELS.len() - 1);
        // Never below the window's minimum.
        assert_eq!(step_zoom(2, -5, 1), 1);
    }

    #[test]
    fn a_big_window_never_sees_past_the_world() {
        // The world is 3200x1600 px.
        assert_eq!(min_zoom(Vec2::new(1280.0, 720.0)), 0);
        // Fullscreen 1920x1080 fits at 1x; a 3840x2160 (logical) screen is wider than the world
        // at 1x, so it needs 2x.
        assert_eq!(min_zoom(Vec2::new(1920.0, 1080.0)), 0);
        assert_eq!(min_zoom(Vec2::new(3840.0, 2160.0)), 1);
        assert_eq!(
            min_zoom(Vec2::new(1700.0, 1700.0)),
            1,
            "taller than the world"
        );
        assert_eq!(min_zoom(Vec2::new(99999.0, 99999.0)), ZOOM_LEVELS.len() - 1);
    }

    #[test]
    fn snap_lands_on_screen_pixels() {
        assert_eq!(snap(Vec2::new(10.3, 7.0), 1.0), Vec2::new(10.0, 7.0));
        assert_eq!(snap(Vec2::new(10.3, 7.1), 4.0), Vec2::new(10.25, 7.0));
    }
}
