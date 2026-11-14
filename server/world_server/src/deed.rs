//! World Deeds on the server (docs/knowledge/ux/world-key.md). The `world_deeds` row is the deed;
//! a player's ring mirrors the rows they hold. Make, tear up, discard and trade run through the
//! rules and reach the database in the world's save transaction, with the inventories, so a failed
//! or crashed save moves nothing. Using a deed changes another world, so it is carried out there:
//! one transaction deletes the deed row (only if the user still holds it) and stores the world with
//! its new owner; only then does the world change in memory.

use std::time::Duration;

use game_protocol::{DEED_FAILED, deed_used_text, deed_void_text};
use game_shared::deeds::{self, Deed, WorldName};
use game_shared::rules::{Action, Event, PlayerId, Reject};
use game_shared::trade::Trade;
use tokio::sync::oneshot;

use crate::store::{DeedOp, DeedUse, encode_world};
use crate::world::{Room, ToWorld, WorldLink};

/// How long a member's deed use waits for the other world's answer before it counts as failed.
const USE_WAIT: Duration = Duration::from_secs(10);

/// A use of THIS world's deed, waiting to be carried out (`process`).
pub struct IncomingUse {
    pub user: PlayerId,
    pub user_name: String,
    pub reply: oneshot::Sender<Result<(), String>>,
}

/// What an action's deed effects are judged against: the actor's open trade and the world claim,
/// as they were before the action.
pub struct DeedContext {
    trade: Option<Trade>,
    world_claim: Option<u32>,
}

impl Room {
    pub(crate) fn deed_context(&self, id: PlayerId) -> DeedContext {
        DeedContext {
            trade: self.state.trades.iter().find(|t| t.involves(id)).cloned(),
            world_claim: self.state.world.world_claim,
        }
    }

    fn here(&self) -> Option<WorldName> {
        WorldName::new(&self.state.world.name)
    }

    /// Records the database side of an accepted deed action (for the next save, or a use to send),
    /// and voids this world's deed if its world claim just broke. Returns events to broadcast.
    pub(crate) fn after_deed_action(
        &mut self,
        id: PlayerId,
        action: Action,
        result: &Result<Vec<Event>, Reject>,
        before: DeedContext,
        now: f64,
    ) -> Vec<Event> {
        let Ok(events) = result else {
            return Vec::new();
        };
        match action {
            Action::MakeDeed { .. } => {
                if let Some(world) = self.here() {
                    self.deed_ops.push(DeedOp::Make { world, holder: id });
                    self.save_due = true;
                    self.audit_line(id, &format!("/deed make {world}"), now);
                }
            }
            Action::TearUpDeed { world } => {
                self.deed_ops.push(DeedOp::TearUp { world, holder: id });
                self.save_due = true;
                self.audit_line(id, &format!("/deed tearup {world}"), now);
                if Some(world) != self.here() {
                    self.cleared_elsewhere.push(world);
                }
            }
            Action::DiscardDeed { world } => {
                self.deed_ops.push(DeedOp::Discard { world, holder: id });
                self.save_due = true;
                self.audit_line(id, &format!("/deed discard {world}"), now);
            }
            Action::UseDeed { world } => {
                self.outgoing_uses.push((id, world));
                self.audit_line(id, &format!("/deed use {world}"), now);
            }
            _ => {}
        }
        // A completed trade moves its deeds in the same save as the items.
        if let Some(trade) = before.trade {
            let completed = events
                .iter()
                .any(|e| matches!(e, Event::TradeCompleted { trade: t } if *t == trade.id));
            if completed {
                let [a_gives, b_gives] = &trade.deeds;
                for (gives, from, to) in [(a_gives, trade.a, trade.b), (b_gives, trade.b, trade.a)]
                {
                    for &world in gives {
                        self.deed_ops.push(DeedOp::Move { world, from, to });
                        let line = format!("/deed trade {world} #{} -> #{}", from.0, to.0);
                        self.audit_line(from, &line, now);
                    }
                }
                self.save_due = true;
            }
        }
        if before.world_claim.is_some() && self.state.world.world_claim.is_none() {
            return self.void_deed(id, now);
        }
        Vec::new()
    }

