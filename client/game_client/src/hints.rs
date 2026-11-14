//! First-run hints (docs/knowledge/ux/first-run-hints.md): one box above the hotbar, three steps
//! in order (move and jump, punch, plant), each gone once the player has done it. Progress is
//! `Settings::hints`, saved with the settings. A step done early is remembered and, when it comes
//! up, shows only its check.

use bevy::prelude::*;
use game_shared::TILE_SIZE;
use game_shared::rules::{Action, Event};

use crate::audio::Sfx;
use crate::hud::{HUD_MARGIN, INK_COLOR, OpenPanel, SLOT_GAP, SLOT_SIZE, ink, sliced};
use crate::settings::Settings;
use crate::state::{ActionResult, Game, Phase};

const STEPS: usize = 3;
/// ponytail: copied from `JUMP_KEYS` / `LEFT_KEYS` / `RIGHT_KEYS` in player.rs (private there);
/// derive the labels from them when rebinding arrives. `·` is not in the pixel font, hence `-`.
const TEXTS: [&str; STEPS] = [
    "A / D to move - Space to jump",
    "Hold left mouse on a block to punch it",
    "Pick a seed in the hotbar and click soil to plant it",
];
const JUMP_KEYS: [KeyCode; 3] = [KeyCode::Space, KeyCode::KeyW, KeyCode::ArrowUp];
const MOVE_TILES: f32 = 3.0;

const FIRST_DELAY: f32 = 1.0;
const NEXT_DELAY: f32 = 1.5;
const FADE: f32 = 0.3;
const CHECK: f32 = 0.6;
/// The pixel font has no check mark.
const CHECK_TEXT: &str = "OK";
const CHECK_COLOR: Color = Color::srgb(0.25, 0.55, 0.15);

const BOX_IMAGE: &str = "ui/tooltip.png";
const BOX_INSET: f32 = 6.0;
const BOX_PADDING: f32 = 10.0;
/// Above the hotbar column (hotbar, item strip, toast line), so it never covers a toast.
const BOX_BOTTOM: f32 = HUD_MARGIN + SLOT_SIZE + 2.0 * (20.0 + SLOT_GAP) + 24.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    /// No local player yet.
    Idle,
    /// Box hidden; the current step appears at `until`.
    Waiting {
        until: f32,
    },
    Showing {
        since: f32,
    },
    /// `step` was finished at `since`: its check shows, then the box fades.
    Done {
        step: usize,
        since: f32,
    },
}

#[derive(Resource)]
struct Hints {
    stage: Stage,
    /// Steps whose condition has happened (possibly before they came up).
    done: [bool; STEPS],
    spawn: Option<(f32, f32)>,
    moved: bool,
    jumped: bool,
    /// `Settings::hints` as last seen, to notice "Show tips again".
    seen: u8,
}

#[derive(Component)]
struct HintBox;
#[derive(Component)]
struct HintText;
#[derive(Component)]
struct HintCounter;

pub struct HintsPlugin;

impl Plugin for HintsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_box)
            .add_systems(Update, (track, advance, show).chain().in_set(Phase::View));
    }
}

fn spawn_box(mut commands: Commands, assets: Res<AssetServer>, settings: Res<Settings>) {
    commands.insert_resource(Hints {
        stage: Stage::Idle,
        done: [false; STEPS],
        spawn: None,
        moved: false,
        jumped: false,
        seen: settings.hints,
    });
    let row = Node {
        position_type: PositionType::Absolute,
        left: px(0),
        right: px(0),
        bottom: px(BOX_BOTTOM),
        justify_content: JustifyContent::Center,
        ..default()
    };
    let frame = Node {
        padding: UiRect::all(px(BOX_PADDING)),
        column_gap: px(3.0 * BOX_PADDING),
        align_items: AlignItems::Center,
        ..default()
    };
    commands
        .spawn((row, Pickable::IGNORE))
        .with_children(|row| {
            row.spawn((
                frame,
                sliced(&assets, BOX_IMAGE, BOX_INSET),
                HintBox,
                Visibility::Hidden,
            ))
            .with_children(|frame| {
                frame.spawn((ink(""), HintText));
                frame.spawn((ink(""), HintCounter));
            });
        });
}

