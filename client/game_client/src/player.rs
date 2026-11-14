//! The local player: keyboard -> `Input`, `physics::step` in `FixedUpdate`, interpolated code-animated rig.
//! No movement rule lives here; the body only changes through `step`, and pickups through `collect_drops`.

pub mod rig;

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::sprite::{Anchor, Text2dShadow};
use bevy::text::TextLayoutInfo;
use game_shared::items::{SLOT_COUNT, item};
use game_shared::physics::{Body, HITBOX_HEIGHT, Input, TICK_RATE, on_ladder, step_with};
use game_shared::rules::{Action, PlayerId, collect_drops};
use game_shared::tile_to_world;

use crate::net::Net;
use crate::state::{ActionMsg, Game, GameEvent, Phase};
use crate::world_view::Z_PLAYER;
use rig::{CANVAS, FEET_Y, Motion, Part, Slot};

/// Rig root (canvas centre) relative to the hitbox centre, so the feet line sits on the hitbox bottom.
const SPRITE_OFFSET_Y: f32 = CANVAS / 2.0 - (CANVAS - FEET_Y) - HITBOX_HEIGHT / 2.0;
/// Px/s. Slower than this on the ground counts as standing still.
const IDLE_SPEED: f32 = 5.0;
/// Z step between body parts; item layers sit between a part and the next one.
const PART_Z: f32 = 0.01;
const ITEM_Z: f32 = 0.001;
/// Below the torso by more than its own z, so wings are behind arm_back.
const WINGS_Z: f32 = 3.5 * PART_Z;
/// Over the front arm's item layers, under the next part.
const HELD_Z: f32 = 8.0 * ITEM_Z;
const _: () = assert!(
    6.0 * ITEM_Z < HELD_Z && HELD_Z < PART_Z,
    "item layers must stay under the next part"
);

/// View state of the local player. The body itself lives in the `WorldState`.
#[derive(Component)]
pub struct Player {
    /// The body before the latest tick; the rig is drawn between it and the current one.
    previous: Body,
    /// Interpolated hitbox centre, world px. What the camera follows.
    pub position: Vec2,
    pub facing_west: bool,
    /// The front arm reaching out to a tile (punch or place), started on input.
    reach: Option<rig::Reach>,
}

/// What a rig wears: one `assets/items` folder name per slot (index = `Slot as usize`), copied
/// from the player's `equipped` by `follow_equipment`. Changing it re-dresses the rig (and, for
/// the local player, re-skins it).
#[derive(Component, Default)]
pub struct Look {
    /// Index into `rig::SKIN_RAMPS`.
    pub skin: usize,
    pub slots: [Option<&'static str>; SLOT_COUNT],
}

/// The folder names of what `player` wears. Item names are the folder names (docs/knowledge/03).
fn worn_folders(player: &game_shared::rules::Player) -> [Option<&'static str>; SLOT_COUNT] {
    player
        .equipped
        .map(|id| id.and_then(item).map(|def| def.name))
}

/// Overhead name: plain text a few px above the head, with a 1 px drop shadow for sky and blocks.
/// Gap between the top edge of the 48 px rig (which tall hats and hair reach) and the name, world px.
const NAME_GAP: f32 = 4.0;
/// assets/README.md, "Font": glyphs on a 16 px grid, so 16 px and no smoothing keep them crisp.
const NAME_FONT: &str = "fonts/growsociaety_pixel.ttf";
const NAME_FONT_SIZE: f32 = 16.0;
const NAME_SHADOW: Text2dShadow = Text2dShadow {
    offset: Vec2::new(1.0, -1.0),
    color: Color::srgba(0.0, 0.0, 0.0, 0.85),
};
/// Above every part of the rig.
const NAME_Z: f32 = 0.5;
/// Shown offline, unless `GAME_NAME` holds a valid name.
const OFFLINE_NAME: &str = "Player";

/// Name colour by player level, the one place it is decided. There is no level yet, so everyone
/// is white; when levels exist, pass the player's level and add bands here.
fn name_color(_level: Option<u32>) -> Color {
    Color::WHITE
}

/// Holds a rig's name above its head, child of the root. Its child is the `NameText`.
#[derive(Component)]
struct Nameplate;

#[derive(Component)]
struct NameText;

/// The country flag left of a name, child of the `Nameplate`; holds its code.
#[derive(Component)]
struct Flag(String);

/// Flags are about 16x11 (assets/README.md, "Flags"); the few narrower ones stay centred in that
/// box because sprites are drawn centred. Gap between the flag and the name, px.
const FLAG_WIDTH: f32 = 16.0;
const FLAG_GAP: f32 = 3.0;

/// The wings layer of a rig, child of its torso; `draw_*` sets its flap.
#[derive(Component)]
struct Wings;

/// Another player in an online world, drawn from the bodies the server sends.
#[derive(Component)]
struct RemotePlayer {
    id: PlayerId,
    /// Smoothed towards the latest server body, world px.
    position: Vec2,
    facing_west: bool,
    /// Their front arm reaching out, from the server's `Swing`.
    reach: Option<rig::Reach>,
}

/// A rig's front arm is fully out at tile `(x, y)`: the moment of contact. Hit particles
/// (`vfx.rs`) can spawn on it so they line up with the fist; `player` is whose arm it is.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct ArmContact {
    pub player: PlayerId,
    pub x: u32,
    pub y: u32,
    pub place: bool,
}

