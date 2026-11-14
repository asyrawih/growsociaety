//! "Level N!" when the local player levels up (docs/knowledge/14). The level and XP bar sit by
//! the gem counter (`hud.rs`).

use bevy::prelude::*;
use game_shared::rules::Event;

use crate::hud::HudText;
use crate::state::{Game, GameEvent, Phase};

const TOAST_SECS: f32 = 2.5;
const TOAST_FONT_SIZE: f32 = 32.0;
const TOAST_TOP_PERCENT: f32 = 25.0;

/// The local player levelled up: `audio.rs` plays `sfx/level_up.ogg` on this.
#[derive(Message, Clone, Copy)]
pub struct LevelUpSound;

#[derive(Component)]
struct LevelToast {
    until: f32,
}

pub struct LevelsUiPlugin;

impl Plugin for LevelsUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LevelUpSound>()
            .add_systems(Startup, spawn_toast)
            .add_systems(Update, show_level_up.in_set(Phase::View));
    }
}

fn spawn_toast(mut commands: Commands) {
    let row = Node {
        position_type: PositionType::Absolute,
        left: px(0),
        right: px(0),
        top: percent(TOAST_TOP_PERCENT),
        justify_content: JustifyContent::Center,
        ..default()
    };
    commands.spawn(row).with_children(|row| {
        row.spawn((
            Text::default(),
            TextFont::from_font_size(TOAST_FONT_SIZE),
            TextColor(Color::srgb(1.0, 0.85, 0.3)),
            HudText,
            LevelToast { until: 0.0 },
        ));
    });
}

fn show_level_up(
    time: Res<Time>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut sounds: MessageWriter<LevelUpSound>,
    toast: Single<(&mut Text, &mut LevelToast)>,
) {
    let (mut text, mut toast) = toast.into_inner();
    let level = events.read().filter_map(|GameEvent(event)| match *event {
        Event::LevelUp { player, level } if player == game.1 => Some(level),
        _ => None,
    });
    let level = level.last();
    let now = time.elapsed_secs();
    if let Some(level) = level {
        text.0 = format!("Level {level}!");
        toast.until = now + TOAST_SECS;
        sounds.write(LevelUpSound);
    } else if !text.0.is_empty() && now >= toast.until {
        text.0.clear();
    }
}
