//! Going home, the client's side (docs/knowledge/ux/respawn.md): the hold ring on the player
//! (the R hold itself is `player::hold_home`), the Settings "Go home" row's click and cooldown
//! label, and the feedback on `Event::Respawned`: puffs, the screen overlay (red on a hazard, dark on going home) and the hazard toast.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use game_shared::items::Collision;
use game_shared::items::{ItemId, LAVA, item};
use game_shared::physics::spawn_body;
use game_shared::rules::{Action, Event, RespawnCause};
use game_shared::world_to_tile;

use crate::audio::Sfx;
use crate::hud::{INK_COLOR, Notice, OpenPanel, TextFocus};
use crate::player::{HomeCooldown, HomeHold, Player};
use crate::state::{ActionMsg, Game, GameEvent, Phase};
use crate::world_view::Z_PLAYER;

/// The key `player::hold_home` reads; here only for the cooldown toast.
const HOME_KEY: KeyCode = KeyCode::KeyR;

const RING_IMAGE: &str = "ui/respawn_ring.png";
const RING_FRAMES: u32 = 8;
const RING_SIZE: u32 = 24;

const PUFF_IMAGE: &str = "fx/respawn_puff.png";
const PUFF_FRAMES: usize = 6;
const PUFF_SIZE: u32 = 32;
/// Per frame, as in `fx/respawn_puff.json` (67 ms).
const PUFF_FRAME_SECS: f32 = 0.067;
/// In front of the players.
const Z_FX: f32 = Z_PLAYER + 0.8;

/// Hazard flash: full red for `FLASH_HOLD_SECS`, then a fade to clear over `FLASH_FADE_SECS`.
const FLASH_HOLD_SECS: f32 = 0.15;
const FLASH_FADE_SECS: f32 = 0.25;
const FLASH_ALPHA: f32 = 0.6;
const FLASH_RGB: (f32, f32, f32) = (0.85, 0.1, 0.05);
/// Going home: a dark overlay up to `FADE_ALPHA` and back down over `FADE_SECS`.
const FADE_SECS: f32 = 0.3;
const FADE_ALPHA: f32 = 0.4;
const FADE_RGB: (f32, f32, f32) = (0.0, 0.0, 0.0);

/// The Settings row is greyed this much during the cooldown.
const GREY_ALPHA: f32 = 0.4;

const LAVA_TOAST: &str = "Burned by lava · back at the door";
const HAZARD_TOAST: &str = "Ouch · back at the door";

/// The Settings "Go home" row (spawned by `settings.rs`), and its label.
#[derive(Component)]
pub struct GoHomeButton;
#[derive(Component)]
pub struct GoHomeLabel;
#[derive(Component)]
pub struct GoHomeIcon;

#[derive(Resource)]
struct FxArt {
    ring: Handle<Image>,
    ring_layout: Handle<TextureAtlasLayout>,
    puff: Handle<Image>,
    puff_layout: Handle<TextureAtlasLayout>,
}

#[derive(Component)]
struct Ring;

/// A puff playing; `started` is `Time::elapsed_secs`.
#[derive(Component)]
struct Puff {
    started: f32,
}

/// The full-screen overlay: red flash (hazard) or dark fade (home). When it started
/// (`Time::elapsed_secs`) and why; `None` when idle.
#[derive(Component, Default)]
struct Flash {
    started: Option<(f32, RespawnCause)>,
}

pub struct RespawnUiPlugin;

impl Plugin for RespawnUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_fx)
            .add_systems(Update, (click_go_home, cooldown_toast).in_set(Phase::Input))
            .add_systems(
                Update,
                (on_respawned, draw_ring, play_puffs, draw_flash, refresh_row)
                    .chain()
                    .in_set(Phase::View),
            );
    }
}

fn spawn_fx(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let mut strip = |size, frames| {
        layouts.add(TextureAtlasLayout::from_grid(
            UVec2::splat(size),
            frames,
            1,
            None,
            None,
        ))
    };
    let art = FxArt {
        ring: assets.load(RING_IMAGE),
        ring_layout: strip(RING_SIZE, RING_FRAMES),
        puff: assets.load(PUFF_IMAGE),
        puff_layout: strip(PUFF_SIZE, PUFF_FRAMES as u32),
    };
    commands.spawn((
        Ring,
        Sprite::from_atlas_image(
            art.ring.clone(),
            TextureAtlas {
                layout: art.ring_layout.clone(),
                index: 0,
            },
        ),
        Transform::default(),
        Visibility::Hidden,
    ));
    commands.insert_resource(art);
    // Over everything, invisible to picking so it never counts as "pointer over the UI".
    commands.spawn((
        Flash::default(),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(i32::MAX - 3),
        Pickable::IGNORE,
        Hovered::default(),
    ));
}

/// `Go home (7s)` during the cooldown, else `Go home`.
fn row_label(left: f32) -> String {
    if left > 0.0 {
        format!("Go home ({}s)", left.ceil() as u32)
    } else {
        "Go home".to_owned()
    }
}

