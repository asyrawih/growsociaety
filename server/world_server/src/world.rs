//! One Tokio task per loaded world. It alone owns the `WorldState`, takes commands one at a time from
//! its queue, and is the only caller of `apply`, `step` and `collect_drops` on the server. No locks
//! around world state, and no two commands ever interleave (docs/knowledge/10, "One world, one task").

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use game_protocol::{ChatKind, ServerMsg, WorldSnapshot};
use game_shared::commands::Role;
use game_shared::physics::{Input, step_with};
use game_shared::rules::{
    Action, Event, PlayerId, WorldState, apply, collect_drops, end_trades_of,
};
use game_shared::worldgen::generate;
use tokio::sync::{mpsc, oneshot};

use crate::accounts::Accounts;
use crate::conn::Server;
use crate::leases::Identity;
use crate::store::{PlayerSave, encode_world};

/// How often a world with players is saved even if nothing asked for it. Tuning: the most a crash
/// can roll back.
const SAVE_INTERVAL: Duration = Duration::from_secs(60);
/// How often everyone gets the authoritative bodies.
const BODIES_INTERVAL: Duration = Duration::from_millis(50);
/// Commands queued for one world before senders start waiting.
const WORLD_QUEUE: usize = 1024;
/// Messages queued for one connection; a client this far behind is dropped.
pub const OUTBOX: usize = 512;

/// Loaded worlds by name. The lock guards the map only, never a world.
pub type Registry = Arc<Mutex<HashMap<String, mpsc::Sender<ToWorld>>>>;

pub enum ToWorld {
    Join {
        id: PlayerId,
        name: String,
        country: Option<String>,
        /// From the connection's token, never from a message (docs/knowledge/13).
        role: Role,
        /// The player's saved rows, loaded by the connection; `None` = a new player.
        save: Option<Box<PlayerSave>>,
        out: mpsc::Sender<Out>,
    },
    Leave {
        id: PlayerId,
    },
    /// Answered once everything before it is saved, so a leaving player's next login reads
    /// their latest inventory.
    Flush {
        done: oneshot::Sender<()>,
    },
    Input {
        id: PlayerId,
        seq: u32,
        input: Input,
    },
    Act {
        id: PlayerId,
        seq: u32,
        action: Action,
    },
    /// Already cleaned by the connection (`clean_chat`).
    Chat {
        id: PlayerId,
        text: String,
    },
    /// Rate-limited by the connection; ownership is checked here (`Room::voice_mute`).
    VoiceMute {
        id: PlayerId,
        player: PlayerId,
        muted: bool,
    },
}

/// A message for one connection, and how to send it.
#[derive(Debug)]
pub enum Out {
    Reliable(ServerMsg),
    /// Latest-wins data; may be lost.
    Datagram(ServerMsg),
}

struct Member {
    out: mpsc::Sender<Out>,
    ack_seq: u32,
}

/// The server's clock: seconds since it started. Every `now` passed to `apply` comes from here.
#[derive(Clone, Copy)]
pub struct Clock(pub Instant);

impl Clock {
    pub fn now(self) -> f64 {
        self.0.elapsed().as_secs_f64()
    }
}

/// Sends `msg` to the world `name`, loading it first if needed. The registry lock is held across
/// the send, and a world only unloads under that lock with an empty queue, so a join is never lost.
pub fn send_to(server: &Server, name: &str, msg: ToWorld) -> bool {
    let Ok(mut worlds) = server.registry.lock() else {
        return false;
    };
    let tx = worlds.entry(name.to_owned()).or_insert_with(|| {
        spawn_world(
            server.registry.clone(),
            server.clock,
            server.accounts.clone(),
            server.identity.clone(),
            name.to_owned(),
        )
    });
    tx.try_send(msg).is_ok()
}

