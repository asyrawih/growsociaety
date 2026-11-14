//! `/commands` inside a world (docs/knowledge/13, phase C2). Parsed by `game_shared::commands`,
//! authorized here (the caller's role comes from the connection, world authority from the world
//! claim), then run. World changes go through `rules::apply_admin` and reach every client as
//! ordinary Events. Staff commands and world-owner punishments, refused ones included, write
//! exactly one audit line each.

use std::io::Write;

use game_protocol::{ChatKind, ServerMsg};
use game_shared::commands::{
    self, Admin, Command, Duration, Requirement, Role, Target, held_before, parse, report, required,
};
use game_shared::physics::spawn_body;
use game_shared::rules::{PlayerId, apply_admin};

use crate::world::{Out, Room};

impl Room {
    pub(crate) fn role(&self, id: PlayerId) -> Role {
        self.roles.get(&id).copied().unwrap_or(Role::Player)
    }

    pub(crate) fn ignoring(&self, who: PlayerId, speaker: PlayerId) -> bool {
        self.ignores
            .get(&who)
            .is_some_and(|set| set.contains(&speaker))
    }

    /// Minutes left on a world ban for `name`, if any.
    pub(crate) fn banned_minutes(&self, name: &str, now: f64) -> Option<u64> {
        let until = *self.bans.get(&name.to_lowercase())?;
        (until > now).then(|| ((until - now) / 60.0).ceil() as u64)
    }

    pub(crate) fn system(&mut self, to: PlayerId, text: String) {
        let msg = ServerMsg::Chat {
            from: to,
            text,
            kind: ChatKind::System,
        };
        self.send(to, Out::Reliable(msg));
    }