fn cooldown_text(left: f32) -> String {
    format!("You can go home again in {}s", left.ceil() as u32)
}

/// Ring frame for a hold progress (0 = hidden, `None`), fill 1/8 .. full.
fn ring_frame(progress: f32) -> Option<usize> {
    (progress > 0.0).then(|| {
        ((progress * RING_FRAMES as f32).ceil() as usize).clamp(1, RING_FRAMES as usize) - 1
    })
}

/// How long the overlay for `cause` lasts.
fn flash_secs(cause: RespawnCause) -> f32 {
    match cause {
        RespawnCause::Hazard => FLASH_HOLD_SECS + FLASH_FADE_SECS,
        RespawnCause::Home => FADE_SECS,
    }
}

/// The overlay's alpha `t` seconds after it started.
fn flash_alpha(cause: RespawnCause, t: f32) -> f32 {
    if !(0.0..flash_secs(cause)).contains(&t) {
        return 0.0;
    }
    match cause {
        RespawnCause::Hazard if t < FLASH_HOLD_SECS => FLASH_ALPHA,
        RespawnCause::Hazard => FLASH_ALPHA * (1.0 - (t - FLASH_HOLD_SECS) / FLASH_FADE_SECS),
        // Out and in: up to the peak at half time, then back down.
        RespawnCause::Home => FADE_ALPHA * (1.0 - (2.0 * t / FADE_SECS - 1.0).abs()),
    }
}

/// One click on the Settings row: go home, or say how long to wait.
fn click_go_home(
    mouse: Res<ButtonInput<MouseButton>>,
    mut open: ResMut<OpenPanel>,
    cooldown: Res<HomeCooldown>,
    rows: Query<&Hovered, With<GoHomeButton>>,
    mut actions: MessageWriter<ActionMsg>,
    mut notices: MessageWriter<Notice>,
    mut sfx: MessageWriter<Sfx>,
) {
    if *open != OpenPanel::Settings
        || !mouse.just_pressed(MouseButton::Left)
        || !rows.iter().any(Hovered::get)
    {
        return;
    }
    let left = cooldown.0;
    if left > 0.0 {
        sfx.write(Sfx::Denied);
        notices.write(Notice(cooldown_text(left)));
    } else {
        sfx.write(Sfx::UiClick);
        actions.write(ActionMsg(Action::Respawn));
        *open = OpenPanel::None;
    }
}

/// Pressing R during the cooldown says how long is left (once per press).
fn cooldown_toast(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<TextFocus>,
    cooldown: Res<HomeCooldown>,
    mut notices: MessageWriter<Notice>,
) {
    let left = cooldown.0;
    if left > 0.0 && !focus.0 && keys.just_pressed(HOME_KEY) {
        notices.write(Notice(cooldown_text(left)));
    }
}