fn spawn_world(
    registry: Registry,
    clock: Clock,
    store: Arc<Accounts>,
    me: Identity,
    name: String,
) -> mpsc::Sender<ToWorld> {
    let (tx, mut rx) = mpsc::channel(WORLD_QUEUE);
    // Messages queue in `rx` while the world loads.
    tokio::spawn(async move {
        if let Some(holder) = store.claim_world(&me, &name).await {
            // Another server runs it: send everyone who asks there, then forget the world.
            // ponytail: a redirect message the client follows comes with the second server.
            let reason = format!("{name} is running on another server ({holder}), try again soon");
            loop {
                while let Ok(msg) = rx.try_recv() {
                    if let ToWorld::Join { out, .. } = msg {
                        let _ = out.try_send(Out::Reliable(ServerMsg::Kicked {
                            reason: reason.clone(),
                        }));
                    }
                }
                let Ok(mut worlds) = registry.lock() else {
                    return;
                };
                if rx.is_empty() {
                    worlds.remove(&name);
                    return;
                }
            }
        }
        let state = match store.load_world(&name).await {
            Some(state) => state,
            None => {
                // A new world gets a random seed for its rules; the seed is saved with it.
                let mut seed = [0u8; 8];
                let _ = getrandom::fill(&mut seed);
                WorldState::new(generate(&name), u64::from_le_bytes(seed))
            }
        };
        run_world(registry, clock, store, me, name, state, rx).await;
    });
    tx
}

/// Events only their own player may see: inventories and gems are private.
fn is_private(event: &Event) -> Option<PlayerId> {
    match *event {
        Event::InventoryChanged { player, .. }
        | Event::GemsChanged { player, .. }
        | Event::BackpackUpgraded { player, .. }
        | Event::InventoryMoved { player, .. }
        | Event::Crafted { player, .. }
        | Event::XpChanged { player, .. } => Some(player),
        _ => None,
    }
}

/// Who may see an event; `None` = everyone in the world. Trades are seen by their two parties only;
/// `parties` remembers them for the events that carry just the trade id.
fn audience(event: &Event, parties: &HashMap<u32, [PlayerId; 2]>) -> Option<Vec<PlayerId>> {
    if let Some(owner) = is_private(event) {
        return Some(vec![owner]);
    }
    Some(match event {
        Event::TradeRequested { from, to, .. } => vec![*from, *to],
        Event::TradeUpdated { trade } => vec![trade.a, trade.b],
        // Unknown id: nobody, rather than everyone.
        Event::TradeCompleted { trade } | Event::TradeCancelled { trade, .. } => {
            parties.get(trade).map_or_else(Vec::new, |p| p.to_vec())
        }
        _ => return None,
    })
}

/// The world's state and its connections. Every method runs inside the world task.
pub struct Room {
    pub state: WorldState,
    members: HashMap<PlayerId, Member>,
    /// Display names of the members.
    pub(crate) names: HashMap<PlayerId, String>,
    /// Country codes of the members who gave one.
    pub(crate) countries: HashMap<PlayerId, String>,
    pub(crate) roles: HashMap<PlayerId, Role>,
    /// Who each member has hidden with `/ignore`.
    pub(crate) ignores: HashMap<PlayerId, HashSet<PlayerId>>,
    /// The other side of each member's last private message, for `/r`.
    pub(crate) last_private: HashMap<PlayerId, PlayerId>,
    /// World bans by lower-case name, until this server time. ponytail: kept in memory, lost when
    /// the world unloads; persisted with sanctions in phase C4.
    pub(crate) bans: HashMap<String, f64>,
    /// Append-only audit log (JSON lines) of staff commands and world-owner punishments.
    pub audit: Option<PathBuf>,
    /// Save at the end of this step (someone left, a trade completed, a flush was asked).
    pub(crate) save_due: bool,
    /// Flushes waiting for the next save.
    pub(crate) acks: Vec<oneshot::Sender<()>>,
    /// Players who left since the last save; they are no longer in `state.players`.
    pub(crate) leavers: Vec<PlayerSave>,
    /// The two players of each open trade, for routing its events (see `audience`).
    pub(crate) trade_parties: HashMap<u32, [PlayerId; 2]>,
    /// This world's voice room; `None` when voice is off (docs/knowledge/12, `voice.rs`).
    pub(crate) voice: Option<crate::voice::Voice>,
}