    /// This world's deed becomes void (reset, or the world claim broke): rows marked void in the
    /// next save, rings here greyed now. Returns the events to broadcast.
    pub(crate) fn void_deed(&mut self, actor: PlayerId, now: f64) -> Vec<Event> {
        let Some(world) = self.here() else {
            return Vec::new();
        };
        self.deed_ops.push(DeedOp::Void { world });
        self.save_due = true;
        self.audit_line(actor, &format!("/deed void {world}"), now);
        deeds::void_here(&mut self.state)
    }

    /// The answer for a member's deed use: a System line, and on failure the deed back on the ring
    /// (its row was never touched).
    pub(crate) fn deed_use_result(
        &mut self,
        user: PlayerId,
        deed: Deed,
        outcome: Result<(), String>,
    ) {
        if !self.state.players.contains_key(&user) {
            return; // they left: their next load reads the rings from the database
        }
        match outcome {
            Ok(()) => self.system(user, deed_used_text(deed.world.as_str())),
            Err(text) => {
                let events = deeds::restore(&mut self.state, user, deed);
                self.broadcast(&events, None);
                self.system(user, text);
            }
        }
    }
}

/// The deed work a world does between messages: tell other worlds, send members' uses to the
/// deed's world, and carry out uses of this world's deed.
pub(crate) async fn process(room: &mut Room, link: &WorldLink, name: &str, now: f64) {
    for world in std::mem::take(&mut room.cleared_elsewhere) {
        link.send_if_loaded(world.as_str(), ToWorld::DeedCleared);
    }
    for (user, world) in std::mem::take(&mut room.outgoing_uses) {
        let user_name = room.names.get(&user).cloned().unwrap_or_default();
        let deed = Deed { world, void: false };
        let (reply, answer) = oneshot::channel();
        let asked = IncomingUse {
            user,
            user_name,
            reply,
        };
        if world.as_str() == name {
            room.incoming_uses.push(asked);
        } else {
            let IncomingUse {
                user,
                user_name,
                reply,
            } = asked;
            let msg = ToWorld::UseDeed {
                user,
                user_name,
                reply,
            };
            if !link.send(world.as_str(), msg) {
                room.deed_use_result(user, deed, Err(DEED_FAILED.into()));
                continue;
            }
        }
        // The answer comes back as a message to this world, whichever world carried it out.
        let (link, home) = (link.clone(), name.to_owned());
        tokio::spawn(async move {
            let outcome = match tokio::time::timeout(USE_WAIT, answer).await {
                Ok(Ok(outcome)) => outcome,
                _ => Err(DEED_FAILED.into()),
            };
            let result = ToWorld::DeedUseResult {
                user,
                deed,
                outcome,
            };
            link.send_if_loaded(&home, result);
        });
    }
    for incoming in std::mem::take(&mut room.incoming_uses) {
        let outcome = carry_out(room, link, name, &incoming, now).await;
        let _ = incoming.reply.send(outcome);
    }
}

