//! Speech bubbles over players who talk in world chat (docs/knowledge/ux/chat-bubbles.md). A view:
//! one bubble per speaker, the newest line replaces it. The bubble sits above the name plate and is
//! counter-scaled like the names, so it reads the same at every zoom. Off in Settings > Display (`Chat bubbles`).

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use bevy::text::{FontSmoothing, LineHeight};
use game_shared::TILE_SIZE;
use game_shared::physics::HITBOX_HEIGHT;
use game_shared::rules::PlayerId;

use crate::camera::ViewZoom;
use crate::hud::{FONT, INK_COLOR, TOOLTIP_IMAGE, TOOLTIP_INSET};
use crate::player::rig::{CANVAS, FEET_Y};
use crate::player::{NAME_FONT_SIZE, NAME_GAP, Player, RemotePlayer};
use crate::settings::Settings;
use crate::state::{Game, Phase};
use crate::world_view::Z_PLAYER;

/// A world-chat line someone said (the local player included), with the exact text the server
/// broadcast. Written by `chat.rs` for every `ChatKind::World` line (growsociaety-1f; until then
/// nothing writes it and no bubble shows).
#[derive(Message, Clone, Debug)]
pub struct SaidInWorld {
    pub from: PlayerId,
    pub text: String,
}

const BUBBLE_IMAGE: &str = "ui/bubble.png";
const BUBBLE_INSET: f32 = 6.0;
const TAIL_IMAGE: &str = "ui/bubble_tail.png";
/// The tail art is 10x6; drawn at `ART_SCALE` like the HUD kit.
const TAIL_SIZE: Vec2 = Vec2::new(10.0 * ART_SCALE, 6.0 * ART_SCALE);
const ART_SCALE: f32 = 2.0;
/// The tail tucks this far under the box's bottom edge, screen px.
const TAIL_OVERLAP: f32 = 2.0 * ART_SCALE;

/// The name text's bottom edge above the hitbox centre, world px (rig root offset + half the
/// canvas + the gap, as in `player::spawn_rig`).
const NAME_BOTTOM: f32 =
    CANVAS / 2.0 - (CANVAS - FEET_Y) - HITBOX_HEIGHT / 2.0 + CANVAS / 2.0 + NAME_GAP;
/// Between the name plate and the tail tip, screen px.
const ABOVE_NAME: f32 = 4.0;

/// Screen px.
pub const MAX_WIDTH: f32 = 220.0;
pub const MAX_LINES: usize = 3;
const ELLIPSIS: &str = "...";
const PADDING: f32 = 8.0;
const LINE_HEIGHT: f32 = 18.0;

const BASE_SECS: f32 = 3.0;
const SECS_PER_CHAR: f32 = 0.06;
const MIN_SECS: f32 = 3.0;
const MAX_SECS: f32 = 6.0;
const FADE_IN_SECS: f32 = 0.1;
const FADE_OUT_SECS: f32 = 0.3;

/// Speakers further than this from the camera centre get no bubble.
const MAX_DISTANCE: f32 = 20.0 * TILE_SIZE;
/// Above names and voice icons; each new bubble a hair higher, so the newest is on top.
const BUBBLE_Z: f32 = Z_PLAYER + 2.0;
const Z_STEP: f32 = 0.0001;
const Z_STEPS: u32 = 1000;

/// Advance of each printable ASCII glyph (space..=`~`) of `growsociaety_pixel.ttf` at 16 px.
/// Read from the font's hmtx table; other characters count as `OTHER_WIDTH`.
const GLYPH_WIDTHS: [u8; 95] = [
    6, 5, 9, 13, 11, 15, 13, 5, 8, 8, 11, 11, 5, 11, 5, 11, 11, 9, 11, 11, 11, 11, 11, 11, 11, 11,
    5, 5, 11, 11, 11, 11, 13, 13, 13, 13, 13, 11, 11, 13, 13, 9, 13, 13, 11, 15, 13, 13, 13, 13,
    13, 13, 13, 13, 13, 17, 13, 13, 13, 7, 6, 7, 6, 11, 6, 11, 11, 11, 11, 11, 10, 11, 11, 7, 8,
    11, 9, 17, 11, 11, 11, 11, 10, 11, 9, 11, 13, 17, 13, 11, 11, 6, 5, 6, 6,
];
const OTHER_WIDTH: f32 = 13.0;

