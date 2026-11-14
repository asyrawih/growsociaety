//! The authoritative state and the single door into it. Systems send an `ActionMsg`; `apply_actions`
//! is the only system that calls `rules::apply`, and it forwards what happened as messages. The
//! client already behaves as if a server were answering. Online (`net.rs`), the same system sends
//! the actions to the server instead and records what the server answers.

use bevy::prelude::*;
use game_shared::commands::{Admin, held_before, report};
use game_shared::rules::{Action, Event, Player, PlayerId, Reject, WorldState, apply, apply_admin};
use game_shared::worldgen::generate;

use crate::net::Net;

const START_WORLD: &str = "START";
/// The offline player's id. Online, the server assigns one: always read `Game::1`.
pub const LOCAL_PLAYER: PlayerId = PlayerId(1);
/// ponytail: fixed seed for the rules' randomness; the server picks it from M7 on.
const RULES_SEED: u64 = 0x5EED;

/// The world and the local player's id in it.
#[derive(Resource)]
pub struct Game(pub WorldState, pub PlayerId);

impl Game {
    pub fn me(&self) -> Option<&Player> {
        self.0.players.get(&self.1)
    }
}

/// A request from the local player. May be rejected.
#[derive(Message, Clone, Copy)]
pub struct ActionMsg(pub Action);

/// A fact about a change that happened. Views update from these only.
#[derive(Message, Clone)]
pub struct GameEvent(pub Event);

/// The answer to one `ActionMsg`, for feedback (reject flash, punch animation).
#[derive(Message)]
pub struct ActionResult {
    pub action: Action,
    pub result: Result<Vec<Event>, Reject>,
}

/// An offline admin command (debug builds), parsed and resolved by `chat.rs`. Online, commands
/// go to the server as chat text and never become an `AdminMsg`.
#[derive(Message, Clone, Copy)]
pub struct AdminMsg(pub Admin);

/// A line for the chat log from the game itself (command results).
#[derive(Message, Clone)]
pub struct SystemLine(pub String);

/// Order of one `Update` frame.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase {
    /// Read devices, send `ActionMsg`.
    Input,
    /// `apply_actions`, nothing else.
    Apply,
    /// React to `GameEvent` and `ActionResult`.
    View,
}

pub struct StatePlugin;