/// The skin-coloured strip from the shoulder to the reached-out arm, child of the rig root.
#[derive(Component)]
struct ArmFill;
/// Px.
const ARM_FILL_WIDTH: f32 = 3.0;

/// Per second, like the camera: `pos += (target - pos) * (1 - exp(-REMOTE_STIFFNESS * dt))`.
const REMOTE_STIFFNESS: f32 = 15.0;

/// A body part entity of a rig, child of the rig root. Its children are item layers.
#[derive(Component)]
struct RigPart(Part);

/// This frame's pose of a rig, on its root: per part (rotation, dy) and the wing flap.
/// `pose_parts` moves the parts and wings to it.
/// The third field is how far (px) the front arm is pushed out along itself (`rig::Reach`).
#[derive(Component, Default)]
struct RigPose([(f32, f32); 6], f32, f32);

/// Body layers as loaded (light skin) and the recoloured copies the sprites draw, indexed like `Part::ALL`.
/// Other views (e.g. the wardrobe preview) draw `skinned` to show the local player's skin tone.
#[derive(Resource)]
pub struct RigImages {
    source: [Handle<Image>; 6],
    /// Body layers already recoloured to the local player's `Look.skin`, indexed like `Part::ALL`.
    pub skinned: [Handle<Image>; 6],
    /// Ramp the skinned copies currently hold.
    skin: Option<usize>,
    name_font: Handle<Font>,
}

/// Jump presses seen in `Update`, kept until exactly one fixed tick has consumed them: Space, and
/// W / ↑ (a jump only off a ladder; on one they climb).
#[derive(Resource, Default)]
struct JumpPressed {
    space: bool,
    up_key: bool,
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(f64::from(TICK_RATE)))
            .init_resource::<JumpPressed>()
            .add_message::<ArmContact>()
            .add_systems(Startup, spawn_player)
            .add_systems(FixedUpdate, tick_player)
            .init_resource::<HomeHold>()
            .init_resource::<HomeCooldown>()
            .add_systems(Update, (latch_jump, hold_home).in_set(Phase::Input))
            .add_systems(
                Update,
                (
                    start_reach,
                    sync_remotes,
                    follow_equipment,
                    dress_rig,
                    skin_rig,
                    draw_player,
                    draw_remotes,
                    pose_parts,
                    name_rigs,
                    place_names,
                    place_flags,
                )
                    .chain()
                    .in_set(Phase::View),
            );
    }
}

const JUMP_KEY: KeyCode = KeyCode::Space;
/// Climb up a ladder; off a ladder they jump too (docs/knowledge/ux/ladders.md).
const UP_KEYS: [KeyCode; 2] = [KeyCode::KeyW, KeyCode::ArrowUp];
/// Hold to go home (docs/knowledge/ux/respawn.md).
const HOME_KEY: KeyCode = KeyCode::KeyR;
/// Seconds `HOME_KEY` must be held.
const HOME_HOLD: f32 = 1.0;

/// How far the go-home hold is, `0..=1` (0 = not holding). The HUD draws the ring from it.
#[derive(Resource, Default)]
pub struct HomeHold(pub f32);

/// Seconds until going home is allowed again, 0 when ready: the one client-side cooldown clock
/// (from our own last `Respawned`). The HUD's "Go home (7s)" label reads it.
#[derive(Resource, Default)]
pub struct HomeCooldown(pub f32);