    fn name(&self, id: PlayerId) -> String {
        self.names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("Player{}", id.0))
    }

    /// A member of this world, by `#id` or case-insensitive name.
    fn resolve(&self, target: &Target) -> Option<PlayerId> {
        match target {
            Target::Id(id) => self.names.contains_key(id).then_some(*id),
            Target::Name(name) => self
                .names
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(name))
                .map(|(&id, _)| id),
        }
    }

    /// World authority: (is the world claim's owner, is on its access list).
    pub(crate) fn authority(&self, id: PlayerId) -> (bool, bool) {
        let claim = self
            .state
            .world
            .world_claim
            .and_then(|key| self.state.world.claim(key));
        claim.map_or((false, false), |claim| {
            (claim.owner == id, claim.access.contains(&id))
        })
    }

    /// Staff powers include every world power.
    pub(crate) fn allowed(&self, id: PlayerId, needs: Requirement) -> bool {
        let staff = self.role(id) >= Role::Moderator;
        let (owner, access) = self.authority(id);
        match needs {
            Requirement::Anyone => true,
            Requirement::WorldAccess => owner || access || staff,
            Requirement::WorldOwner => owner || staff,
            Requirement::Role(role) => self.role(id) >= role,
        }
    }

    /// World authority never reaches staff, and staff never act on an equal or higher role.
    pub(crate) fn may_act_on(&self, actor: PlayerId, target: PlayerId) -> bool {
        let theirs = self.role(target);
        actor == target || theirs < Role::Moderator || self.role(actor) > theirs
    }

    /// The one place a `/command` arrives.
    pub(crate) fn command(&mut self, id: PlayerId, line: &str, now: f64) {
        let command = match parse(line) {
            Ok(command) => command,
            Err(error) => return self.system(id, error.to_string()),
        };
        let needs = required(&command);
        let audited = matches!(needs, Requirement::Role(_))
            || matches!(
                command,
                Command::Kick { .. }
                    | Command::Pull { .. }
                    | Command::WorldBan { .. }
                    | Command::WorldUnban { .. }
            );
        let outcome = if self.allowed(id, needs) {
            self.run(id, command, now)
        } else {
            Err(format!("You cannot use /{} here", command.name()))
        };
        if audited {
            self.audit(id, line, &outcome, now);
        }
        match outcome {
            Ok(reply) | Err(reply) if !reply.is_empty() => self.system(id, reply),
            _ => {}
        }
    }

    /// Runs an authorized command. `Ok` and `Err` both carry the reply line (empty: none).
    fn run(&mut self, id: PlayerId, command: Command, now: f64) -> Result<String, String> {
        let missing = || "No such player in this world".to_owned();
        match command {
            Command::ResetWorld(step) => self.reset_command(id, step, now),
            Command::Help { topic } => Ok(commands::help(topic.as_deref(), |spec| {
                self.allowed(id, spec.needs)
            })),
            Command::Who => {
                let mut names: Vec<&str> = self.names.values().map(String::as_str).collect();
                names.sort_unstable();
                Ok(format!("In this world: {}", names.join(", ")))
            }
            Command::Msg { to, text } => {
                let to = self.resolve(&to).ok_or_else(missing)?;
                self.private(id, to, text, now)
            }
            Command::Reply { text } => {
                let to = self
                    .last_private
                    .get(&id)
                    .copied()
                    .filter(|to| self.names.contains_key(to))
                    .ok_or("Nobody to reply to")?;
                self.private(id, to, text, now)
            }
            Command::Ignore { who, on } => {
                let who = self.resolve(&who).ok_or_else(missing)?;
                if on && self.role(who) >= Role::Moderator {
                    return Err("Staff cannot be ignored".into());
                }
                let set = self.ignores.entry(id).or_default();
                if on {
                    set.insert(who);
                } else {
                    set.remove(&who);
                }
                self.send_ignoring(id);
                let verb = if on { "Ignoring" } else { "No longer ignoring" };
                Ok(format!("{verb} {}", self.name(who)))
            }
            Command::Access { who, on } => {
                // Texts as in docs/knowledge/ux/claims.md (the panel's Add errors). A name must be
                // someone in this world; `#<id>` reaches anyone.
                let target = match &who {
                    Target::Id(id) => *id,
                    Target::Name(name) => self
                        .resolve(&who)
                        .ok_or_else(|| format!("No player called {name}"))?,
                };
                let (x, y) = self
                    .nearest_own_claim(id)
                    .ok_or("No claim of yours within reach")?;
                let key = game_shared::world::World::tile_key(x, y)
                    .ok_or("No claim of yours within reach")?;
                let listed = self
                    .state
                    .world
                    .claim(key)
                    .map(|c| (c.access.contains(&target), c.access.len()));
                let name = self.name(target);
                match (on, listed) {
                    (true, _) if target == id => return Err("You are the owner".into()),
                    (true, Some((true, _))) => {
                        return Err(format!("{name} can already build here"));
                    }
                    (true, Some((false, n))) if n >= game_shared::claims::MAX_CLAIM_ACCESS => {
                        return Err(format!(
                            "The list is full ({})",
                            game_shared::claims::MAX_CLAIM_ACCESS
                        ));
                    }
                    _ => {}
                }
                let action = game_shared::rules::Action::SetClaimAccess {
                    x,
                    y,
                    player: target,
                    allow: on,
                };
                match game_shared::rules::apply(&mut self.state, id, action, now) {
                    Ok(events) => {
                        self.broadcast(&events, None);
                        let verb = if on {
                            "can build here now"
                        } else {
                            "can no longer build here"
                        };
                        Ok(format!("{name} {verb}"))
                    }
                    Err(reject) => Err(format!("Cannot change access: {reject:?}")),
                }
            }
            Command::Trade { who } => {
                let to = self.resolve(&who).ok_or_else(missing)?;
                // The trade rules live in `apply`; the parties see its events.
                let result = game_shared::rules::apply(
                    &mut self.state,
                    id,
                    game_shared::rules::Action::TradeRequest { to },
                    now,
                );
                match result {
                    Ok(events) => {
                        self.broadcast(&events, None);
                        Ok(format!("Asked {} to trade", self.name(to)))
                    }
                    Err(reject) => Err(format!("Cannot trade: {reject:?}")),
                }
            }
            Command::Kick { who } | Command::Pull { who } if self.resolve(&who).is_none() => {
                Err(missing())
            }
            Command::Kick { who } => {
                let who = self.resolve(&who).ok_or_else(missing)?;
                self.target_ok(id, who)?;
                let door = spawn_body(&self.state.world);
                if let Some(player) = self.state.players.get_mut(&who) {
                    player.body = door;
                }
                let by = self.name(id);
                self.system(who, format!("{by} sent you to the main door"));
                Ok(format!("Sent {} to the main door", self.name(who)))
            }
            Command::Pull { who } => {
                let who = self.resolve(&who).ok_or_else(missing)?;
                self.target_ok(id, who)?;
                let here = self.state.players.get(&id).map(|p| p.body);
                if let (Some(here), Some(player)) = (here, self.state.players.get_mut(&who)) {
                    player.body.pos = here.pos;
                    player.body.vel = (0.0, 0.0);
                }
                let by = self.name(id);
                self.system(who, format!("{by} pulled you"));
                Ok(format!("Pulled {}", self.name(who)))
            }
            Command::WorldBan { who, length } => {
                let member = self.resolve(&who);
                if let Some(member) = member {
                    self.target_ok(id, member)?;
                }
                let name = match (&who, member) {
                    (_, Some(member)) => self.name(member),
                    (Target::Name(name), None) => name.clone(),
                    (Target::Id(_), None) => return Err(missing()),
                };
                let minutes = match length {
                    Duration::Minutes(minutes) => minutes,
                    Duration::Permanent => return Err("World bans are temporary".into()),
                };
                self.bans
                    .insert(name.to_lowercase(), now + f64::from(minutes) * 60.0);
                if let Some(member) = member {
                    let reason = format!("You were banned from this world for {minutes} minutes");
                    self.send(member, Out::Reliable(ServerMsg::Kicked { reason }));
                    self.remove(member);
                }
                Ok(format!(
                    "Banned {name} from this world for {minutes} minutes"
                ))
            }
            Command::WorldUnban { who: None } => {
                self.bans.clear();
                Ok("Lifted every world ban".into())
            }
            Command::WorldUnban { who: Some(who) } => {
                let name = match &who {
                    Target::Name(name) => name.to_lowercase(),
                    Target::Id(_) => return Err("Unban by name".into()),
                };
                match self.bans.remove(&name) {
                    Some(_) => Ok(format!("Lifted the world ban on {name}")),
                    None => Err(format!("{name} is not banned here")),
                }
            }
            command => {
                let admin = command
                    .admin(|target| self.resolve(target))
                    .ok_or_else(missing)?;
                // Taking from or changing the gems of an equal or higher role is refused (the
                // Owner is above everyone).
                if let Admin::Take { from: target, .. }
                | Admin::Gems { who: target, .. }
                | Admin::ClearInv { from: target } = admin
                {
                    self.target_ok(id, target)?;
                }
                let held = held_before(&self.state, &admin);
                let world_claim = self.state.world.world_claim;
                let result = apply_admin(&mut self.state, id, admin, now);
                // An admin clearing the world claim block voids the world's deed too.
                if world_claim.is_some() && self.state.world.world_claim.is_none() {
                    let voided = self.void_deed(id, now);
                    self.broadcast(&voided, None);
                }
                self.touched |= result.is_ok();
                if let Ok(events) = &result {
                    self.broadcast(events, None);
                }
                let line = report(&admin, held, &result);
                if result.is_ok() { Ok(line) } else { Err(line) }
            }
        }
    }

    /// The block of the claim `id` owns that is closest to them and within their reach.
    fn nearest_own_claim(&self, id: PlayerId) -> Option<(u32, u32)> {
        let player = self.state.players.get(&id)?;
        let mods = game_shared::mods::worn_mods(&player.equipped);
        let (px, py) = game_shared::world_to_tile(player.body.pos.0, player.body.pos.1)?;
        self.state
            .world
            .extras
            .iter()
            .filter(|(_, extra)| matches!(extra, game_shared::world::TileExtra::Claim(c) if c.owner == id))
            .filter_map(|(&key, _)| game_shared::world::World::tile_xy(key))
            .filter(|&(x, y)| game_shared::rules::in_reach_with(&player.body, x, y, mods))
            .min_by_key(|&(x, y)| x.abs_diff(px).max(y.abs_diff(py)))
    }

    fn target_ok(&self, actor: PlayerId, target: PlayerId) -> Result<(), String> {
        if self.may_act_on(actor, target) {
            Ok(())
        } else {
            Err(format!("You cannot do that to {}", self.name(target)))
        }
    }

    fn private(
        &mut self,
        from: PlayerId,
        to: PlayerId,
        text: String,
        now: f64,
    ) -> Result<String, String> {
        // An ignoring receiver hears nothing; the sender is not told (no ignore probing).
        if !self.ignoring(to, from) {
            self.heard(to, from, &text, now);
            let msg = ServerMsg::Chat {
                from,
                text: text.clone(),
                kind: ChatKind::Private,
            };
            self.send(to, Out::Reliable(msg));
            self.last_private.insert(to, from);
        }
        self.last_private.insert(from, to);
        Ok(format!("To {}: {text}", self.name(to)))
    }

    /// An audit line for something that is not a typed command (a report from the avatar menu).
    pub(crate) fn audit_line(&self, actor: PlayerId, line: &str, now: f64) {
        self.audit(actor, line, &Ok("stored".into()), now);
    }

    /// One JSON line: time, actor, role, the command as typed, world, result.
    fn audit(&self, actor: PlayerId, line: &str, outcome: &Result<String, String>, now: f64) {
        let Some(path) = &self.audit else { return };
        let (ok, reply) = match outcome {
            Ok(reply) => (true, reply.as_str()),
            Err(reply) => (false, reply.as_str()),
        };
        let entry = format!(
            "{{\"at\":{now:.3},\"actor\":{},\"name\":{},\"role\":\"{:?}\",\"command\":{},\"world\":{},\"ok\":{ok},\"result\":{}}}\n",
            actor.0,
            json(&self.name(actor)),
            self.role(actor),
            json(line),
            json(&self.state.world.name),
            json(reply),
        );
        // ponytail: a file per server, one blocking append per entry; a table in phase C4.
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(entry.as_bytes()));
        if let Err(error) = written {
            eprintln!("audit log {}: {error}", path.display());
        }
    }
}