fn char_width(c: char) -> f32 {
    match c {
        ' '..='~' => f32::from(GLYPH_WIDTHS[c as usize - ' ' as usize]),
        _ => OTHER_WIDTH,
    }
}

pub fn text_width(text: &str) -> f32 {
    text.chars().map(char_width).sum()
}

/// `3 s + 0.06 s per character`, clamped to 3..=6 s.
pub fn lifetime(text: &str) -> f32 {
    (BASE_SECS + SECS_PER_CHAR * text.chars().count() as f32).clamp(MIN_SECS, MAX_SECS)
}

/// Opacity `age` seconds into a bubble that lives `life` seconds.
pub fn alpha(age: f32, life: f32) -> f32 {
    (age / FADE_IN_SECS)
        .min((life - age) / FADE_OUT_SECS)
        .clamp(0.0, 1.0)
}

/// Word-wrapped lines no wider than `max` px; a word longer than a line breaks. More than
/// `MAX_LINES` lines: the last kept one ends in `...`.
pub fn wrap(text: &str, max: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let joined = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if text_width(&joined) <= max {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        for c in word.chars() {
            if !line.is_empty() && text_width(&line) + char_width(c) > max {
                lines.push(std::mem::take(&mut line));
            }
            line.push(c);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > MAX_LINES {
        lines.truncate(MAX_LINES);
        let last = &mut lines[MAX_LINES - 1];
        while !last.is_empty() && text_width(last) + text_width(ELLIPSIS) > max {
            last.pop();
        }
        let kept = last.trim_end().len();
        last.truncate(kept);
        last.push_str(ELLIPSIS);
    }
    lines
}

/// Whether a speaker at `speaker` gets a bubble: on screen (`screen`, its viewport point, inside
/// `viewport`) and within `MAX_DISTANCE` of the camera centre.
pub fn shown(speaker: Vec2, centre: Vec2, screen: Option<Vec2>, viewport: Vec2) -> bool {
    let on_screen =
        screen.is_some_and(|p| p.x >= 0.0 && p.y >= 0.0 && p.x <= viewport.x && p.y <= viewport.y);
    on_screen && speaker.distance(centre) <= MAX_DISTANCE
}

#[derive(Component)]
struct Bubble {
    from: PlayerId,
    said_at: f32,
    life: f32,
    /// Hitbox centre of the speaker's rig, world px.
    at: Vec2,
}

/// The art in use: the bubble kit, or the tooltip box (no tail) while it is missing.
#[derive(Resource)]
struct BubbleArt {
    font: Handle<Font>,
    box_image: Handle<Image>,
    inset: f32,
    tail: Option<Handle<Image>>,
}

impl FromWorld for BubbleArt {
    fn from_world(world: &mut World) -> Self {
        let assets = world.resource::<AssetServer>();
        let have = |file: &str| crate::player::asset_root().join(file).is_file();
        let (box_file, inset) = if have(BUBBLE_IMAGE) {
            (BUBBLE_IMAGE, BUBBLE_INSET)
        } else {
            (TOOLTIP_IMAGE, TOOLTIP_INSET)
        };
        Self {
            font: assets.load(FONT),
            box_image: assets.load(box_file),
            inset,
            tail: have(TAIL_IMAGE).then(|| assets.load(TAIL_IMAGE)),
        }
    }
}

pub struct ChatBubblesPlugin;

impl Plugin for ChatBubblesPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SaidInWorld>()
            .init_resource::<BubbleArt>()
            .add_systems(
                Update,
                (spawn_bubbles, place_bubbles).chain().in_set(Phase::View),
            );
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn spawn_bubbles(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<Settings>,
    art: Res<BubbleArt>,
    game: Res<Game>,
    mut said: MessageReader<SaidInWorld>,
    bubbles: Query<(Entity, &Bubble)>,
    mut count: Local<u32>,
) {
    if !settings.chat_bubbles {
        said.clear();
        for (entity, _) in &bubbles {
            commands.entity(entity).despawn();
        }
        return;
    }
    let now = time.elapsed_secs();
    // The newest line per speaker this frame.
    let mut latest: HashMap<PlayerId, &str> = HashMap::new();
    for line in said.read() {
        latest.insert(line.from, &line.text);
    }
    for (from, text) in latest {
        let Some(player) = game.0.players.get(&from) else {
            continue;
        };
        let mut at = Vec2::from(player.body.pos);
        for (entity, bubble) in &bubbles {
            if bubble.from == from {
                at = bubble.at;
                commands.entity(entity).despawn();
            }
        }
        let lines = wrap(text, MAX_WIDTH);
        if lines.is_empty() {
            continue;
        }
        let width = lines.iter().map(|l| text_width(l)).fold(0.0, f32::max);
        let size = Vec2::new(width, lines.len() as f32 * LINE_HEIGHT) + 2.0 * PADDING;
        let box_y = TAIL_SIZE.y - TAIL_OVERLAP + size.y / 2.0;
        *count = (*count + 1) % Z_STEPS;
        let z = BUBBLE_Z + *count as f32 * Z_STEP;
        let slicer = TextureSlicer {
            border: BorderRect::all(art.inset),
            max_corner_scale: ART_SCALE,
            ..default()
        };
        commands
            .spawn((
                Bubble {
                    from,
                    said_at: now,
                    life: lifetime(text),
                    at,
                },
                Transform::from_xyz(at.x, at.y, z),
                Visibility::Hidden,
            ))
            .with_children(|bubble| {
                bubble.spawn((
                    Sprite {
                        image: art.box_image.clone(),
                        custom_size: Some(size),
                        image_mode: SpriteImageMode::Sliced(slicer),
                        ..default()
                    },
                    Transform::from_xyz(0.0, box_y, 0.0),
                ));
                if let Some(tail) = &art.tail {
                    bubble.spawn((
                        Sprite {
                            image: tail.clone(),
                            custom_size: Some(TAIL_SIZE),
                            ..default()
                        },
                        Anchor::BOTTOM_CENTER,
                        // Over the box's outline, so the tail joins it.
                        Transform::from_xyz(0.0, 0.0, Z_STEP / 2.0),
                    ));
                }
                bubble.spawn((
                    Text2d::new(lines.join("\n")),
                    TextFont::from_font_size(NAME_FONT_SIZE)
                        .with_font(art.font.clone())
                        .with_font_smoothing(FontSmoothing::None),
                    LineHeight::Px(LINE_HEIGHT),
                    TextLayout::justify(Justify::Center).with_no_wrap(),
                    TextColor(INK_COLOR),
                    Transform::from_xyz(0.0, box_y, Z_STEP / 2.0),
                ));
            });
    }
}

/// Follows each speaker, counter-scales, culls, fades, and ends bubbles.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn place_bubbles(
    mut commands: Commands,
    time: Res<Time>,
    zoom: Res<ViewZoom>,
    game: Res<Game>,
    local: Query<&Player>,
    remotes: Query<&RemotePlayer>,
    camera: Single<(&Camera, &GlobalTransform)>,
    mut bubbles: Query<(
        Entity,
        &mut Bubble,
        &mut Transform,
        &mut Visibility,
        &Children,
    )>,
    mut sprites: Query<&mut Sprite>,
    mut texts: Query<&mut TextColor>,
) {
    let now = time.elapsed_secs();
    let inverse = 1.0 / zoom.0.max(1.0);
    let (camera, camera_transform) = *camera;
    let centre = camera_transform.translation().truncate();
    let viewport = camera.logical_viewport_size().unwrap_or_default();
    for (entity, mut bubble, mut transform, mut visibility, children) in &mut bubbles {
        let age = now - bubble.said_at;
        // The rig's own (interpolated, smoothed) position, so the bubble moves with the name.
        let rig = if bubble.from == game.1 {
            local.single().ok().map(|me| me.position)
        } else {
            remotes
                .iter()
                .find(|r| r.id == bubble.from)
                .map(|r| r.position)
        };
        let (true, Some(at)) = (age < bubble.life, rig) else {
            commands.entity(entity).despawn();
            continue;
        };
        bubble.at = at;
        let screen = camera
            .world_to_viewport(camera_transform, bubble.at.extend(0.0))
            .ok();
        let next = if shown(bubble.at, centre, screen, viewport) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(next);
        let y = bubble.at.y + NAME_BOTTOM + (NAME_FONT_SIZE + ABOVE_NAME) * inverse;
        transform.translation.x = bubble.at.x;
        transform.translation.y = y;
        transform.scale = Vec3::new(inverse, inverse, 1.0);
        let a = alpha(age, bubble.life);
        for &child in children {
            if let Ok(mut sprite) = sprites.get_mut(child) {
                sprite.color.set_alpha(a);
            }
            if let Ok(mut color) = texts.get_mut(child) {
                color.0.set_alpha(a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifetime_grows_per_character_within_3_to_6_seconds() {
        assert_eq!(lifetime(""), 3.0);
        assert_eq!(lifetime("hi"), 3.0 + 0.12);
        assert!((lifetime(&"a".repeat(25)) - 4.5).abs() < 1e-4);
        assert_eq!(lifetime(&"a".repeat(200)), 6.0);
        assert_eq!(alpha(0.0, 4.0), 0.0);
        assert_eq!(alpha(0.05, 4.0), 0.5);
        assert_eq!(alpha(2.0, 4.0), 1.0);
        assert!((alpha(3.85, 4.0) - 0.5).abs() < 1e-4);
        assert_eq!(alpha(4.0, 4.0), 0.0);
    }

    #[test]
    fn wraps_on_words_breaks_long_words_and_caps_at_three_lines() {
        let fits = |lines: &[String]| lines.iter().all(|l| text_width(l) <= MAX_WIDTH);
        assert_eq!(wrap("hello world", MAX_WIDTH), vec!["hello world"]);
        let two = wrap("anyone selling dirt seeds? paying well", MAX_WIDTH);
        assert_eq!(two.len(), 2, "{two:?}");
        assert!(fits(&two));
        assert_eq!(two.join(" "), "anyone selling dirt seeds? paying well");
        let long = wrap(&"W".repeat(30), MAX_WIDTH);
        assert!(long.len() > 1 && fits(&long), "{long:?}");
        assert_eq!(long.concat(), "W".repeat(30));
        let many = wrap(&"word ".repeat(60), MAX_WIDTH);
        assert_eq!(many.len(), MAX_LINES);
        assert!(many[2].ends_with("...") && fits(&many), "{many:?}");
        assert!(wrap("   ", MAX_WIDTH).is_empty());
        // Exactly three lines: no ellipsis.
        let three = wrap(
            &format!("{} {} {}", "a".repeat(15), "b".repeat(15), "c".repeat(15)),
            MAX_WIDTH,
        );
        assert_eq!(three.len(), 3);
        assert!(!three[2].ends_with("..."));
    }

    #[test]
    fn offscreen_and_far_speakers_get_no_bubble() {
        let view = Vec2::new(1280.0, 720.0);
        let centre = Vec2::new(1000.0, 1000.0);
        assert!(shown(centre, centre, Some(Vec2::new(640.0, 360.0)), view));
        assert!(!shown(centre, centre, None, view));
        assert!(!shown(centre, centre, Some(Vec2::new(-1.0, 10.0)), view));
        assert!(!shown(centre, centre, Some(Vec2::new(10.0, 721.0)), view));
        let far = centre + Vec2::X * (MAX_DISTANCE + 1.0);
        assert!(!shown(far, centre, Some(Vec2::new(10.0, 10.0)), view));
        let near = centre + Vec2::X * MAX_DISTANCE;
        assert!(shown(near, centre, Some(Vec2::new(10.0, 10.0)), view));
    }
}