impl Room {
    pub fn new(state: WorldState) -> Self {
        Self {
            state,
            members: HashMap::new(),
            names: HashMap::new(),
            countries: HashMap::new(),
            roles: HashMap::new(),
            ignores: HashMap::new(),
            last_private: HashMap::new(),
            bans: HashMap::new(),
            audit: None,
            save_due: false,
            acks: Vec::new(),
            leavers: Vec::new(),
            trade_parties: HashMap::new(),
            voice: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub(crate) fn send(&mut self, id: PlayerId, out: Out) {
        // A full outbox means the client stopped reading: drop it, its connection task notices.
        if let Some(member) = self.members.get(&id)
            && member.out.try_send(out).is_err()
        {
            self.remove(id);
        }
    }

    /// Sends `events` to everyone except `skip`, each receiving only what they may see.
    pub(crate) fn broadcast(&mut self, events: &[Event], skip: Option<PlayerId>) {
        for event in events {
            match event {
                Event::TradeRequested { trade, from, to } => {
                    self.trade_parties.insert(*trade, [*from, *to]);
                }
                Event::TradeUpdated { trade } => {
                    self.trade_parties.insert(trade.id, [trade.a, trade.b]);
                }
                _ => {}
            }
        }
        let ids: Vec<PlayerId> = self.members.keys().copied().collect();
        for id in ids.into_iter().filter(|&id| Some(id) != skip) {
            let visible: Vec<Event> = events
                .iter()
                .filter(|e| audience(e, &self.trade_parties).is_none_or(|who| who.contains(&id)))
                .cloned()
                .collect();
            if !visible.is_empty() {
                let msg = ServerMsg::Events { events: visible };
                self.send(id, Out::Reliable(msg));
            }
        }
        for event in events {
            if let Event::TradeCompleted { trade } | Event::TradeCancelled { trade, .. } = event {
                self.trade_parties.remove(trade);
            }
        }
    }

    pub fn handle(&mut self, msg: ToWorld, now: f64) {
        match msg {
            ToWorld::Join {
                id,
                name,
                country,
                role,
                save,
                out,
            } => {
                if let Some(minutes) = self.banned_minutes(&name, now) {
                    let reason =
                        format!("You are banned from this world for {minutes} more minutes");
                    let _ = out.try_send(Out::Reliable(ServerMsg::Kicked { reason }));
                    return;
                }
                self.roles.insert(id, role);
                self.state.join(id);
                if let (Some(save), Some(player)) = (save, self.state.players.get_mut(&id)) {
                    save.apply_to(player);
                }
                self.names.insert(id, name.clone());
                if let Some(code) = &country {
                    self.countries.insert(id, code.clone());
                }
                let Some(mut snapshot) = WorldSnapshot::of(&self.state, id, &self.names) else {
                    return;
                };
                snapshot.countries = self
                    .countries
                    .iter()
                    .map(|(&id, code)| (id, code.clone()))
                    .collect();
                let (body, equipped, level) =
                    (snapshot.you.body, snapshot.you.equipped, snapshot.you.level);
                let snapshot = Box::new(snapshot);
                if out
                    .try_send(Out::Reliable(ServerMsg::Joined { snapshot }))
                    .is_err()
                {
                    self.state.players.remove(&id);
                    self.names.remove(&id);
                    self.countries.remove(&id);
                    return;
                }
                let joined = ServerMsg::PlayerJoined {
                    id,
                    body,
                    equipped,
                    level,
                    name,
                    country,
                };
                for member in self.members.values() {
                    let _ = member.out.try_send(Out::Reliable(joined.clone()));
                }
                self.members.insert(id, Member { out, ack_seq: 0 });
                self.voice_joined(id);
            }
            ToWorld::Leave { id } => self.remove(id),
            ToWorld::Flush { done } => {
                self.acks.push(done);
                self.save_due = true;
            }
            ToWorld::VoiceMute { id, player, muted } => {
                if self.members.contains_key(&id) {
                    self.voice_mute(id, player, muted);
                }
            }
            ToWorld::Chat { id, text } => {
                if self.members.contains_key(&id) {
                    self.chat(id, text, now);
                }
            }
            ToWorld::Input { id, seq, input } => {
                let Some(member) = self.members.get_mut(&id) else {
                    return;
                };
                member.ack_seq = seq;
                let state = &mut self.state;
                let Some(player) = state.players.get_mut(&id) else {
                    return;
                };
                // The server's own step is the position; the client only ever predicts it.
                let mods = game_shared::mods::worn_mods(&player.equipped);
                step_with(&mut player.body, input, &state.world, mods);
                let events = collect_drops(state, id);
                self.broadcast(&events, None);
            }
            ToWorld::Act { id, seq, action } => {
                if !self.members.contains_key(&id) {
                    return;
                }
                let result = apply(&mut self.state, id, action, now);
                let traded = result.as_ref().is_ok_and(|events| {
                    events
                        .iter()
                        .any(|e| matches!(e, Event::TradeCompleted { .. }))
                });
                // Both inventories reach the database together, before anything else happens.
                self.save_due |= traded;
                let left = result
                    .as_ref()
                    .is_ok_and(|events| events.contains(&Event::LeftWorld { player: id }));
                if let Ok(events) = &result {
                    self.broadcast(events, Some(id));
                }
                self.send(id, Out::Reliable(ServerMsg::Acted { seq, result }));
                if left {
                    // ponytail: leaving through the door ends the visit; world travel comes with a world menu.
                    self.remove(id);
                }
            }
        }
    }

    /// Everyone in the world, the speaker included, gets the line as the server cleaned it.
    fn chat(&mut self, id: PlayerId, text: String, now: f64) {
        if text.starts_with('/') {
            return self.command(id, &text, now);
        }
        let ids: Vec<PlayerId> = self
            .members
            .keys()
            .copied()
            .filter(|&to| !self.ignoring(to, id))
            .collect();
        for to in ids {
            let msg = ServerMsg::Chat {
                from: id,
                text: text.clone(),
                kind: ChatKind::World,
            };
            self.send(to, Out::Reliable(msg));
        }
    }

    pub(crate) fn remove(&mut self, id: PlayerId) {
        if self.members.remove(&id).is_none() {
            return;
        }
        // An open trade with them ends (nothing has moved yet); the other party is told.
        let ended = end_trades_of(&mut self.state, id);
        self.broadcast(&ended, Some(id));
        // Their final inventory is saved before the player leaves `state.players`.
        if let Some(player) = self.state.players.get(&id) {
            self.leavers.push(PlayerSave::of(player));
        }
        self.save_due = true;
        self.state.players.remove(&id);
        self.names.remove(&id);
        self.countries.remove(&id);
        self.roles.remove(&id);
        self.ignores.remove(&id);
        self.last_private.remove(&id);
        self.voice_left(id);
        let left = ServerMsg::PlayerLeft { id };
        for member in self.members.values() {
            let _ = member.out.try_send(Out::Reliable(left.clone()));
        }
    }

    /// Everyone's authoritative body, as one datagram per member.
    pub fn send_bodies(&mut self) {
        let bodies: Vec<_> = self
            .state
            .players
            .values()
            .map(|p| (p.id, p.body))
            .collect();
        let ids: Vec<PlayerId> = self.members.keys().copied().collect();
        for id in ids {
            let (Some(member), Some(you)) = (self.members.get(&id), self.state.players.get(&id))
            else {
                continue;
            };
            let msg = ServerMsg::Bodies {
                ack_seq: member.ack_seq,
                you: you.body,
                others: bodies.iter().filter(|b| b.0 != id).copied().collect(),
            };
            self.send(id, Out::Datagram(msg));
        }
    }
}

/// Saves the world and everyone in it (and who just left) in one transaction, then answers flushes.
async fn save(room: &mut Room, store: &Accounts, name: &str) {
    let mut players: Vec<PlayerSave> = room.state.players.values().map(PlayerSave::of).collect();
    players.append(&mut room.leavers);
    if let Err(error) = store.save(name, &encode_world(&room.state), &players).await {
        eprintln!("world {name}: save failed, the last save stays current: {error}");
    }
    room.save_due = false;
    for ack in room.acks.drain(..) {
        let _ = ack.send(());
    }
}

async fn run_world(
    registry: Registry,
    clock: Clock,
    store: Arc<Accounts>,
    me: Identity,
    name: String,
    state: WorldState,
    mut rx: mpsc::Receiver<ToWorld>,
) {
    let mut room = Room::new(state);
    room.audit = Some(
        std::env::var("GAME_AUDIT_LOG")
            .unwrap_or_else(|_| "audit.log".into())
            .into(),
    );
    room.voice = crate::voice::Voice::for_world(&name);
    let mut bodies = tokio::time::interval(BODIES_INTERVAL);
    let mut saves =
        tokio::time::interval_at(tokio::time::Instant::now() + SAVE_INTERVAL, SAVE_INTERVAL);
    let refresh = crate::voice::TOKEN_REFRESH;
    let mut voice_tokens = tokio::time::interval_at(tokio::time::Instant::now() + refresh, refresh);
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(msg) => room.handle(msg, clock.now()),
                None => return,
            },
            _ = bodies.tick() => room.send_bodies(),
            _ = saves.tick() => room.save_due = !room.is_empty(),
            _ = voice_tokens.tick() => room.refresh_voice_tokens(),
        }
        if room.save_due {
            save(&mut room, &store, &name).await;
        }
        if room.is_empty() {
            // Unload only with nothing queued, under the registry lock (see `send_to`). Everyone
            // who left was saved just above, together with the world.
            let unloaded = match registry.lock() {
                Ok(mut worlds) if rx.is_empty() => worlds.remove(&name).is_some(),
                Ok(_) => false,
                Err(_) => return,
            };
            if unloaded {
                store.release_world(&me, &name).await;
                println!("world {name} unloaded");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_shared::items::DIRT;

    fn join(room: &mut Room, id: u32) -> mpsc::Receiver<Out> {
        let (out, rx) = mpsc::channel(OUTBOX);
        room.handle(
            ToWorld::Join {
                id: PlayerId(id),
                name: format!("p{id}"),
                country: None,
                role: Role::Player,
                save: None,
                out,
            },
            0.0,
        );
        rx
    }

    fn drain(rx: &mut mpsc::Receiver<Out>) -> Vec<ServerMsg> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|out| match out {
                Out::Reliable(msg) | Out::Datagram(msg) => msg,
            })
            .collect()
    }

