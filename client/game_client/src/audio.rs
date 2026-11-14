//! Music and sound effects (assets/README.md, "Audio"). Global, non-positional playback through
//! Bevy's audio; voice chat keeps its own output stream (`voice/`). Sounds follow `GameEvent`,
//! `ActionResult`, chat, level-ups and the local player's body; HUD code asks for UI sounds by
//! writing an `Sfx` message. A screenshot run plays nothing.

use bevy::audio::Volume;
use bevy::prelude::*;
use game_shared::items::{
    BEDROCK, COBBLESTONE, GLASS, ICE, ItemId, ItemKind, MOSSY_STONE, STONE, STONE_BRICKS, item,
};
use game_shared::physics::{Body, WALK_SPEED};
use game_shared::rules::{Action, Event, Layer, PlayerId, Reject};
use game_shared::world::{Tile, World};
use game_shared::{WORLD_HEIGHT, WORLD_WIDTH};

use crate::chat::ChatReceived;
use crate::hud::OpenPanel;
use crate::levels_ui::LevelUpSound;
use crate::screenshot;
use crate::settings::{MAX_VOLUME, Settings};
use crate::state::{ActionResult, Game, GameEvent, Phase};

const WORLD_BGM: &str = "audio/bgm/world.ogg";
/// Seconds between two footsteps while walking on the ground.
const FOOTSTEP_SECS: f32 = 0.35;
/// Below this horizontal speed (world px/s) the player is standing, not walking.
const WALKING_SPEED: f32 = WALK_SPEED * 0.25;
/// The same sound never starts twice within this many seconds (a held punch, a burst of drops).
const SFX_MIN_GAP: f32 = 0.08;
/// Struck with `punch_stone`; everything else sounds like `punch_dirt`. Claim blocks count too.
const HARD_BLOCKS: [ItemId; 7] = [
    STONE,
    BEDROCK,
    COBBLESTONE,
    STONE_BRICKS,
    MOSSY_STONE,
    ICE,
    GLASS,
];

/// A sound effect. Written by the HUD for clicks; everything else is derived here.
#[derive(Message, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sfx {
    PunchDirt,
    PunchStone,
    BlockBreak,
    BlockPlace,
    ItemPickup,
    Gem,
    Jump,
    Land,
    Footstep,
    Harvest,
    Plant,
    Door,
    Equip,
    Purchase,
    Denied,
    Chat,
    LevelUp,
    UiClick,
    PanelOpen,
}

const SFX_COUNT: usize = Sfx::PanelOpen as usize + 1;

impl Sfx {
    fn file(self) -> &'static str {
        match self {
            Sfx::PunchDirt => "audio/sfx/punch_dirt.ogg",
            Sfx::PunchStone => "audio/sfx/punch_stone.ogg",
            Sfx::BlockBreak => "audio/sfx/block_break.ogg",
            Sfx::BlockPlace => "audio/sfx/block_place.ogg",
            Sfx::ItemPickup => "audio/sfx/item_pickup.ogg",
            Sfx::Gem => "audio/sfx/gem.ogg",
            Sfx::Jump => "audio/sfx/jump.ogg",
            Sfx::Land => "audio/sfx/land.ogg",
            Sfx::Footstep => "audio/sfx/footstep_grass.ogg",
            Sfx::Harvest => "audio/sfx/harvest.ogg",
            Sfx::Plant => "audio/sfx/plant.ogg",
            Sfx::Door => "audio/sfx/door.ogg",
            Sfx::Equip => "audio/sfx/equip.ogg",
            Sfx::Purchase => "audio/sfx/purchase.ogg",
            Sfx::Denied => "audio/sfx/denied.ogg",
            Sfx::Chat => "audio/sfx/chat.ogg",
            Sfx::LevelUp => "audio/sfx/level_up.ogg",
            Sfx::UiClick => "audio/sfx/ui_click.ogg",
            Sfx::PanelOpen => "audio/sfx/panel_open.ogg",
        }
    }
}