/// The hazard under or right around `pos`, for the toast's wording.
fn hazard_near(game: &Game, pos: Vec2) -> Option<ItemId> {
    let (tx, ty) = world_to_tile(pos.x, pos.y)?;
    let near = |v: u32| v.saturating_sub(1)..=v + 1;
    near(ty)
        .flat_map(|y| near(tx).map(move |x| (x, y)))
        .filter_map(|(x, y)| game.0.world.get(x, y))
        .map(|tile| tile.fg)
        .find(|&fg| item(fg).is_some_and(|def| def.collision == Collision::Hazard))
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per thing a respawn shows
fn on_respawned(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    art: Res<FxArt>,
    mut events: MessageReader<GameEvent>,
    mut flash: Single<&mut Flash>,
    mut notices: MessageWriter<Notice>,
    mut sfx: MessageWriter<Sfx>,
) {
    let now = time.elapsed_secs();

    let door = Vec2::from(spawn_body(&game.0.world).pos);
    let mut puff = |at: Vec2| {
        commands.spawn((
            Puff { started: now },
            Sprite::from_atlas_image(
                art.puff.clone(),
                TextureAtlas {
                    layout: art.puff_layout.clone(),
                    index: 0,
                },
            ),
            Transform::from_translation(at.extend(Z_FX)),
        ));
    };
    for GameEvent(event) in events.read() {
        let &Event::Respawned {
            player: who,
            cause,
            from,
        } = event
        else {
            continue;
        };
        let old = Vec2::from(from);
        puff(door);
        puff(old);
        if who != game.1 {
            continue;
        }
        sfx.write(Sfx::Door);
        flash.started = Some((now, cause));
        if cause == RespawnCause::Hazard {
            let text = match hazard_near(&game, old) {
                Some(LAVA) => LAVA_TOAST,
                _ => HAZARD_TOAST,
            };
            notices.write(Notice(text.to_owned()));
        }
    }
}

fn draw_ring(
    hold: Res<HomeHold>,
    cooldown: Res<HomeCooldown>,
    player: Single<&Player>,
    ring: Single<(&mut Sprite, &mut Transform, &mut Visibility), With<Ring>>,
) {
    let (mut sprite, mut transform, mut visibility) = ring.into_inner();
    // During the cooldown the ring does not start (the hold still runs in player.rs; the
    // server refuses it).
    let frame = ring_frame(hold.0).filter(|_| cooldown.0 <= 0.0);
    let Some(frame) = frame else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    visibility.set_if_neq(Visibility::Inherited);
    if let Some(atlas) = &mut sprite.texture_atlas {
        atlas.index = frame;
    }
    // Centred on the body, in front of the rig.
    transform.translation = player.position.extend(Z_FX);
}

fn play_puffs(
    mut commands: Commands,
    time: Res<Time>,
    mut puffs: Query<(Entity, &Puff, &mut Sprite)>,
) {
    let now = time.elapsed_secs();
    for (entity, puff, mut sprite) in &mut puffs {
        let frame = ((now - puff.started) / PUFF_FRAME_SECS) as usize;
        if frame >= PUFF_FRAMES {
            commands.entity(entity).despawn();
        } else if let Some(atlas) = &mut sprite.texture_atlas {
            atlas.index = frame;
        }
    }
}

fn draw_flash(time: Res<Time>, flash: Single<(&mut Flash, &mut Node, &mut BackgroundColor)>) {
    let (mut flash, mut node, mut color) = flash.into_inner();
    let Some((started, cause)) = flash.started else {
        return;
    };
    let t = time.elapsed_secs() - started;
    if t >= flash_secs(cause) {
        flash.started = None;
        node.display = Display::None;
        return;
    }
    node.display = Display::Flex;
    let alpha = flash_alpha(cause, t);
    let (r, g, b) = match cause {
        RespawnCause::Hazard => FLASH_RGB,
        RespawnCause::Home => FADE_RGB,
    };
    color.0 = Color::srgba(r, g, b, alpha);
}

/// The row counts down and greys out during the cooldown.
fn refresh_row(
    cooldown: Res<HomeCooldown>,
    mut labels: Query<(&mut Text, &mut TextColor), With<GoHomeLabel>>,
    mut icons: Query<&mut ImageNode, With<GoHomeIcon>>,
) {
    let left = cooldown.0;
    let text = row_label(left);
    let alpha = if left > 0.0 { GREY_ALPHA } else { 1.0 };
    for (mut label, mut color) in &mut labels {
        if label.0 != text {
            label.0.clone_from(&text);
            color.0 = INK_COLOR.with_alpha(alpha);
        }
    }
    for mut icon in &mut icons {
        if icon.color.alpha() != alpha {
            icon.color = Color::WHITE.with_alpha(alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_counts_down_in_whole_seconds() {
        assert_eq!(row_label(0.0), "Go home");
        assert_eq!(row_label(6.2), "Go home (7s)");
        assert_eq!(row_label(7.0), "Go home (7s)");
        assert_eq!(row_label(0.01), "Go home (1s)");
        assert_eq!(cooldown_text(6.5), "You can go home again in 7s");
    }

    #[test]
    fn the_ring_fills_in_eighths_and_hides_at_zero() {
        assert_eq!(ring_frame(0.0), None);
        assert_eq!(ring_frame(0.01), Some(0));
        assert_eq!(ring_frame(0.125), Some(0));
        assert_eq!(ring_frame(0.5), Some(3));
        assert_eq!(ring_frame(1.0), Some(7));
    }

    #[test]
    fn the_hazard_flash_holds_then_fades_to_clear() {
        let alpha = |t| flash_alpha(RespawnCause::Hazard, t);
        assert_eq!(alpha(-0.1), 0.0);
        assert_eq!(alpha(0.0), FLASH_ALPHA);
        assert_eq!(alpha(0.14), FLASH_ALPHA);
        let mid = alpha(FLASH_HOLD_SECS + FLASH_FADE_SECS / 2.0);
        assert!((mid - FLASH_ALPHA / 2.0).abs() < 1e-4, "{mid}");
        assert_eq!(alpha(FLASH_HOLD_SECS + FLASH_FADE_SECS), 0.0);
        assert_eq!(alpha(5.0), 0.0);
    }

    #[test]
    fn going_home_fades_dark_out_and_back_in() {
        let alpha = |t| flash_alpha(RespawnCause::Home, t);
        assert_eq!(alpha(0.0), 0.0);
        assert!((alpha(FADE_SECS / 2.0) - FADE_ALPHA).abs() < 1e-4);
        assert!((alpha(FADE_SECS / 4.0) - FADE_ALPHA / 2.0).abs() < 1e-4);
        assert!((alpha(FADE_SECS * 0.75) - FADE_ALPHA / 2.0).abs() < 1e-4);
        assert_eq!(alpha(FADE_SECS), 0.0);
        assert_eq!(flash_secs(RespawnCause::Home), FADE_SECS);
    }
}