    #[test]
    fn actions_answer_the_actor_and_others_see_only_public_events() {
        let mut room = Room::new(WorldState::new(generate("ROOM"), 1));
        let (a, b) = (PlayerId(1), PlayerId(2));
        let mut rx_a = join(&mut room, 1);
        let mut rx_b = join(&mut room, 2);
        assert!(
            matches!(drain(&mut rx_a).as_slice(), [ServerMsg::Joined { .. }, ServerMsg::PlayerJoined { id, .. }] if *id == b)
        );
        assert!(matches!(
            drain(&mut rx_b).as_slice(),
            [ServerMsg::Joined { .. }]
        ));

        room.state
            .players
            .get_mut(&a)
            .expect("joined")
            .inventory
            .add(DIRT, 3);
        let door = room.state.world.door;
        let action = Action::Drop {
            item: DIRT,
            count: 1,
            left: false,
        };
        room.handle(
            ToWorld::Act {
                id: a,
                seq: 1,
                action,
            },
            1.0,
        );

        let to_a = drain(&mut rx_a);
        let [
            ServerMsg::Acted {
                seq: 1,
                result: Ok(events),
            },
        ] = to_a.as_slice()
        else {
            panic!("{to_a:?}");
        };
        assert!(events.iter().any(|e| is_private(e) == Some(a)));
        let to_b = drain(&mut rx_b);
        let [ServerMsg::Events { events }] = to_b.as_slice() else {
            panic!("{to_b:?}");
        };
        assert!(events.iter().all(|e| is_private(e).is_none()), "{events:?}");
        assert!(matches!(events.as_slice(), [Event::ItemDropped { drop }] if drop.y == door.1));
    }