/// The background music entity.
#[derive(Component)]
pub struct Music;

/// A looping music player at `level` (0..=`MAX_VOLUME`). The lobby app can spawn one with the
/// saved level: `Settings::load_from(&dir).0.music`.
pub fn music_player(assets: &AssetServer, track: &'static str, level: u8) -> impl Bundle {
    (
        AudioPlayer::new(assets.load(track)),
        PlaybackSettings::LOOP.with_volume(volume(level)),
        Music,
    )
}

/// Settings volume (0..=`MAX_VOLUME`) to playback volume; squared so the slider feels even.
fn volume(level: u8) -> Volume {
    let fraction = f32::from(level.min(MAX_VOLUME)) / f32::from(MAX_VOLUME);
    Volume::Linear(fraction * fraction)
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Sfx>();
        if screenshot::screenshot_path().is_some() {
            return;
        }
        app.add_systems(Startup, start_music).add_systems(
            Update,
            (listen, play_sfx, music_volume).chain().in_set(Phase::View),
        );
    }
}

fn start_music(mut commands: Commands, assets: Res<AssetServer>, settings: Res<Settings>) {
    commands.spawn(music_player(&assets, WORLD_BGM, settings.music));
}

fn music_volume(settings: Res<Settings>, mut sinks: Query<&mut AudioSink, With<Music>>) {
    // One query: a separate `Added<AudioSink>` query conflicts with `&mut AudioSink` (Bevy B0001).
    for mut sink in &mut sinks {
        if settings.is_changed() || sink.is_added() {
            sink.set_volume(volume(settings.music));
        }
    }
}