/// A JSON string literal.
fn json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::ToWorld;
    use game_shared::claims::Claim;
    use game_shared::items::DIRT;
    use game_shared::rules::{Event, WorldState};
    use game_shared::worldgen::generate;
    use tokio::sync::mpsc;

    struct Seat {
        id: PlayerId,
        rx: mpsc::Receiver<Out>,
    }

    fn room() -> Room {
        Room::new(WorldState::new(generate("CMD"), 1))
    }

    fn join(room: &mut Room, id: u32, name: &str, role: Role) -> Seat {
        let (out, rx) = mpsc::channel(crate::world::OUTBOX);
        let id = PlayerId(id);
        room.handle(
            ToWorld::Join {
                id,
                name: name.into(),
                country: None,
                role,
                save: None,
                out,
            },
            0.0,
        );
        let mut seat = Seat { id, rx };
        seat.drain();
        seat
    }

    impl Seat {
        fn drain(&mut self) -> Vec<ServerMsg> {
            std::iter::from_fn(|| self.rx.try_recv().ok())
                .map(|out| match out {
                    Out::Reliable(msg) | Out::Datagram(msg) => msg,
                })
                .collect()
        }

        /// The System replies received since the last drain.
        fn replies(&mut self) -> Vec<String> {
            self.drain()
                .into_iter()
                .filter_map(|msg| match msg {
                    ServerMsg::Chat {
                        text,
                        kind: ChatKind::System,
                        ..
                    } => Some(text),
                    _ => None,
                })
                .collect()
        }
    }

    fn say(room: &mut Room, seat: &Seat, line: &str) {
        room.handle(
            ToWorld::Chat {
                id: seat.id,
                text: line.into(),
            },
            1.0,
        );
    }

    fn claim_world(room: &mut Room, owner: PlayerId, access: Vec<PlayerId>) {
        let claim = Claim {
            owner,
            access,
            public: false,
            tiles: vec![],
        };
        room.state.world.insert_claim(0, claim);
    }

    #[test]
    fn only_admins_create_value_and_the_result_reaches_everyone() {
        let mut room = room();
        let mut admin = join(&mut room, 1, "Root", Role::Admin);
        let mut player = join(&mut room, 2, "Budi", Role::Player);

        say(&mut room, &player, "/give Budi dirt 5 cheat");
        assert_eq!(player.replies(), ["You cannot use /give here"]);
        assert_eq!(room.state.players[&player.id].inventory.count(DIRT), 0);

        say(&mut room, &admin, "/give budi dirt 5 event");
        assert_eq!(admin.replies(), ["Gave 5 dirt"]);
        // The inventory change is private: only Budi gets it, as an ordinary Event.
        let got = player.drain();
        assert!(got.iter().any(|m| matches!(m, ServerMsg::Events { events }
            if events.contains(&Event::InventoryChanged { player: PlayerId(2), item: DIRT, count: 5 }))));
        say(&mut room, &admin, "/give Nobody dirt 5 x");
        assert_eq!(admin.replies(), ["No such player in this world"]);
    }

    #[test]
    fn a_player_can_run_no_staff_or_world_command() {
        let mut room = room();
        let mut player = join(&mut room, 2, "Budi", Role::Player);
        let _other = join(&mut room, 3, "Cici", Role::Player);
        let before = room.state.clone();
        for line in [
            "/spawn dirt",
            "/give Budi dirt 5 x",
            "/take Cici dirt 1 x",
            "/gems Budi +9 x",
            "/setblock 5 30 dirt",
            "/clearblock 5 30",
            "/giveall Budi",
            "/kick Cici",
            "/pull Cici",
            "/wban Cici",
            "/wunban all",
        ] {
            say(&mut room, &player, line);
            let name = line.split_whitespace().next().unwrap_or("");
            assert_eq!(
                player.replies(),
                [format!("You cannot use {name} here")],
                "{line}"
            );
        }
        assert_eq!(room.state, before, "nothing changed");
    }

    #[test]
    fn nobody_acts_on_the_owner_and_only_the_owner_gives_all() {
        let mut room = room();
        let mut owner = join(&mut room, 1, "hanin", Role::Owner);
        let mut admin = join(&mut room, 2, "Root", Role::Admin);
        say(&mut room, &admin, "/pull hanin");
        assert_eq!(admin.replies(), ["You cannot do that to hanin"]);
        say(&mut room, &admin, "/take hanin dirt 1 x");
        assert_eq!(admin.replies(), ["You cannot do that to hanin"]);
        say(&mut room, &admin, "/gems hanin -5 x");
        assert_eq!(admin.replies(), ["You cannot do that to hanin"]);
        say(&mut room, &admin, "/giveall Root");
        assert_eq!(admin.replies(), ["You cannot use /giveall here"]);
        let mut other_owner = join(&mut room, 3, "asyrawih", Role::Owner);
        say(&mut room, &other_owner, "/clearinv hanin oops");
        assert_eq!(
            other_owner.replies(),
            ["You cannot do that to hanin"],
            "not another Owner"
        );
        say(&mut room, &admin, "/clearinv Root x");
        assert_eq!(admin.replies(), ["You cannot use /clearinv here"]);
        say(&mut room, &owner, "/giveall hanin");
        assert!(owner.replies()[0].starts_with("Gave full stacks of"));
        assert_eq!(room.state.players[&owner.id].inventory.capacity, 396);
    }

    #[test]
    fn authority_rules() {
        let mut room = room();
        let mut owner = join(&mut room, 1, "Owner", Role::Player);
        let mut helper = join(&mut room, 2, "Helper", Role::Player);
        let mut moderator = join(&mut room, 3, "Mod", Role::Moderator);
        let mut admin = join(&mut room, 4, "Root", Role::Admin);
        let mut other = join(&mut room, 5, "Other", Role::Player);
        claim_world(&mut room, owner.id, vec![helper.id]);

        // Access holders kick but cannot ban.
        say(&mut room, &helper, "/kick Other");
        assert_eq!(helper.replies(), ["Sent Other to the main door"]);
        assert_eq!(other.replies(), ["Helper sent you to the main door"]);
        say(&mut room, &helper, "/wban Other");
        assert_eq!(helper.replies(), ["You cannot use /wban here"]);
        // A stranger has no world authority.
        say(&mut room, &other, "/kick Helper");
        assert_eq!(other.replies(), ["You cannot use /kick here"]);
        // World authority never reaches staff.
        say(&mut room, &owner, "/kick Mod");
        assert_eq!(owner.replies(), ["You cannot do that to Mod"]);
        say(&mut room, &owner, "/wban Root");
        assert_eq!(owner.replies(), ["You cannot do that to Root"]);
        // A moderator cannot act on an admin; an admin can act on a moderator.
        say(&mut room, &moderator, "/pull Root");
        assert_eq!(moderator.replies(), ["You cannot do that to Root"]);
        say(&mut room, &admin, "/pull Mod");
        assert_eq!(admin.replies(), ["Pulled Mod"]);
        assert_eq!(moderator.replies(), ["Root pulled you"]);
        // A moderator is not an admin.
        say(&mut room, &moderator, "/spawn dirt");
        assert_eq!(moderator.replies(), ["You cannot use /spawn here"]);
    }

    #[test]
    fn a_world_ban_blocks_joining_that_world_only() {
        let mut room = room();
        let mut owner = join(&mut room, 1, "Owner", Role::Player);
        let mut budi = join(&mut room, 2, "Budi", Role::Player);
        claim_world(&mut room, owner.id, vec![]);

        say(&mut room, &owner, "/wban Budi 30m");
        assert_eq!(
            owner.replies(),
            ["Banned Budi from this world for 30 minutes"]
        );
        assert!(
            budi.drain()
                .iter()
                .any(|m| matches!(m, ServerMsg::Kicked { .. }))
        );
        assert!(!room.state.players.contains_key(&budi.id));

        // Rejoining with the same name, even as a new connection, is refused until it expires.
        let (out, mut rx) = mpsc::channel(crate::world::OUTBOX);
        let join = |out| ToWorld::Join {
            id: PlayerId(9),
            name: "budi".into(),
            country: None,
            role: Role::Player,
            save: None,
            out,
        };
        room.handle(join(out.clone()), 60.0);
        assert!(matches!(
            rx.try_recv(),
            Ok(Out::Reliable(ServerMsg::Kicked { .. }))
        ));
        room.handle(join(out), 31.0 * 60.0);
        assert!(matches!(
            rx.try_recv(),
            Ok(Out::Reliable(ServerMsg::Joined { .. }))
        ));
        // A different world has its own (empty) ban list.
        assert!(
            Room::new(WorldState::new(generate("OTHER"), 1))
                .banned_minutes("budi", 0.0)
                .is_none()
        );
    }

    #[test]
    fn trade_requests_reach_only_the_two_players() {
        let mut room = room();
        let mut a = join(&mut room, 1, "Ana", Role::Player);
        let mut b = join(&mut room, 2, "Budi", Role::Player);
        let mut c = join(&mut room, 3, "Cici", Role::Player);
        say(&mut room, &a, "/trade budi");
        let to_a = a.drain();
        assert!(
            to_a.iter().any(
                |m| matches!(m, ServerMsg::Chat { text, .. } if text == "Asked Budi to trade")
            )
        );
        let requested = |msgs: &[ServerMsg]| {
            msgs.iter().any(|m| {
                matches!(m, ServerMsg::Events { events }
            if events.iter().any(|e| matches!(e, Event::TradeRequested { .. })))
            })
        };
        assert!(requested(&to_a));
        assert!(requested(&b.drain()));
        assert!(
            !requested(&c.drain()),
            "a third player sees nothing of the trade"
        );
        // Leaving ends the trade for the other side, and only they hear of it.
        room.handle(ToWorld::Leave { id: a.id }, 2.0);
        let cancelled = |msgs: &[ServerMsg]| {
            msgs.iter().any(|m| {
                matches!(m, ServerMsg::Events { events }
            if events.iter().any(|e| matches!(e, Event::TradeCancelled { .. })))
            })
        };
        assert!(cancelled(&b.drain()));
        assert!(!cancelled(&c.drain()));
    }

    #[test]
    fn access_on_and_off_for_the_nearest_own_claim() {
        let mut room = room();
        let mut a = join(&mut room, 1, "Ana", Role::Player);
        let mut b = join(&mut room, 2, "Budi", Role::Player);
        say(&mut room, &a, "/access Budi on");
        assert_eq!(a.replies(), ["No claim of yours within reach"]);
        let (dx, dy) = room.state.world.door;
        room.state
            .players
            .get_mut(&a.id)
            .expect("joined")
            .inventory
            .add(game_shared::items::SMALL_CLAIM, 1);
        let place = game_shared::rules::Action::Place {
            x: dx + 1,
            y: dy,
            item: game_shared::items::SMALL_CLAIM,
        };
        room.handle(
            ToWorld::Act {
                id: a.id,
                seq: 1,
                action: place,
            },
            1.0,
        );
        a.drain();
        b.drain();
        say(&mut room, &a, "/access budi on");
        assert_eq!(a.replies(), ["Budi can build here now"]);
        say(&mut room, &a, "/access Budi on");
        assert_eq!(a.replies(), ["Budi can already build here"]);
        say(&mut room, &a, "/access Ana on");
        assert_eq!(a.replies(), ["You are the owner"]);
        say(&mut room, &a, "/access Zed on");
        assert_eq!(a.replies(), ["No player called Zed"]);
        let key = game_shared::world::World::tile_key(dx + 1, dy).expect("in world");
        assert!(
            room.state
                .world
                .claim(key)
                .is_some_and(|c| c.access == [b.id])
        );
        say(&mut room, &a, "/access #2 off");
        assert!(
            room.state
                .world
                .claim(key)
                .is_some_and(|c| c.access.is_empty())
        );
        // Only an owner: Budi has no claim here.
        say(&mut room, &b, "/access Ana on");
        assert_eq!(b.replies(), ["No claim of yours within reach"]);
    }

    #[test]
    fn private_messages_replies_and_ignore() {
        let mut room = room();
        let mut a = join(&mut room, 1, "Ana", Role::Player);
        let mut b = join(&mut room, 2, "Budi", Role::Player);
        let mut m = join(&mut room, 3, "Mod", Role::Moderator);

        say(&mut room, &a, "/msg budi hi");
        assert_eq!(a.replies(), ["To Budi: hi"]);
        let got = b.drain();
        assert!(got.contains(&ServerMsg::Chat {
            from: a.id,
            text: "hi".into(),
            kind: ChatKind::Private
        }));
        say(&mut room, &b, "/r hey");
        assert!(a.drain().contains(&ServerMsg::Chat {
            from: b.id,
            text: "hey".into(),
            kind: ChatKind::Private
        }));

        say(&mut room, &b, "/ignore Ana");
        b.drain();
        say(&mut room, &a, "hello all");
        assert!(b.drain().is_empty(), "ignored speakers are not heard");
        say(&mut room, &b, "/ignore Mod");
        assert_eq!(b.replies(), ["Staff cannot be ignored"]);
        m.drain();
    }

    #[test]
    fn every_staff_command_writes_exactly_one_audit_line() {
        let path = std::env::temp_dir().join(format!("audit-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut room = room();
        room.audit = Some(path.clone());
        let admin = join(&mut room, 1, "Root", Role::Admin);
        let player = join(&mut room, 2, "Budi", Role::Player);
        let owner_less = join(&mut room, 3, "Cici", Role::Player);

        say(&mut room, &admin, "/spawn dirt 2"); // staff, ok
        say(&mut room, &player, "/gems Budi +9 me"); // staff, refused
        say(&mut room, &owner_less, "/kick Budi"); // world punishment, refused
        say(&mut room, &player, "/who"); // not audited
        say(&mut room, &player, "/help"); // not audited

        let log = std::fs::read_to_string(&path).expect("audit written");
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 3, "{log}");
        assert!(lines[0].contains("\"ok\":true") && lines[0].contains("/spawn dirt 2"));
        assert!(lines[1].contains("\"ok\":false") && lines[1].contains("\"role\":\"Player\""));
        assert!(lines[2].contains("/kick Budi"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn json_escapes() {
        assert_eq!(json("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }
}

#[cfg(test)]
mod reset_tests {
    use super::*;
    use crate::store::PlayerSave;
    use crate::world::ToWorld;
    use game_shared::claims::Claim;
    use game_shared::commands::RESET_CONFIRM_SECS;
    use game_shared::items::{SMALL_CLAIM, item};
    use game_shared::rules::WorldState;
    use game_shared::world::{Tile, World};
    use game_shared::worldgen::generate;
    use tokio::sync::mpsc;

    struct Seat {
        id: PlayerId,
        rx: mpsc::Receiver<Out>,
    }

    impl Seat {
        fn drain(&mut self) -> Vec<ServerMsg> {
            std::iter::from_fn(|| self.rx.try_recv().ok())
                .map(|out| match out {
                    Out::Reliable(msg) | Out::Datagram(msg) => msg,
                })
                .collect()
        }

        fn replies(&mut self) -> Vec<String> {
            self.drain()
                .into_iter()
                .filter_map(|msg| match msg {
                    ServerMsg::Chat {
                        text,
                        kind: ChatKind::System,
                        ..
                    } => Some(text),
                    _ => None,
                })
                .collect()
        }
    }

    fn join(room: &mut Room, id: u32, role: Role, save: Option<PlayerSave>) -> Seat {
        let (out, rx) = mpsc::channel(crate::world::OUTBOX);
        let id = PlayerId(id);
        room.handle(
            ToWorld::Join {
                id,
                name: format!("p{}", id.0),
                country: None,
                role,
                save: save.map(Box::new),
                out,
            },
            0.0,
        );
        Seat { id, rx }
    }

    fn say(room: &mut Room, seat: &Seat, line: &str, now: f64) {
        let text = line.into();
        room.handle(ToWorld::Chat { id: seat.id, text }, now);
    }

    /// A small claim block owned by `owner` at `(x, 30)`.
    fn claim_at(room: &mut Room, x: u32, owner: PlayerId) {
        let world = &mut room.state.world;
        world.set(
            x,
            30,
            Tile {
                fg: SMALL_CLAIM,
                bg: game_shared::items::EMPTY,
            },
        );
        let key = World::tile_key(x, 30).expect("in range");
        let claim = Claim {
            owner,
            access: vec![],
            public: false,
            tiles: vec![key],
        };
        world.insert_claim(key, claim);
    }

    fn room() -> Room {
        Room::new(WorldState::new(generate("RESET"), 1))
    }

    #[test]
    fn only_the_owner_and_only_with_a_timely_confirm_by_the_same_caller() {
        let mut room = room();
        let mut owner = join(&mut room, 1, Role::Owner, None);
        let mut admin = join(&mut room, 2, Role::Admin, None);
        claim_at(&mut room, 10, admin.id);
        let before = room.state.world.clone();
        owner.drain();
        admin.drain();

        say(&mut room, &admin, "/resetworld", 1.0);
        assert_eq!(admin.replies(), ["You cannot use /resetworld here"]);

        say(&mut room, &owner, "/resetworld", 1.0);
        let prepared = owner.replies();
        assert!(prepared[0].starts_with("Reset RESET? This wipes 1 claims (owned by 1 players)"));
        assert!(
            prepared[0].ends_with("2 players are inside. Type /resetworld confirm within 15s.")
        );
        assert_eq!(room.state.world, before, "prepare changes nothing");

        say(&mut room, &admin, "/resetworld confirm", 2.0);
        assert_eq!(admin.replies(), ["You cannot use /resetworld here"]);
        say(
            &mut room,
            &owner,
            "/resetworld confirm",
            1.0 + RESET_CONFIRM_SECS + 0.1,
        );
        assert_eq!(
            owner.replies(),
            ["Nothing to confirm. Run /resetworld again"]
        );
        assert_eq!(room.state.world, before, "too late changes nothing");

        say(&mut room, &owner, "/resetworld", 30.0);
        say(&mut room, &owner, "/resetworld cancel", 31.0);
        say(&mut room, &owner, "/resetworld confirm", 32.0);
        assert_eq!(
            owner.replies().last().unwrap(),
            "Nothing to confirm. Run /resetworld again"
        );
        say(&mut room, &owner, "/resetworld OTHER", 33.0);
        assert!(owner.replies()[0].starts_with("Join OTHER first"));
        assert_eq!(room.state.world, before);
    }

    #[test]
    fn a_reset_returns_claims_wipes_the_world_and_moves_everyone_to_the_door() {
        let mut room = room();
        let mut owner = join(&mut room, 1, Role::Owner, None);
        let mut builder = join(&mut room, 2, Role::Player, None);
        let away = PlayerId(99);
        claim_at(&mut room, 10, builder.id);
        claim_at(&mut room, 20, away);
        room.state.world.drops.clear();
        if let Some(player) = room.state.players.get_mut(&builder.id) {
            player.body.pos.0 += 64.0;
        }
        let held = room.state.players[&builder.id].inventory.count(SMALL_CLAIM);
        owner.drain();
        builder.drain();

        say(&mut room, &owner, "/resetworld seed=5", 1.0);
        say(&mut room, &owner, "/resetworld confirm", 2.0);
        assert_eq!(
            owner.replies().last().unwrap(),
            "RESET was reset (2 claims returned, 0 trees, 0 drops removed)."
        );

        let world = &room.state.world;
        assert!(world.extras.is_empty() && world.drops.is_empty() && world.world_claim.is_none());
        let door = spawn_body(world);
        for player in room.state.players.values() {
            assert_eq!(player.body.pos, door.pos);
        }
        assert_eq!(
            room.state.players[&builder.id].inventory.count(SMALL_CLAIM),
            held + 1
        );
        let msgs = builder.drain();
        assert!(
            msgs.iter().any(|m| matches!(m, ServerMsg::Joined { .. })),
            "new world sent"
        );
        let name = item(SMALL_CLAIM).unwrap().name;
        assert!(msgs.iter().any(|m| matches!(m, ServerMsg::Chat { text, .. }
            if text.starts_with("Your ") && text.to_lowercase().contains(name))));
        assert_eq!(
            room.returned.len(),
            1,
            "the absent owner's claim waits in the store"
        );
        assert_eq!(room.returned[0].player, away);
        assert!(room.backup.is_some() && room.save_due);
    }

    #[test]
    fn a_join_shows_returned_notices_and_pays_nothing_itself() {
        // `load_player` already paid in the database; a world save that fails afterwards cannot
        // pay again, because the world never pays on join.
        let mut room = room();
        let notice = "Your Small Claim in START was returned because the owner reset the world.";
        let save = PlayerSave {
            id: PlayerId(3),
            gems: 0,
            capacity: game_shared::inventory::Inventory::default().capacity,
            items: vec![(SMALL_CLAIM, 1)],
            equipped: [None; game_shared::items::SLOT_COUNT],
            level: 1,
            xp: 0,
            notices: vec![notice.into()],
            ignores: vec![],
            deeds: vec![],
        };
        let mut seat = join(&mut room, 3, Role::Player, Some(save));
        assert_eq!(room.state.players[&seat.id].inventory.count(SMALL_CLAIM), 1);
        assert!(seat.replies().iter().any(|line| line == notice));
    }

    fn ignoring_msgs(seat: &mut Seat) -> Vec<Vec<PlayerId>> {
        seat.drain()
            .into_iter()
            .filter_map(|m| match m {
                ServerMsg::Ignoring { ids } => Some(ids),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_saved_ignore_list_applies_on_join_and_changes_are_sent_and_saved() {
        let mut room = room();
        let mut other = join(&mut room, 2, Role::Player, None);
        other.drain();
        let save = PlayerSave {
            id: PlayerId(1),
            gems: 0,
            capacity: game_shared::inventory::Inventory::default().capacity,
            items: vec![],
            equipped: [None; game_shared::items::SLOT_COUNT],
            level: 1,
            xp: 0,
            notices: vec![],
            ignores: vec![PlayerId(2)],
            deeds: vec![],
        };
        let mut me = join(&mut room, 1, Role::Player, Some(save));
        assert_eq!(ignoring_msgs(&mut me), [vec![PlayerId(2)]], "sent on join");
        assert!(room.ignoring(PlayerId(1), PlayerId(2)));
        // Muted in this world too: their world chat does not reach me.
        say(&mut room, &other, "hello", 1.0);
        assert!(
            !me.drain()
                .iter()
                .any(|m| matches!(m, ServerMsg::Chat { text, .. } if text == "hello"))
        );

        say(&mut room, &me, "/unignore p2", 2.0);
        assert_eq!(ignoring_msgs(&mut me), [Vec::<PlayerId>::new()]);
        say(&mut room, &me, "/ignore p2", 3.0);
        assert_eq!(ignoring_msgs(&mut me), [vec![PlayerId(2)]]);
        let saved = room.save_of(&room.state.players[&PlayerId(1)]);
        assert_eq!(saved.ignores, [PlayerId(2)], "goes into the save");
    }

    #[test]
    fn a_report_gets_the_lines_the_reporter_heard_and_an_audit_line() {
        let path = std::env::temp_dir().join(format!("report-audit-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut room = room();
        room.audit = Some(path.clone());
        let mut me = join(&mut room, 1, Role::Player, None);
        let them = join(&mut room, 2, Role::Player, None);
        me.drain();
        say(&mut room, &them, "buy cheap gems at scam.example", 1.0);
        let ask = |target| {
            let (reply, answer) = tokio::sync::oneshot::channel();
            let msg = ToWorld::Report {
                id: PlayerId(1),
                target: PlayerId(target),
                reason: game_protocol::ReportReason::Spam,
                note: Some("spamming".into()),
                reply,
            };
            (msg, answer)
        };
        let (msg, mut answer) = ask(2);
        room.handle(msg, 2.0);
        let chat = answer
            .try_recv()
            .expect("answered")
            .expect("target is here");
        let lines: serde_json::Value = serde_json::from_str(&chat).expect("json");
        assert_eq!(lines[0]["from"], 2);
        assert_eq!(lines[0]["text"], "buy cheap gems at scam.example");
        let log = std::fs::read_to_string(&path).expect("audit written");
        assert!(log.contains("/report #2 spam spamming"), "{log}");

        let (msg, mut answer) = ask(99);
        room.handle(msg, 3.0);
        assert_eq!(
            answer.try_recv().expect("answered"),
            None,
            "not in this world"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn roles_reach_the_client_in_the_snapshot_and_on_joining() {
        let mut room = room();
        let mut staff = join(&mut room, 1, Role::Moderator, None);
        staff.drain();
        let mut player = join(&mut room, 2, Role::Player, None);
        let snapshot_roles = player.drain().into_iter().find_map(|m| match m {
            ServerMsg::Joined { snapshot } => Some(snapshot.roles),
            _ => None,
        });
        let roles = snapshot_roles.expect("joined");
        assert!(roles.contains(&(PlayerId(1), Role::Moderator)), "{roles:?}");
        let announced = staff.drain().into_iter().any(|m| {
            matches!(m, ServerMsg::PlayerJoined { id, role: Role::Player, .. } if id == PlayerId(2))
        });
        assert!(announced);
    }
}