/// Drop through one-way platforms (docs/knowledge/ux/platforms.md); climb down a ladder.
const DOWN_KEYS: [KeyCode; 2] = [KeyCode::KeyS, KeyCode::ArrowDown];
const LEFT_KEYS: [KeyCode; 2] = [KeyCode::KeyA, KeyCode::ArrowLeft];
const RIGHT_KEYS: [KeyCode; 2] = [KeyCode::KeyD, KeyCode::ArrowRight];

/// Where a part's pivot sits relative to the rig root (canvas centre), world axes.
fn pivot_offset(part: Part) -> Vec2 {
    let [x, y] = part.pivot().map(|v| v as f32);
    Vec2::new(x - CANVAS / 2.0, CANVAS / 2.0 - y)
}

pub fn spawn_player(
    mut commands: Commands,
    assets: Res<AssetServer>,
    images: Res<Assets<Image>>,
    game: Res<Game>,
) {
    let Some(me) = game.me() else { return };
    let rig = RigImages {
        source: Part::ALL.map(|part| assets.load(format!("player_rig/{}.png", part.name()))),
        skinned: Part::ALL.map(|_| images.reserve_handle()),
        skin: None,
        name_font: assets.load(NAME_FONT),
    };
    let position = Vec2::from(me.body.pos);
    let player = Player {
        previous: me.body,
        position,
        facing_west: false,
        reach: None,
    };
    spawn_rig(&mut commands, &rig, position, (player, Look::default()));
    commands.insert_resource(rig);
}

/// Where assets are read from, as Bevy resolves it (see `main.rs`).
pub fn asset_root() -> std::path::PathBuf {
    bevy::asset::io::file::FileAssetReader::get_base_path().join(crate::asset_dir())
}

/// A rig root at `position` (hitbox centre) carrying `root`, with one child per body part.
fn spawn_rig(commands: &mut Commands, rig: &RigImages, position: Vec2, root: impl Bundle) {
    commands
        .spawn((
            root,
            RigPose::default(),
            Transform::from_translation((position + Vec2::Y * SPRITE_OFFSET_Y).extend(Z_PLAYER)),
            Visibility::default(),
        ))
        .with_children(|root| {
            root.spawn((
                Nameplate,
                Transform::from_xyz(0.0, CANVAS / 2.0 + NAME_GAP, NAME_Z),
                Visibility::default(),
            ))
            .with_child((
                NameText,
                Text2d::new(""),
                TextFont::from_font_size(NAME_FONT_SIZE)
                    .with_font(rig.name_font.clone())
                    .with_font_smoothing(bevy::text::FontSmoothing::None),
                TextColor(name_color(None)),
                NAME_SHADOW,
            ));
            for (i, part) in Part::ALL.into_iter().enumerate() {
                root.spawn((
                    RigPart(part),
                    Sprite::from_image(rig.skinned[i].clone()),
                    Anchor(part.anchor().into()),
                    Transform::from_translation(pivot_offset(part).extend(i as f32 * PART_Z)),
                ));
            }
            // Just under the front arm, over everything of the part below it.
            let z = arm_front_index() as f32 * PART_Z - ITEM_Z;
            root.spawn((
                ArmFill,
                Sprite::from_color(Color::WHITE, Vec2::ZERO),
                Anchor::TOP_CENTER,
                Transform::from_translation(pivot_offset(Part::ArmFront).extend(z)),
                Visibility::Hidden,
            ));
        });
}