/// What `listen` remembers between frames.
#[derive(Default)]
struct Ears {
    entered: bool,
    gems: Option<u32>,
    steps: Steps,
    /// The world's tiles as last seen, to tell a break from a placement.
    tiles: Option<(String, Vec<Tile>)>,
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per thing that makes a sound
fn listen(
    time: Res<Time>,
    game: Res<Game>,
    open: Res<OpenPanel>,
    mut events: MessageReader<GameEvent>,
    mut results: MessageReader<ActionResult>,
    mut chats: MessageReader<ChatReceived>,
    mut level_ups: MessageReader<LevelUpSound>,
    mut ears: Local<Ears>,
    mut sfx: MessageWriter<Sfx>,
) {
    let me = game.1;
    if !ears.entered {
        ears.entered = true;
        sfx.write(Sfx::Door);
    }
    let world = &game.0.world;
    let mut tiles = match ears.tiles.take() {
        Some((name, tiles)) if name == world.name => tiles,
        _ => snapshot(world),
    };
    let mut gems = ears.gems.or(game.me().map(|p| p.gems)).unwrap_or(0);
    for GameEvent(event) in events.read() {
        let sound = match *event {
            Event::TileChanged { x, y, tile } => World::tile_key(x, y)
                .and_then(|key| tiles.get_mut(key as usize))
                .and_then(|seen| tile_sfx(std::mem::replace(seen, tile), tile)),
            _ => event_sfx(event, me, &mut gems),
        };
        sfx.write_batch(sound);
    }
    ears.gems = Some(gems);
    ears.tiles = Some((world.name.clone(), tiles));
    for ActionResult { action, result } in results.read() {
        sfx.write_batch(result_sfx(action, result, world));
    }
    sfx.write_batch(chats.read().map(|_| Sfx::Chat));
    sfx.write_batch(level_ups.read().map(|_| Sfx::LevelUp));
    if open.is_changed() && *open != OpenPanel::None {
        sfx.write(Sfx::PanelOpen);
    }
    if let Some(me) = game.me() {
        sfx.write_batch(body_sfx(&mut ears.steps, &me.body, time.delta_secs()));
    }
}

fn snapshot(world: &World) -> Vec<Tile> {
    let keys = 0..WORLD_WIDTH * WORLD_HEIGHT;
    let tile = |key| world.get(key % WORLD_WIDTH, key / WORLD_WIDTH);
    keys.map(|key| tile(key).unwrap_or_default()).collect()
}

fn play_sfx(
    mut commands: Commands,
    assets: Res<AssetServer>,
    time: Res<Time>,
    settings: Res<Settings>,
    mut requests: MessageReader<Sfx>,
    mut gaps: Local<Gaps>,
) {
    let now = time.elapsed_secs();
    for &sfx in requests.read() {
        if !gaps.allow(sfx, now) {
            continue;
        }
        debug!("sfx {sfx:?}");
        if settings.sfx > 0 {
            commands.spawn((
                AudioPlayer::new(assets.load(sfx.file())),
                PlaybackSettings::DESPAWN.with_volume(volume(settings.sfx)),
            ));
        }
    }
}

/// When each sound last started.
struct Gaps([f32; SFX_COUNT]);

impl Default for Gaps {
    fn default() -> Self {
        Self([f32::NEG_INFINITY; SFX_COUNT])
    }
}

impl Gaps {
    fn allow(&mut self, sfx: Sfx, now: f32) -> bool {
        let last = &mut self.0[sfx as usize];
        let free = now - *last >= SFX_MIN_GAP;
        if free {
            *last = now;
        }
        free
    }
}

/// The sound of an event, other than `TileChanged`. `gems` is the local player's last known total.
fn event_sfx(event: &Event, me: PlayerId, gems: &mut u32) -> Option<Sfx> {
    match *event {
        Event::DropTaken { by, .. } if by == me => Some(Sfx::ItemPickup),
        Event::GemsChanged { player, gems: now } if player == me => {
            let more = now > *gems;
            *gems = now;
            more.then_some(Sfx::Gem)
        }
        Event::TreeHarvested { .. } => Some(Sfx::Harvest),
        Event::TreePlanted { .. } => Some(Sfx::Plant),
        Event::LeftWorld { player } if player == me => Some(Sfx::Door),
        Event::Equipped { player, .. } if player == me => Some(Sfx::Equip),
        // The trade window opens.
        Event::TradeRequested { from, to, .. } if from == me || to == me => Some(Sfx::PanelOpen),
        _ => None,
    }
}

/// A tile lost a layer (break) or gained one (place).
fn tile_sfx(before: Tile, after: Tile) -> Option<Sfx> {
    let layers = |tile: Tile| u8::from(tile.fg.0 != 0) + u8::from(tile.bg.0 != 0);
    match layers(after).cmp(&layers(before)) {
        std::cmp::Ordering::Less => Some(Sfx::BlockBreak),
        std::cmp::Ordering::Greater => Some(Sfx::BlockPlace),
        std::cmp::Ordering::Equal => None,
    }
}

/// The answer to one of the local player's actions: a punch that landed, a purchase, a refusal.
/// A punch sent too early (`Cooldown`) is silent: holding the button sends them all the time.
fn result_sfx(action: &Action, result: &Result<Vec<Event>, Reject>, world: &World) -> Option<Sfx> {
    let events = match result {
        Err(Reject::Cooldown) => return None,
        Err(_) => return Some(Sfx::Denied),
        Ok(events) => events,
    };
    match action {
        Action::Buy { .. } | Action::BuyBackpackUpgrade => Some(Sfx::Purchase),
        Action::Punch { .. } => events.iter().find_map(|event| match *event {
            Event::TileDamaged { x, y, layer, .. } => {
                let tile = world.get(x, y).unwrap_or_default();
                Some(punch_sfx(match layer {
                    Layer::Foreground => tile.fg,
                    Layer::Background => tile.bg,
                }))
            }
            _ => None,
        }),
        _ => None,
    }
}

fn punch_sfx(id: ItemId) -> Sfx {
    let claim = item(id).is_some_and(|def| def.kind == ItemKind::Claim);
    if claim || HARD_BLOCKS.contains(&id) {
        Sfx::PunchStone
    } else {
        Sfx::PunchDirt
    }
}

/// Footstep timing and the last ground state, for `body_sfx`.
#[derive(Default)]
struct Steps {
    on_ground: Option<bool>,
    walked: f32,
}

/// Jump (left the ground going up), land (back on it), or a footstep every `FOOTSTEP_SECS` of
/// walking on the ground.
fn body_sfx(steps: &mut Steps, body: &Body, dt: f32) -> Option<Sfx> {
    let was = steps.on_ground.replace(body.on_ground);
    let edge = match (was, body.on_ground) {
        (Some(true), false) if body.vel.1 > 0.0 => Some(Sfx::Jump),
        (Some(false), true) => Some(Sfx::Land),
        _ => None,
    };
    if !body.on_ground || body.vel.0.abs() < WALKING_SPEED {
        steps.walked = 0.0;
        return edge;
    }
    steps.walked += dt;
    if edge.is_none() && steps.walked >= FOOTSTEP_SECS {
        steps.walked -= FOOTSTEP_SECS;
        return Some(Sfx::Footstep);
    }
    edge
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::{DIRT, EMPTY, SAND, SMALL_CLAIM, seed_of};
    use game_shared::physics::spawn_body;
    use game_shared::worldgen::generate;

    const ME: PlayerId = PlayerId(1);
    const OTHER: PlayerId = PlayerId(2);

    #[test]
    fn punches_sound_like_the_material_struck() {
        for id in [DIRT, SAND, seed_of(DIRT).expect("dirt seed"), EMPTY] {
            assert_eq!(punch_sfx(id), Sfx::PunchDirt, "{id:?}");
        }
        for id in [STONE, COBBLESTONE, GLASS, SMALL_CLAIM] {
            assert_eq!(punch_sfx(id), Sfx::PunchStone, "{id:?}");
        }
    }

    #[test]
    fn results_map_to_punch_purchase_and_denied_but_cooldown_is_silent() {
        let mut world = generate("TEST");
        let (x, y) = (3, 20);
        world.set(
            x,
            y,
            Tile {
                fg: STONE,
                bg: EMPTY,
            },
        );
        let punch = Action::Punch { x, y };
        let damaged = |layer| Event::TileDamaged {
            x,
            y,
            layer,
            damage: 1,
        };
        let ok = Ok(vec![damaged(Layer::Foreground)]);
        assert_eq!(result_sfx(&punch, &ok, &world), Some(Sfx::PunchStone));
        world.set(
            x,
            y,
            Tile {
                fg: EMPTY,
                bg: DIRT,
            },
        );
        let ok = Ok(vec![damaged(Layer::Background)]);
        assert_eq!(result_sfx(&punch, &ok, &world), Some(Sfx::PunchDirt));
        assert_eq!(result_sfx(&punch, &Err(Reject::Cooldown), &world), None);
        assert_eq!(
            result_sfx(&punch, &Err(Reject::NoPermission), &world),
            Some(Sfx::Denied)
        );
        let buy = Action::Buy { item: SMALL_CLAIM };
        assert_eq!(result_sfx(&buy, &Ok(vec![]), &world), Some(Sfx::Purchase));
        assert_eq!(
            result_sfx(&buy, &Err(Reject::NotEnoughGems), &world),
            Some(Sfx::Denied)
        );
        let drop = Action::Drop {
            item: DIRT,
            count: 1,
            left: true,
        };
        assert_eq!(result_sfx(&drop, &Ok(vec![]), &world), None);
    }

    #[test]
    fn events_map_to_sounds_for_the_local_player() {
        let mut gems = 10;
        let taken = |by| Event::DropTaken {
            drop_id: 1,
            by,
            count: 1,
        };
        assert_eq!(event_sfx(&taken(ME), ME, &mut gems), Some(Sfx::ItemPickup));
        assert_eq!(event_sfx(&taken(OTHER), ME, &mut gems), None);
        let gems_now = |gems| Event::GemsChanged { player: ME, gems };
        assert_eq!(event_sfx(&gems_now(15), ME, &mut gems), Some(Sfx::Gem));
        assert_eq!(event_sfx(&gems_now(5), ME, &mut gems), None, "spent");
        assert_eq!(gems, 5);
        let left = |player| Event::LeftWorld { player };
        assert_eq!(event_sfx(&left(ME), ME, &mut gems), Some(Sfx::Door));
        assert_eq!(event_sfx(&left(OTHER), ME, &mut gems), None);
        let harvested = Event::TreeHarvested { x: 1, y: 1 };
        assert_eq!(event_sfx(&harvested, ME, &mut gems), Some(Sfx::Harvest));
        let requested = Event::TradeRequested {
            trade: 1,
            from: OTHER,
            to: ME,
        };
        assert_eq!(event_sfx(&requested, ME, &mut gems), Some(Sfx::PanelOpen));
    }

    #[test]
    fn a_tile_that_loses_a_layer_breaks_and_one_that_gains_one_is_placed() {
        let tile = |fg, bg| Tile { fg, bg };
        let (empty, wall, block) = (tile(EMPTY, EMPTY), tile(EMPTY, DIRT), tile(DIRT, DIRT));
        assert_eq!(tile_sfx(block, wall), Some(Sfx::BlockBreak));
        assert_eq!(tile_sfx(wall, empty), Some(Sfx::BlockBreak));
        assert_eq!(tile_sfx(empty, wall), Some(Sfx::BlockPlace));
        assert_eq!(tile_sfx(wall, block), Some(Sfx::BlockPlace));
        assert_eq!(tile_sfx(block, tile(STONE, DIRT)), None);
    }

    #[test]
    fn the_body_jumps_lands_and_walks() {
        let world = generate("TEST");
        let standing = spawn_body(&world);
        let body = |on_ground, vel| Body {
            on_ground,
            vel,
            ..standing
        };
        let mut steps = Steps::default();
        let dt = 0.125;
        let run = |steps: &mut Steps, b: Body| body_sfx(steps, &b, dt);
        assert_eq!(run(&mut steps, body(true, (0.0, 0.0))), None);
        assert_eq!(run(&mut steps, body(false, (0.0, 300.0))), Some(Sfx::Jump));
        assert_eq!(run(&mut steps, body(false, (0.0, -300.0))), None);
        assert_eq!(run(&mut steps, body(true, (0.0, 0.0))), Some(Sfx::Land));
        // Walking off a ledge is not a jump.
        assert_eq!(run(&mut steps, body(false, (0.0, -10.0))), None);
        assert_eq!(run(&mut steps, body(true, (0.0, 0.0))), Some(Sfx::Land));
        // One footstep per FOOTSTEP_SECS of walking: 0.35 s at 0.125 s frames is frames 2 and 5.
        let walking = body(true, (WALK_SPEED, 0.0));
        let sounds: Vec<_> = (0..8).map(|_| run(&mut steps, walking)).collect();
        let steps_at: Vec<_> = sounds
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == Some(Sfx::Footstep))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(steps_at, vec![2, 5]);
        // Standing still resets the count.
        assert_eq!(run(&mut steps, standing), None);
        assert_eq!(steps.walked, 0.0);
    }

    #[test]
    fn volume_maps_the_slider_to_a_squared_linear_gain() {
        assert_eq!(volume(0).to_linear(), 0.0);
        assert_eq!(volume(MAX_VOLUME).to_linear(), 1.0);
        assert!((volume(50).to_linear() - 0.25).abs() < 1e-6);
        assert_eq!(volume(255).to_linear(), 1.0, "clamped");
    }

    #[test]
    fn the_same_sound_cannot_start_twice_within_the_gap() {
        let mut gaps = Gaps::default();
        assert!(gaps.allow(Sfx::PunchDirt, 1.0));
        assert!(!gaps.allow(Sfx::PunchDirt, 1.0 + SFX_MIN_GAP / 2.0));
        assert!(gaps.allow(Sfx::PunchStone, 1.0), "per sound");
        assert!(gaps.allow(Sfx::PunchDirt, 1.0 + SFX_MIN_GAP));
    }
}