/// Uses a deed on this world: ONE transaction deletes the deed row (only if `user` still holds a
/// valid one) and stores the world with its new owner; only then does it change here.
async fn carry_out(
    room: &mut Room,
    link: &WorldLink,
    name: &str,
    incoming: &IncomingUse,
    now: f64,
) -> Result<(), String> {
    let user = incoming.user;
    let Some(key) = room.state.world.world_claim else {
        return Err(deed_void_text(name));
    };
    let old_owner = room.state.world.claim(key).map(|claim| claim.owner);
    let mut next = room.state.clone();
    deeds::transfer(&mut next, user).ok_or_else(|| deed_void_text(name))?;
    match link.store.use_deed(name, user, &encode_world(&next)).await {
        Ok(DeedUse::Used) => {}
        Ok(DeedUse::Void) => return Err(deed_void_text(name)),
        Ok(DeedUse::NotHeld) => return Err(DEED_FAILED.into()),
        Err(error) => {
            eprintln!("world {name}: deed use by {}: {error}", user.0);
            return Err(DEED_FAILED.into());
        }
    }
    // Stored: now the same change here (nothing else ran in this task meanwhile).
    let events = deeds::transfer(&mut room.state, user).unwrap_or_default();
    room.broadcast(&events, None);
    room.touched = true;
    room.save_due = true;
    room.audit_line(
        user,
        &format!("/deed used {name} (was #{})", old_owner.map_or(0, |o| o.0)),
        now,
    );
    if let Some(old) = old_owner.filter(|&old| old != user) {
        let text = format!(
            "{name} now belongs to {} (your deed was used).",
            incoming.user_name
        );
        if room.state.players.contains_key(&old) {
            room.system(old, text);
        } else if let Err(error) = link.store.add_notice(old, &text).await {
            eprintln!("world {name}: notice for {}: {error}", old.0);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{Clock, OUTBOX, Out};
    use game_protocol::{ChatKind, ServerMsg};
    use game_shared::claims::Claim;
    use game_shared::commands::Role;
    use game_shared::rules::WorldState;
    use game_shared::world::World;
    use game_shared::worldgen::generate;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    const OWNER: PlayerId = PlayerId(1);
    const BUYER: PlayerId = PlayerId(2);

    fn join(room: &mut Room, id: PlayerId, role: Role) -> mpsc::Receiver<Out> {
        let (out, rx) = mpsc::channel(OUTBOX);
        let name = format!("p{}", id.0);
        let join = ToWorld::Join {
            id,
            name,
            country: None,
            role,
            save: None,
            out,
        };
        room.handle(join, 0.0);
        rx
    }

    fn act(room: &mut Room, id: PlayerId, action: Action, now: f64) {
        room.handle(ToWorld::Act { id, seq: 0, action }, now);
    }

    fn lines(rx: &mut mpsc::Receiver<Out>) -> Vec<String> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|out| match out {
                Out::Reliable(ServerMsg::Chat {
                    text,
                    kind: ChatKind::System,
                    ..
                }) => Some(text),
                _ => None,
            })
            .collect()
    }

    /// FARM, its world claim owned by OWNER (with BUYER on the access list and public on).
    fn farm() -> (Room, (u32, u32), [mpsc::Receiver<Out>; 2]) {
        let mut room = Room::new(WorldState::new(generate("FARM"), 1));
        let rx = [
            join(&mut room, OWNER, Role::Owner),
            join(&mut room, BUYER, Role::Player),
        ];
        let at = (room.state.world.door.0 + 1, room.state.world.door.1);
        let key = World::tile_key(at.0, at.1).unwrap();
        let block = game_shared::world::Tile {
            fg: game_shared::items::WORLD_CLAIM,
            bg: game_shared::items::EMPTY,
        };
        room.state.world.set(at.0, at.1, block);
        let claim = Claim {
            owner: OWNER,
            access: vec![BUYER],
            public: true,
            tiles: vec![],
        };
        room.state.world.insert_claim(key, claim);
        (room, at, rx)
    }

    fn name(world: &str) -> WorldName {
        WorldName::new(world).unwrap()
    }

    #[test]
    fn making_and_tearing_up_reach_the_save_as_ops() {
        let (mut room, (x, y), _rx) = farm();
        act(&mut room, OWNER, Action::MakeDeed { x, y }, 1.0);
        assert_eq!(
            room.deed_ops,
            [DeedOp::Make {
                world: name("FARM"),
                holder: OWNER
            }]
        );
        assert!(room.save_due && room.state.deed_out);
        act(
            &mut room,
            OWNER,
            Action::TearUpDeed {
                world: name("FARM"),
            },
            2.0,
        );
        assert_eq!(
            room.deed_ops[1],
            DeedOp::TearUp {
                world: name("FARM"),
                holder: OWNER
            }
        );
        assert!(!room.state.deed_out);
    }

    #[test]
    fn a_trade_with_deeds_both_ways_moves_them_in_the_same_save() {
        let (mut room, _, _rx) = farm();
        let (a, b) = (name("ALPHA"), name("BETA"));
        room.state
            .players
            .get_mut(&OWNER)
            .unwrap()
            .deeds
            .push(Deed {
                world: a,
                void: false,
            });
        room.state
            .players
            .get_mut(&BUYER)
            .unwrap()
            .deeds
            .push(Deed {
                world: b,
                void: false,
            });
        act(&mut room, OWNER, Action::TradeRequest { to: BUYER }, 0.0);
        act(&mut room, BUYER, Action::TradeRespond { accept: true }, 0.0);
        act(
            &mut room,
            OWNER,
            Action::TradeDeed {
                world: a,
                add: true,
            },
            0.0,
        );
        act(
            &mut room,
            BUYER,
            Action::TradeDeed {
                world: b,
                add: true,
            },
            0.0,
        );
        act(&mut room, OWNER, Action::TradeLock { locked: true }, 1.0);
        act(&mut room, BUYER, Action::TradeLock { locked: true }, 1.0);
        act(&mut room, OWNER, Action::TradeConfirm, 5.0);
        act(&mut room, BUYER, Action::TradeConfirm, 5.0);
        assert_eq!(
            room.deed_ops,
            [
                DeedOp::Move {
                    world: a,
                    from: OWNER,
                    to: BUYER
                },
                DeedOp::Move {
                    world: b,
                    from: BUYER,
                    to: OWNER
                },
            ]
        );
        assert!(room.save_due, "saved with both inventories");
        assert_eq!(
            room.state.players[&BUYER].deeds,
            [Deed {
                world: a,
                void: false
            }]
        );
    }

    #[test]
    fn breaking_the_world_claim_or_a_reset_voids_the_deed() {
        let (mut room, (x, y), _rx) = farm();
        act(&mut room, OWNER, Action::MakeDeed { x, y }, 1.0);
        room.handle(
            ToWorld::Chat {
                id: OWNER,
                text: format!("/clearblock {x} {y}"),
            },
            2.0,
        );
        assert_eq!(
            room.deed_ops.last(),
            Some(&DeedOp::Void {
                world: name("FARM")
            })
        );
        assert!(
            room.state.players[&OWNER].deeds[0].void,
            "greyed on the ring"
        );

        let (mut room, (x, y), _rx) = farm();
        act(&mut room, OWNER, Action::MakeDeed { x, y }, 1.0);
        room.handle(
            ToWorld::Chat {
                id: OWNER,
                text: "/resetworld".into(),
            },
            2.0,
        );
        room.handle(
            ToWorld::Chat {
                id: OWNER,
                text: "/resetworld confirm".into(),
            },
            3.0,
        );
        assert!(room.deed_ops.contains(&DeedOp::Void {
            world: name("FARM")
        }));
        assert!(room.state.players[&OWNER].deeds[0].void);
    }

    #[tokio::test]
    async fn using_a_deed_hands_the_world_over_and_tells_both() {
        let (mut room, (x, y), [mut owner_rx, mut buyer_rx]) = farm();
        act(&mut room, OWNER, Action::MakeDeed { x, y }, 1.0);
        // Hand the deed to BUYER as a trade would, then BUYER uses it here.
        let deed = room
            .state
            .players
            .get_mut(&OWNER)
            .unwrap()
            .deeds
            .pop()
            .unwrap();
        room.state.players.get_mut(&BUYER).unwrap().deeds.push(deed);
        act(
            &mut room,
            BUYER,
            Action::UseDeed {
                world: name("FARM"),
            },
            2.0,
        );
        assert!(
            room.state.players[&BUYER].deeds.is_empty(),
            "locked while in flight"
        );
        let link = WorldLink {
            registry: Default::default(),
            clock: Clock(std::time::Instant::now()),
            store: Arc::new(crate::accounts::Accounts::open(None).await.unwrap()),
            me: crate::leases::Identity::from_env(),
        };
        process(&mut room, &link, "FARM", 2.0).await;
        let claim = room
            .state
            .world
            .claim(room.state.world.world_claim.unwrap())
            .unwrap();
        assert_eq!(claim.owner, BUYER);
        assert!(
            claim.access.is_empty() && !claim.public,
            "a bought world starts clean"
        );
        assert!(!room.state.deed_out);
        assert!(
            lines(&mut owner_rx)
                .iter()
                .any(|l| l == "FARM now belongs to p2 (your deed was used).")
        );
        // The user's answer comes back as a message (a spawned task delivers it).
        room.deed_use_result(BUYER, deed, Ok(()));
        assert!(lines(&mut buyer_rx).iter().any(|l| l == "You now own FARM"));
    }

    #[test]
    fn a_failed_use_puts_the_deed_back() {
        let (mut room, _, [_, mut buyer_rx]) = farm();
        let deed = Deed {
            world: name("ELSEWHERE"),
            void: false,
        };
        room.deed_use_result(BUYER, deed, Err(DEED_FAILED.into()));
        assert_eq!(room.state.players[&BUYER].deeds, [deed]);
        assert!(lines(&mut buyer_rx).iter().any(|l| l == DEED_FAILED));
    }
}