/// Records what the local player has done, whatever step is on show.
fn track(
    game: Res<Game>,
    keys: Res<ButtonInput<KeyCode>>,
    mut results: MessageReader<ActionResult>,
    mut hints: ResMut<Hints>,
) {
    let Some(me) = game.0.players.get(&game.1) else {
        return;
    };
    let pos = me.body.pos;
    let spawn = *hints.spawn.get_or_insert(pos);
    if (pos.0 - spawn.0).hypot(pos.1 - spawn.1) >= MOVE_TILES * TILE_SIZE {
        hints.moved = true;
    }
    if keys.any_just_pressed(JUMP_KEYS) {
        hints.jumped = true;
    }
    if hints.moved && hints.jumped {
        hints.done[0] = true;
    }
    for ActionResult { action, result } in results.read() {
        let Ok(events) = result else { continue };
        let any = |f: fn(&Event) -> bool| events.iter().any(f);
        match action {
            Action::Punch { .. }
                if any(|e| matches!(e, Event::TileDamaged { .. } | Event::TileChanged { .. })) =>
            {
                hints.done[1] = true;
            }
            Action::Place { .. } if any(|e| matches!(e, Event::TreePlanted { .. })) => {
                hints.done[2] = true;
            }
            _ => {}
        }
    }
}

fn advance(
    time: Res<Time>,
    game: Res<Game>,
    mut settings: ResMut<Settings>,
    mut hints: ResMut<Hints>,
    mut sfx: MessageWriter<Sfx>,
) {
    let now = time.elapsed_secs();
    if settings.hints < hints.seen {
        // "Show tips again": start over from where the player stands now.
        *hints = Hints {
            stage: Stage::Waiting {
                until: now + FIRST_DELAY,
            },
            done: [false; STEPS],
            spawn: None,
            moved: false,
            jumped: false,
            seen: settings.hints,
        };
    }
    hints.seen = settings.hints;
    let step = usize::from(settings.hints);
    match hints.stage {
        Stage::Idle if game.0.players.contains_key(&game.1) => {
            hints.stage = Stage::Waiting {
                until: now + FIRST_DELAY,
            };
        }
        Stage::Waiting { until } if step < STEPS && now >= until => {
            hints.stage = Stage::Showing { since: now };
        }
        Stage::Showing { .. } if step < STEPS && hints.done[step] => {
            hints.stage = Stage::Done { step, since: now };
            // Saved at once, so quitting during the check never shows this step again.
            settings.hints = (settings.hints + 1).min(STEPS as u8);
            hints.seen = settings.hints;
            sfx.write(Sfx::UiClick);
        }
        Stage::Done { since, .. } if now >= since + CHECK + FADE => {
            hints.stage = Stage::Waiting {
                until: now + NEXT_DELAY,
            };
        }
        _ => {}
    }
}

/// Opacity of the box at `now`.
fn alpha(stage: Stage, now: f32) -> f32 {
    match stage {
        Stage::Idle | Stage::Waiting { .. } => 0.0,
        Stage::Showing { since } => ((now - since) / FADE).clamp(0.0, 1.0),
        Stage::Done { since, .. } => (1.0 - (now - since - CHECK) / FADE).clamp(0.0, 1.0),
    }
}

fn show(
    time: Res<Time>,
    hints: Res<Hints>,
    settings: Res<Settings>,
    open: Res<OpenPanel>,
    mut frame: Single<(&mut Visibility, &mut ImageNode), With<HintBox>>,
    mut text: Single<(&mut Text, &mut TextColor), (With<HintText>, Without<HintCounter>)>,
    mut counter: Single<(&mut Text, &mut TextColor), (With<HintCounter>, Without<HintText>)>,
) {
    let a = alpha(hints.stage, time.elapsed_secs());
    let (step, label, colour) = match hints.stage {
        Stage::Done { step, .. } => (step, CHECK_TEXT.to_string(), CHECK_COLOR),
        _ => {
            let step = usize::from(settings.hints);
            (step, format!("{}/{STEPS}", step + 1), INK_COLOR)
        }
    };
    let hidden = a <= 0.0 || step >= STEPS || *open != OpenPanel::None;
    *frame.0 = if hidden {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if hidden {
        return;
    }
    frame.1.color = Color::WHITE.with_alpha(a);
    if text.0.0 != TEXTS[step] {
        text.0.0 = TEXTS[step].to_string();
    }
    text.1.0 = INK_COLOR.with_alpha(a);
    counter.0.0 = label;
    counter.1.0 = colour.with_alpha(a);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_fades_in_holds_the_check_then_fades_out() {
        assert_eq!(alpha(Stage::Waiting { until: 5.0 }, 9.0), 0.0);
        assert_eq!(alpha(Stage::Showing { since: 1.0 }, 1.0), 0.0);
        assert_eq!(alpha(Stage::Showing { since: 1.0 }, 9.0), 1.0);
        let done = Stage::Done {
            step: 0,
            since: 1.0,
        };
        assert_eq!(alpha(done, 1.0 + CHECK), 1.0);
        assert_eq!(alpha(done, 1.0 + CHECK + FADE), 0.0);
    }
}