    #[test]
    fn moving_an_item_is_private_and_reorders_the_saved_inventory() {
        let mut room = Room::new(WorldState::new(generate("MOVE"), 1));
        let (a, b) = (PlayerId(1), PlayerId(2));
        let mut rx_a = join(&mut room, 1);
        let mut rx_b = join(&mut room, 2);
        drain(&mut rx_a);
        drain(&mut rx_b);
        room.state
            .players
            .get_mut(&a)
            .expect("joined")
            .inventory
            .add(DIRT, 4);
        let last = room.state.players[&a].inventory.slots().len() as u16 - 1;
        let action = Action::MoveItem { from: last, to: 0 };
        room.handle(
            ToWorld::Act {
                id: a,
                seq: 1,
                action,
            },
            1.0,
        );
        let to_a = drain(&mut rx_a);
        assert!(
            matches!(to_a.as_slice(), [ServerMsg::Acted { result: Ok(events), .. }]
            if events == &[Event::InventoryMoved { player: a, from: last, to: 0 }]),
            "{to_a:?}"
        );
        assert!(drain(&mut rx_b).is_empty(), "nobody else sees it");
        // What the next save writes is the new order.
        let saved = crate::store::PlayerSave::of(&room.state.players[&a]);
        assert_eq!(saved.items.first().map(|s| s.0), Some(DIRT));
        let bad = Action::MoveItem { from: 99, to: 0 };
        room.handle(
            ToWorld::Act {
                id: a,
                seq: 2,
                action: bad,
            },
            1.0,
        );
        assert!(matches!(
            drain(&mut rx_a).as_slice(),
            [ServerMsg::Acted { result: Err(_), .. }]
        ));
        let _ = b;
    }