impl Plugin for StatePlugin {
    fn build(&self, app: &mut App) {
        // `NetPlugin` has already inserted the server's world when playing online.
        if !app.world().contains_resource::<Game>() {
            let mut state = WorldState::new(generate(START_WORLD), RULES_SEED);
            state.join(LOCAL_PLAYER);
            app.insert_resource(Game(state, LOCAL_PLAYER));
        }
        app.add_message::<ActionMsg>()
            .add_message::<GameEvent>()
            .add_message::<ActionResult>()
            .add_message::<AdminMsg>()
            .add_message::<SystemLine>()
            .configure_sets(Update, (Phase::Input, Phase::Apply, Phase::View).chain())
            .add_systems(Update, apply_actions.in_set(Phase::Apply));
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per message stream
fn apply_actions(
    time: Res<Time>,
    mut game: ResMut<Game>,
    net: Option<ResMut<Net>>,
    mut actions: MessageReader<ActionMsg>,
    mut admins: MessageReader<AdminMsg>,
    mut events: MessageWriter<GameEvent>,
    mut results: MessageWriter<ActionResult>,
    mut lines: MessageWriter<SystemLine>,
) {
    let now = time.elapsed_secs_f64();
    if let Some(mut net) = net {
        for &ActionMsg(action) in actions.read() {
            net.act(action);
        }
        net.pump(&mut game, now, &mut events, &mut results);
        return;
    }
    let me = game.1;
    for &ActionMsg(action) in actions.read() {
        let result = apply(&mut game.0, me, action, now);
        for event in result.iter().flatten() {
            events.write(GameEvent(event.clone()));
        }
        results.write(ActionResult { action, result });
    }
    for &AdminMsg(admin) in admins.read() {
        let held = held_before(&game.0, &admin);
        let result = apply_admin(&mut game.0, me, admin, now);
        for event in result.iter().flatten() {
            events.write(GameEvent(event.clone()));
        }
        lines.write(SystemLine(report(&admin, held, &result)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::message::Messages;
    use game_shared::items::FIST;
    use game_shared::rules::Layer;

    fn send(app: &mut App, action: Action) -> (Vec<Event>, Result<Vec<Event>, Reject>) {
        app.world_mut().write_message(ActionMsg(action));
        app.update();
        let world = app.world_mut();
        let events: Vec<Event> = world
            .resource_mut::<Messages<GameEvent>>()
            .drain()
            .map(|GameEvent(event)| event)
            .collect();
        let mut results = world.resource_mut::<Messages<ActionResult>>();
        let mut results: Vec<_> = results.drain().collect();
        assert_eq!(results.len(), 1);
        (events, results.remove(0).result)
    }

    #[test]
    fn actions_go_through_apply_and_come_back_as_messages() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatePlugin));
        let door = app.world().resource::<Game>().0.world.door;
        // The surface dirt diagonally below the player, who stands on the door tile.
        let (x, y) = (door.0 + 1, door.1 - 1);
        let before = app.world().resource::<Game>().0.world.get(x, y);

        let damaged = Event::TileDamaged {
            x,
            y,
            layer: Layer::Foreground,
            damage: game_shared::mods::FIST_DAMAGE as u8,
        };
        let (events, result) = send(&mut app, Action::Punch { x, y });
        assert_eq!(events, vec![damaged.clone()]);
        assert_eq!(result, Ok(vec![damaged]));

        // Rejects come back as a result and produce no events.
        let (events, result) = send(&mut app, Action::Punch { x, y });
        assert_eq!((events, result), (vec![], Err(Reject::Cooldown)));
        let (events, result) = send(&mut app, Action::Place { x, y, item: FIST });
        assert_eq!((events, result), (vec![], Err(Reject::Invalid)));

        let game = app.world().resource::<Game>();
        assert_eq!(game.0.world.get(x, y), before);
        assert_eq!(game.0.damage.len(), 1);
    }

    #[test]
    fn a_seed_is_planted_grown_and_harvested_through_the_message_pipeline() {
        use game_shared::farming::grow_seconds;
        use game_shared::items::{DIRT, seed_of};
        use std::time::Duration;

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatePlugin));
        let seed = seed_of(DIRT).expect("dirt has a seed");
        let door = {
            let mut game = app.world_mut().resource_mut::<Game>();
            let me = game.0.players.get_mut(&LOCAL_PLAYER).expect("joined");
            assert_eq!(me.inventory.add(seed, 1), 1);
            game.0.world.door
        };
        // Beside the door, on the surface dirt.
        let (x, y) = (door.0 + 1, door.1);

        let (events, result) = send(&mut app, Action::Place { x, y, item: seed });
        assert!(result.is_ok(), "{result:?}");
        let planted = events.iter().find_map(|event| match event {
            Event::TreePlanted { tree, .. } => Some(*tree),
            _ => None,
        });
        let tree = planted.expect("TreePlanted arrives");
        assert_eq!(tree.item, DIRT);
        let inventory = Event::InventoryChanged {
            player: LOCAL_PLAYER,
            item: seed,
            count: 0,
        };
        assert!(events.contains(&inventory));

        // Too early: the punch only damages the sapling.
        let (events, _) = send(&mut app, Action::Punch { x, y });
        assert!(
            matches!(events.as_slice(), [Event::TileDamaged { .. }]),
            "{events:?}"
        );

        // The same clock `apply` reads moves past the grow time.
        let wait = Duration::from_secs(grow_seconds(1) + 1);
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .advance_by(wait);
        let (events, result) = send(&mut app, Action::Punch { x, y });
        assert!(result.is_ok(), "{result:?}");
        assert!(
            events.contains(&Event::TreeHarvested { x, y }),
            "{events:?}"
        );
        let fruit = events.iter().find_map(|event| match event {
            Event::ItemDropped { drop } if drop.item == DIRT => Some(drop.count),
            _ => None,
        });
        assert_eq!(fruit, Some(u16::from(tree.fruit)));
        let game = app.world().resource::<Game>();
        assert!(game.0.world.extras.is_empty());
        assert_eq!(
            game.0.world.get(x, y).map(|tile| tile.fg),
            Some(game_shared::items::EMPTY)
        );
    }

    #[test]
    fn a_claim_is_bought_placed_and_opened_through_the_message_pipeline() {
        use game_shared::claims::claim_kind;
        use game_shared::items::SMALL_CLAIM;
        use game_shared::world::World;

        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatePlugin));
        let price = claim_kind(SMALL_CLAIM).expect("on the price list").price;
        let door = {
            let mut game = app.world_mut().resource_mut::<Game>();
            game.0.players.get_mut(&LOCAL_PLAYER).expect("joined").gems = price + 7;
            game.0.world.door
        };
        let (x, y) = (door.0 + 2, door.1);

        let (events, result) = send(&mut app, Action::Buy { item: SMALL_CLAIM });
        assert!(result.is_ok(), "{result:?}");
        let paid = Event::GemsChanged {
            player: LOCAL_PLAYER,
            gems: 7,
        };
        assert!(events.contains(&paid), "{events:?}");
        let (_, result) = send(&mut app, Action::Buy { item: SMALL_CLAIM });
        assert_eq!(result, Err(Reject::NotEnoughGems));

        let place = Action::Place {
            x,
            y,
            item: SMALL_CLAIM,
        };
        let (events, _) = send(&mut app, place);
        let placed = events.iter().find_map(|event| match event {
            Event::ClaimPlaced { claim, .. } => Some(claim.clone()),
            _ => None,
        });
        let claim = placed.expect("ClaimPlaced arrives");
        assert_eq!(
            (claim.owner, claim.tiles.len(), claim.public),
            (LOCAL_PLAYER, 10, false)
        );

        let (events, _) = send(&mut app, Action::SetClaimPublic { x, y, public: true });
        assert!(matches!(events.as_slice(), [Event::ClaimUpdated { claim, .. }] if claim.public));
        let game = app.world().resource::<Game>();
        let key = World::tile_key(x, y).expect("in the world");
        assert!(game.0.world.claim(key).is_some_and(|claim| claim.public));
    }
}