/// One rig per other player in the world; gone when they leave.
fn sync_remotes(
    mut commands: Commands,
    game: Res<Game>,
    rig: Res<RigImages>,
    remotes: Query<(Entity, &RemotePlayer)>,
) {
    let mut drawn = Vec::new();
    for (entity, remote) in &remotes {
        if game.0.players.contains_key(&remote.id) {
            drawn.push(remote.id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for player in game.0.players.values() {
        if player.id == game.1 || drawn.contains(&player.id) {
            continue;
        }
        let position = Vec2::from(player.body.pos);
        let remote = RemotePlayer {
            id: player.id,
            position,
            facing_west: false,
            reach: None,
        };
        spawn_rig(&mut commands, &rig, position, (remote, Look::default()));
    }
}

/// Holding `HOME_KEY` for `HOME_HOLD` seconds asks to respawn; releasing early cancels. The
/// cooldown is the server's (or `apply`'s offline) to enforce.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource/stream
fn hold_home(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut hold: ResMut<HomeHold>,
    mut sent: Local<bool>,
    mut actions: MessageWriter<crate::state::ActionMsg>,
    text_focus: Option<Res<crate::hud::TextFocus>>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut last_home: Local<Option<f32>>,
    mut cooldown: ResMut<HomeCooldown>,
) {
    // The cooldown is counted here too, from our own last respawn, so a hold during it never
    // sends an action the server would refuse.
    let now = time.elapsed_secs();
    for GameEvent(event) in events.read() {
        if matches!(event, game_shared::rules::Event::Respawned { player, .. } if *player == game.1)
        {
            *last_home = Some(now);
        }
    }
    let left = last_home.map_or(0.0, |at| {
        (game_shared::rules::RESPAWN_COOLDOWN as f32 - (now - at)).max(0.0)
    });
    if cooldown.0 != left {
        cooldown.0 = left;
    }
    let cooling = left > 0.0;
    // Typing an R into a HUD field is not "go home". (An open chat line clears the keyboard, so
    // R never reads as pressed then.)
    let typing = text_focus.is_some_and(|focus| focus.0);
    if typing || cooling || !keys.pressed(HOME_KEY) {
        *sent = false;
        if hold.0 != 0.0 {
            hold.0 = 0.0;
        }
        return;
    }
    if *sent {
        return;
    }
    hold.0 = (hold.0 + time.delta_secs() / HOME_HOLD).min(1.0);
    if hold.0 >= 1.0 {
        actions.write(crate::state::ActionMsg(game_shared::rules::Action::Respawn));
        *sent = true;
        hold.0 = 0.0;
    }
}

fn latch_jump(keys: Res<ButtonInput<KeyCode>>, mut jump: ResMut<JumpPressed>) {
    jump.space |= keys.just_pressed(JUMP_KEY);
    jump.up_key |= keys.any_just_pressed(UP_KEYS);
}

/// One simulation tick: step the body, then pick up whatever it now touches. Online the step is a
/// prediction: the input also goes to the server, which steps it for real and decides pickups.
fn tick_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    net: Option<ResMut<Net>>,
    mut jump: ResMut<JumpPressed>,
    mut player: Single<&mut Player>,
    mut events: MessageWriter<GameEvent>,
) {
    let id = game.1;
    let state = &mut game.0;
    let Some(me) = state.players.get_mut(&id) else {
        return;
    };
    let pressed = std::mem::take(&mut *jump);
    // On a ladder W / ↑ climb and only Space jumps; elsewhere W / ↑ jump as before.
    let ladder = me.body.climbing || on_ladder(&state.world, me.body.pos);
    let up = keys.any_pressed(UP_KEYS);
    let input = Input {
        move_x: i8::from(keys.any_pressed(RIGHT_KEYS)) - i8::from(keys.any_pressed(LEFT_KEYS)),
        jump_held: keys.pressed(JUMP_KEY) || (up && !ladder),
        jump_pressed: pressed.space || (pressed.up_key && !ladder),
        down: keys.any_pressed(DOWN_KEYS),
        up,
    };
    if input.move_x != 0 {
        player.facing_west = input.move_x < 0;
    }
    player.previous = me.body;
    // The same worn mods the server steps with, so the prediction matches.
    let mods = game_shared::mods::worn_mods(&me.equipped);
    let burned = step_with(&mut me.body, input, &state.world, mods);
    if burned.is_some() {
        // Back at the door: no streak across the screen.
        player.previous = me.body;
    }
    if let Some(mut net) = net {
        // Online the server decides and announces the respawn; this is only the prediction.
        net.input(input);
        return;
    }
    if let Some(from) = burned {
        me.last_respawn = time.elapsed_secs_f64();
        events.write(GameEvent(game_shared::rules::Event::Respawned {
            player: id,
            cause: game_shared::rules::RespawnCause::Hazard,
            from,
        }));
    }
    for event in collect_drops(state, id) {
        events.write(GameEvent(event));
    }
}

fn arm_front_index() -> usize {
    Part::ALL
        .iter()
        .position(|&p| p == Part::ArmFront)
        .unwrap_or(0)
}

/// The shoulder in world px for a rig drawn at `position` (hitbox centre).
fn shoulder(position: Vec2, facing_west: bool) -> [f32; 2] {
    let side = if facing_west { -1.0 } else { 1.0 };
    let pivot = pivot_offset(Part::ArmFront);
    let root = position + Vec2::Y * SPRITE_OFFSET_Y;
    [root.x + side * pivot.x, root.y + pivot.y]
}

/// The tile a punch or place swings at, and whether it is a place.
fn swing_target(action: Action) -> Option<(u32, u32, bool)> {
    match action {
        Action::Punch { x, y } => Some((x, y, false)),
        Action::Place { x, y, .. } => Some((x, y, true)),
        _ => None,
    }
}

/// Starts a reach-out: turn towards the tile first, then the arm goes out from wherever it is.
fn reach_towards(
    reach: &mut Option<rig::Reach>,
    facing_west: &mut bool,
    position: Vec2,
    (x, y, place): (u32, u32, bool),
    now: f32,
) {
    let target = tile_to_world(x, y);
    if target.0 != position.x {
        *facing_west = target.0 < position.x;
    }
    let from = reach.and_then(|r| r.length_at(now)).unwrap_or(rig::ARM_LEN);
    *reach = Some(rig::Reach::new(
        now,
        from,
        shoulder(position, *facing_west),
        [target.0, target.1],
        (x, y),
        place,
        *facing_west,
    ));
}

/// Every punch or place reaches out: the local player's on input (prediction, refused or
/// not), other players' from the server's `Swing`.
fn start_reach(
    time: Res<Time>,
    net: Option<ResMut<Net>>,
    mut actions: MessageReader<ActionMsg>,
    mut player: Single<&mut Player>,
    mut remotes: Query<&mut RemotePlayer>,
) {
    let now = time.elapsed_secs();
    let player: &mut Player = &mut player;
    for &ActionMsg(action) in actions.read() {
        if let Some(target) = swing_target(action) {
            let position = player.position;
            reach_towards(
                &mut player.reach,
                &mut player.facing_west,
                position,
                target,
                now,
            );
        }
    }
    let Some(mut net) = net else { return };
    for (id, x, y, place) in net.take_swings() {
        if let Some(mut remote) = remotes.iter_mut().find(|remote| remote.id == id) {
            let remote = &mut *remote;
            let position = remote.position;
            reach_towards(
                &mut remote.reach,
                &mut remote.facing_west,
                position,
                (x, y, place),
                now,
            );
        }
    }
}

/// The reach at `now`: (px the arm is pushed out, its angle). Sends `ArmContact` once at full
/// extension and clears the reach when the arm is back.
fn reach_now(
    reach: &mut Option<rig::Reach>,
    now: f32,
    player: PlayerId,
    contacts: &mut MessageWriter<ArmContact>,
) -> (f32, Option<f32>) {
    let Some(current) = reach.as_mut() else {
        return (0.0, None);
    };
    if !current.contacted && now >= current.contact() {
        current.contacted = true;
        let (x, y) = current.tile;
        let place = current.place;
        contacts.write(ArmContact {
            player,
            x,
            y,
            place,
        });
    }
    match current.length_at(now) {
        Some(length) => ((length - rig::ARM_LEN).max(0.0), Some(current.angle)),
        None => {
            *reach = None;
            (0.0, None)
        }
    }
}

/// Copies what each player wears into their rig's `Look`; only a real change re-dresses it.
fn follow_equipment(
    game: Res<Game>,
    mut local: Query<&mut Look, (With<Player>, Without<RemotePlayer>)>,
    mut remotes: Query<(&RemotePlayer, &mut Look), Without<Player>>,
) {
    let follow = |look: &mut Mut<Look>, id: PlayerId| {
        let Some(player) = game.0.players.get(&id) else {
            return;
        };
        let slots = worn_folders(player);
        if look.slots != slots {
            look.slots = slots;
        }
    };
    for mut look in &mut local {
        follow(&mut look, game.1);
    }
    for (remote, mut look) in &mut remotes {
        follow(&mut look, remote.id);
    }
}

/// Re-spawns the item layers under each body part when the `Look` changes (and once at spawn).
fn dress_rig(
    mut commands: Commands,
    assets: Res<AssetServer>,
    looks: Query<(&Look, &Children), Changed<Look>>,
    parts: Query<&RigPart>,
) {
    for (look, children) in &looks {
        for &entity in children {
            let Ok(&RigPart(part)) = parts.get(entity) else {
                continue;
            };
            dress_part(&mut commands, &assets, look, entity, part);
        }
    }
}

fn dress_part(
    commands: &mut Commands,
    assets: &AssetServer,
    look: &Look,
    entity: Entity,
    part: Part,
) {
    commands.entity(entity).despawn_children();
    let root = asset_root();
    for item in look.slots.iter().flatten() {
        let Some(layer) = rig::layer(item) else {
            continue;
        };
        if !rig::item_parts(&root, item).contains(&part) {
            continue;
        }
        commands.entity(entity).with_child((
            Sprite::from_image(assets.load(format!("items/{item}/{}.png", part.name()))),
            Anchor(part.anchor().into()),
            Transform::from_xyz(0.0, 0.0, (layer + 1) as f32 * ITEM_Z),
        ));
    }
    match (
        part,
        look.slots[Slot::Back as usize],
        look.slots[Slot::Hand as usize],
    ) {
        // Behind every part: the torso sits at 3 * PART_Z, so this ends up below arm_back (0).
        (Part::Torso, Some(wings), _) => {
            commands.entity(entity).with_child((
                Wings,
                Sprite::from_image(assets.load(format!("items/{wings}/{}", rig::WINGS_FILE))),
                Anchor(part.anchor().into()),
                Transform::from_xyz(0.0, 0.0, -WINGS_Z),
            ));
        }
        // In the front hand, over the arm and its sleeves, so it swings with the punch.
        (Part::ArmFront, _, Some(held)) => {
            let [x, y] = rig::hand_offset();
            commands.entity(entity).with_child((
                Sprite::from_image(assets.load(format!("items/{held}/{}", rig::HELD_FILE))),
                Anchor(rig::held_anchor().into()),
                Transform::from_xyz(x, y, HELD_Z)
                    .with_rotation(Quat::from_rotation_z(rig::HELD_ANGLE)),
            ));
        }
        _ => {}
    }
}

/// Fills the skinned body images once the sources are loaded, and again whenever the skin tone changes.
/// ponytail: one set of skinned images, in the local player's tone, shared by every rig.
fn skin_rig(
    mut images: ResMut<Assets<Image>>,
    mut rig: ResMut<RigImages>,
    look: Single<&Look, With<Player>>,
) {
    if rig.skin == Some(look.skin) {
        return;
    }
    let mut skinned = Vec::with_capacity(6);
    for source in &rig.source {
        let Some(image) = images.get(source) else {
            return; // still loading
        };
        let mut image = match image.texture_descriptor.format {
            TextureFormat::Rgba8UnormSrgb => image.clone(),
            _ => image
                .convert(TextureFormat::Rgba8UnormSrgb)
                .expect("rig layers are RGBA PNGs"),
        };
        if let Some(data) = image.data.as_mut() {
            rig::recolor_skin(data, look.skin);
        }
        skinned.push(image);
    }
    for (handle, image) in rig.skinned.iter().zip(skinned) {
        images
            .insert(handle, image)
            .expect("reserved handle is valid");
    }
    rig.skin = Some(look.skin);
}

fn motion(body: &Body) -> Motion {
    if body.climbing {
        Motion::Climb {
            moving: body.vel != (0.0, 0.0),
        }
    } else if !body.on_ground {
        Motion::Air
    } else if body.vel.0.abs() < IDLE_SPEED {
        Motion::Idle
    } else {
        Motion::Walk
    }
}

fn draw_player(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    game: Res<Game>,
    player: Single<(&mut Player, &mut Transform, &mut RigPose)>,
    mut contacts: MessageWriter<ArmContact>,
) {
    let Some(me) = game.me() else { return };
    let (mut player, mut transform, mut pose) = player.into_inner();
    let (from, to) = (Vec2::from(player.previous.pos), Vec2::from(me.body.pos));
    player.position = from.lerp(to, fixed.overstep_fraction());

    let now = time.elapsed_secs();
    let (pushed, angle) = reach_now(&mut player.reach, now, game.1, &mut contacts);

    // The layers face east; mirroring the root mirrors every part and its rotation.
    let side = if player.facing_west { -1.0 } else { 1.0 };
    transform.translation = (player.position + Vec2::Y * SPRITE_OFFSET_Y).extend(Z_PLAYER);
    transform.scale.x = side;

    let motion = motion(&me.body);
    let mut parts = rig::pose(motion, now, None);
    if let Some(angle) = angle {
        parts[arm_front_index()].0 = angle;
    }
    *pose = RigPose(parts, rig::flap(motion, now), pushed);
}

fn draw_remotes(
    time: Res<Time>,
    game: Res<Game>,
    mut remotes: Query<(&mut RemotePlayer, &mut Transform, &mut RigPose)>,
    mut contacts: MessageWriter<ArmContact>,
) {
    let blend = 1.0 - (-REMOTE_STIFFNESS * time.delta_secs()).exp();
    for (mut remote, mut transform, mut pose) in &mut remotes {
        let Some(player) = game.0.players.get(&remote.id) else {
            continue;
        };
        let body = player.body;
        let target = Vec2::from(body.pos);
        let position = remote.position;
        remote.position = position + (target - position) * blend;
        // While reaching out they keep facing the tile.
        if body.vel.0.abs() >= IDLE_SPEED && remote.reach.is_none() {
            remote.facing_west = body.vel.0 < 0.0;
        }
        let side = if remote.facing_west { -1.0 } else { 1.0 };
        transform.translation = (remote.position + Vec2::Y * SPRITE_OFFSET_Y).extend(Z_PLAYER);
        transform.scale.x = side;
        let (motion, now) = (motion(&body), time.elapsed_secs());
        let id = remote.id;
        let (pushed, angle) = reach_now(&mut remote.reach, now, id, &mut contacts);
        let mut parts = rig::pose(motion, now, None);
        if let Some(angle) = angle {
            parts[arm_front_index()].0 = angle;
        }
        *pose = RigPose(parts, rig::flap(motion, now), pushed);
    }
}

/// The local player's name offline: `GAME_NAME` if valid, else `OFFLINE_NAME`.
pub fn offline_name() -> String {
    std::env::var("GAME_NAME")
        .ok()
        .filter(|name| game_protocol::valid_name(name))
        .unwrap_or_else(|| OFFLINE_NAME.into())
}

/// The name over each rig: the server's name for that player online, `GAME_NAME` offline.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn name_rigs(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    net: Option<Res<Net>>,
    local: Query<Entity, With<Player>>,
    remotes: Query<&RemotePlayer>,
    plates: Query<(Entity, &ChildOf, &Children), With<Nameplate>>,
    mut texts: Query<&mut Text2d, With<NameText>>,
    flags: Query<&Flag>,
    mut no_flag: Local<std::collections::HashSet<String>>,
) {
    let country_of = |id: PlayerId| match &net {
        Some(net) => net.country(id).map(str::to_owned),
        None => crate::net::local_country(),
    };
    let name_of = |id: PlayerId| match &net {
        Some(net) => net
            .name(id)
            .map_or_else(|| format!("Player{}", id.0), str::to_owned),
        None => offline_name(),
    };
    for (plate, root, children) in &plates {
        let id = match (local.get(root.parent()), remotes.get(root.parent())) {
            (Ok(_), _) => game.1,
            (_, Ok(remote)) => remote.id,
            _ => continue,
        };
        let name = name_of(id);
        let country = country_of(id);
        let mut shown = None;
        for &child in children {
            if let Ok(mut text) = texts.get_mut(child)
                && text.0 != name
            {
                text.0 = name.clone();
            }
            if let Ok(Flag(code)) = flags.get(child) {
                if country.as_ref() == Some(code) {
                    shown = Some(code);
                } else {
                    commands.entity(child).despawn();
                }
            }
        }
        // No flag for an unknown code or one without a file; a missing file is looked up once.
        let mut has_file = |code: &String| {
            if no_flag.contains(code) {
                return false;
            }
            let found = asset_root().join(format!("flags/{code}.png")).is_file();
            if !found {
                no_flag.insert(code.clone());
            }
            found
        };
        if let (Some(code), None) = (country.as_ref().filter(|c| has_file(c)), shown) {
            commands.entity(plate).with_child((
                Flag(code.clone()),
                Sprite::from_image(assets.load(format!("flags/{code}.png"))),
                Transform::from_xyz(0.0, 0.0, ITEM_Z),
            ));
        }
    }
}

/// Keeps each flag just left of its name, whatever the name's width.
fn place_flags(
    plates: Query<&Children, With<Nameplate>>,
    texts: Query<&TextLayoutInfo, With<NameText>>,
    mut flags: Query<&mut Transform, With<Flag>>,
) {
    for children in &plates {
        let width = children
            .iter()
            .find_map(|child| texts.get(child).ok())
            .map_or(0.0, |layout| layout.size.x / layout.scale_factor.max(1.0));
        let x = -(width / 2.0 + FLAG_GAP + FLAG_WIDTH / 2.0).round();
        for child in children.iter() {
            if let Ok(mut transform) = flags.get_mut(child)
                && transform.translation.x != x
            {
                transform.translation.x = x;
            }
        }
    }
}

/// The root's scale.x is -1 when facing west; cancel it so names read left to right.
/// Keeps names (and flags) `NAME_FONT_SIZE` px tall on screen at every zoom, so the pixel font stays
/// on its native grid, sitting just above the rig. Also cancels the root's mirroring (scale.x = -1
/// when facing west) so names read left to right.
fn place_names(
    zoom: Res<crate::camera::ViewZoom>,
    mut names: Query<(&ChildOf, &mut Transform), With<Nameplate>>,
    roots: Query<&Transform, Without<Nameplate>>,
) {
    let inverse = 1.0 / zoom.0.max(1.0);
    for (root, mut transform) in &mut names {
        let Ok(root) = roots.get(root.parent()) else {
            continue;
        };
        let scale = Vec3::new(root.scale.x * inverse, inverse, 1.0);
        // The text is centred on its origin: lift it by half its on-screen height.
        let y = CANVAS / 2.0 + NAME_GAP + NAME_FONT_SIZE / 2.0 * inverse;
        if transform.scale != scale || transform.translation.y != y {
            transform.scale = scale;
            transform.translation.y = y;
        }
    }
}

/// Moves every body part, and the wings on the torso, to its rig's pose.
fn pose_parts(
    poses: Query<&RigPose>,
    rig: Res<RigImages>,
    mut parts: Query<(&RigPart, &ChildOf, &mut Transform), Without<Wings>>,
    mut wings: Query<(&ChildOf, &mut Transform), (With<Wings>, Without<RigPart>)>,
    mut fills: Query<
        (&ChildOf, &mut Transform, &mut Sprite, &mut Visibility),
        (With<ArmFill>, Without<RigPart>, Without<Wings>),
    >,
) {
    for (&RigPart(part), child_of, mut transform) in &mut parts {
        let Ok(pose) = poses.get(child_of.parent()) else {
            continue;
        };
        let i = Part::ALL.iter().position(|&p| p == part).unwrap_or(0);
        let (angle, dy) = pose.0[i];
        let mut at = pivot_offset(part) + Vec2::Y * dy;
        if part == Part::ArmFront {
            // Reaching out: the whole arm slides along itself, so the fist and the held item stay
            // crisp; `ArmFill` covers the gap from the shoulder.
            at += Vec2::new(angle.sin(), -angle.cos()) * pose.2;
        }
        transform.translation = at.extend(transform.translation.z);
        transform.rotation = Quat::from_rotation_z(angle);
    }
    let [r, g, b] = rig::SKIN_RAMPS[rig.skin.unwrap_or(0)].1[1].to_be_bytes()[1..] else {
        return;
    };
    for (root, mut transform, mut sprite, mut visibility) in &mut fills {
        let Ok(pose) = poses.get(root.parent()) else {
            continue;
        };
        let shown = pose.2 > 0.5;
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if shown {
            transform.rotation = Quat::from_rotation_z(pose.0[arm_front_index()].0);
            sprite.custom_size = Some(Vec2::new(ARM_FILL_WIDTH, pose.2 + 2.0));
            sprite.color = Color::srgb_u8(r, g, b);
        }
    }
    for (torso, mut transform) in &mut wings {
        let flap = parts
            .get(torso.parent())
            .ok()
            .and_then(|(_, root, _)| poses.get(root.parent()).ok())
            .map_or(0.0, |pose| pose.1);
        transform.rotation = Quat::from_rotation_z(flap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::physics::spawn_body;
    use game_shared::worldgen::generate;

    #[test]
    fn feet_line_sits_on_the_hitbox_bottom() {
        // The canvas bottom is (CANVAS - FEET_Y) px below the feet line.
        let canvas_bottom = SPRITE_OFFSET_Y - CANVAS / 2.0;
        assert_eq!(canvas_bottom + (CANVAS - FEET_Y), -HITBOX_HEIGHT / 2.0);
    }

    #[test]
    fn motion_states() {
        let mut body = spawn_body(&generate("START"));
        body.on_ground = true;
        body.vel.0 = 0.0;
        assert_eq!(motion(&body), Motion::Idle);
        body.vel.0 = -100.0;
        assert_eq!(motion(&body), Motion::Walk);
        body.on_ground = false;
        assert_eq!(motion(&body), Motion::Air);
        body.climbing = true;
        assert_eq!(motion(&body), Motion::Climb { moving: true });
        body.vel = (0.0, 0.0);
        assert_eq!(motion(&body), Motion::Climb { moving: false });
    }
}