    #[test]
    fn the_server_steps_inputs_and_acks_them() {
        let mut room = Room::new(WorldState::new(generate("STEP"), 1));
        let id = PlayerId(5);
        let mut rx = join(&mut room, 5);
        let start = room.state.players[&id].body.pos;
        let right = Input {
            move_x: 1,
            ..Input::default()
        };
        for seq in 1..=30 {
            room.handle(
                ToWorld::Input {
                    id,
                    seq,
                    input: right,
                },
                0.0,
            );
        }
        room.send_bodies();
        let msgs = drain(&mut rx);
        let Some(ServerMsg::Bodies { ack_seq, you, .. }) = msgs.last() else {
            panic!("{msgs:?}");
        };
        assert_eq!(*ack_seq, 30);
        assert!(you.pos.0 > start.0);
        assert_eq!(*you, room.state.players[&id].body);
    }

    #[test]
    fn chat_reaches_everyone_and_commands_answer_only_the_sender() {
        let mut room = Room::new(WorldState::new(generate("TALK"), 1));
        let mut rx_a = join(&mut room, 1);
        let mut rx_b = join(&mut room, 2);
        drain(&mut rx_a);
        drain(&mut rx_b);
        let (a, b) = (PlayerId(1), PlayerId(2));

        room.handle(
            ToWorld::Chat {
                id: a,
                text: "hello".into(),
            },
            0.0,
        );
        let said = ServerMsg::Chat {
            from: a,
            text: "hello".into(),
            kind: ChatKind::World,
        };
        assert_eq!(drain(&mut rx_a), vec![said.clone()]);
        assert_eq!(drain(&mut rx_b), vec![said]);

        room.handle(
            ToWorld::Chat {
                id: b,
                text: "/who".into(),
            },
            0.0,
        );
        assert!(drain(&mut rx_a).is_empty());
        assert!(matches!(
            drain(&mut rx_b).as_slice(),
            [ServerMsg::Chat {
                kind: ChatKind::System,
                ..
            }]
        ));
    }

    #[test]
    fn a_dead_connection_is_removed_and_others_are_told() {
        let mut room = Room::new(WorldState::new(generate("GONE"), 1));
        let mut rx_a = join(&mut room, 1);
        let rx_b = join(&mut room, 2);
        drop(rx_b);
        room.send_bodies();
        assert!(!room.state.players.contains_key(&PlayerId(2)));
        let msgs = drain(&mut rx_a);
        assert!(msgs.contains(&ServerMsg::PlayerLeft { id: PlayerId(2) }));
    }
}
