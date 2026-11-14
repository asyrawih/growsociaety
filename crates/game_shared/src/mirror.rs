//! A client's copy of a world, kept in step by the server's `Event`s (docs/knowledge/10).
//! `apply` decides; `apply_event` only records what was decided, in the order it was sent.
//! The test replays `apply`'s own events onto a copy and expects the same world.

use crate::rules::{Damage, Event, WorldState};
use crate::trade::Trade;
use crate::world::{TileExtra, World};

/// Records one event in `state`. `now` is the receiver's clock, for the damage it shows.
/// Events about players or tiles the copy does not have are ignored.
pub fn apply_event(state: &mut WorldState, event: &Event, now: f64) {
    match event {
        &Event::TileDamaged {
            x,
            y,
            layer,
            damage,
        } => {
            if let Some(key) = World::tile_key(x, y) {
                let damage = Damage {
                    hits: damage,
                    layer,
                    last_hit: now,
                };
                state.damage.insert(key, damage);
            }
        }
        &Event::TileChanged { x, y, tile } => {
            let (Some(before), Some(key)) = (state.world.get(x, y), World::tile_key(x, y)) else {
                return;
            };
            state.world.set(x, y, tile);
            // Whatever hung on the old foreground (tree, claim) went with it; a broken or replaced
            // layer starts undamaged. A tree or claim placed here arrives as its own event next.
            if before.fg != tile.fg {
                state.world.remove_extra(key);
                state.damage.remove(&key);
            } else if before.bg != tile.bg {
                state.damage.remove(&key);
            }
        }
        Event::ItemDropped { drop } => {
            match state.world.drops.iter_mut().find(|d| d.id == drop.id) {
                Some(existing) => *existing = *drop,
                None => state.world.drops.push(*drop),
            }
            state.world.next_drop_id = state.world.next_drop_id.max(drop.id.saturating_add(1));
        }
        &Event::DropTaken { drop_id, count, .. } => {
            state.world.drops.retain_mut(|drop| {
                if drop.id == drop_id {
                    drop.count = drop.count.saturating_sub(count);
                }
                drop.count > 0
            });
        }
        &Event::InventoryChanged {
            player,
            item,
            count,
        } => {
            if let Some(player) = state.players.get_mut(&player) {
                let held = player.inventory.count(item);
                if count > held {
                    player.inventory.add(item, count - held);
                } else if count < held {
                    player.inventory.remove(item, held - count);
                }
            }
        }
        &Event::GemsChanged { player, gems } => {
            if let Some(player) = state.players.get_mut(&player) {
                player.gems = gems;
            }
        }
        &Event::BackpackUpgraded { player, capacity } => {
            if let Some(player) = state.players.get_mut(&player) {
                player.inventory.capacity = capacity;
            }
        }
        &Event::Equipped { player, slot, item } => {
            let entry = state
                .players
                .get_mut(&player)
                .and_then(|player| player.equipped.get_mut(slot as usize));
            if let Some(entry) = entry {
                *entry = item;
            }
        }
        &Event::TreePlanted { x, y, tree } | &Event::TreeSpliced { x, y, tree, .. } => {
            if let Some(key) = World::tile_key(x, y) {
                state.world.extras.insert(key, TileExtra::Tree(tree));
                state.damage.remove(&key);
            }
        }
        Event::ClaimPlaced { x, y, claim } | Event::ClaimUpdated { x, y, claim } => {
            if let Some(key) = World::tile_key(*x, *y) {
                state.world.insert_claim(key, claim.clone());
            }
        }
        &Event::ClaimRemoved { x, y } => {
            if let Some(key) = World::tile_key(x, y) {
                state.world.remove_extra(key);
            }
        }
        &Event::TradeRequested { trade, from, to } => {
            state.trades.retain(|open| open.id != trade);
            state.trades.push(Trade::new(trade, from, to));
        }
        Event::TradeUpdated { trade } => {
            match state.trades.iter_mut().find(|open| open.id == trade.id) {
                Some(open) => *open = trade.clone(),
                None => state.trades.push(trade.clone()),
            }
        }
        // The inventory changes of a completed trade follow as their own events.
        &Event::TradeCompleted { trade } | &Event::TradeCancelled { trade, .. } => {
            state.trades.retain(|open| open.id != trade);
        }
        &Event::XpChanged { player, level, xp } => {
            if let Some(player) = state.players.get_mut(&player) {
                player.level = level;
                player.xp = xp;
            }
        }
        // Others' XP is private; their level is enough for the name colour.
        &Event::LevelUp { player, level } => {
            if let Some(player) = state.players.get_mut(&player) {
                player.level = level;
            }
        }
        // The tile change came as its own event; leaving is handled by whoever owns the connection.
        Event::TreeHarvested { .. } | Event::LeftWorld { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::claim_kind;
    use crate::items::{DIRT, HAIR_BROWN, SHIRT_RED, SMALL_CLAIM, STONE, Slot, seed_of};
    use crate::rules::{Action, PUNCH_COOLDOWN, PlayerId, apply, collect_drops};
    use crate::trade::TRADE_REVIEW_DELAY;
    use crate::worldgen::generate;

    /// The fields a client sees; `last_punch`, the RNG and damage timestamps are the server's business.
    fn visible(state: &WorldState) -> impl PartialEq + std::fmt::Debug {
        let mut players: Vec<_> = state
            .players
            .values()
            .map(|p| (p.id, p.inventory.clone(), p.gems, p.equipped, p.level, p.xp))
            .collect();
        players.sort_by_key(|p| p.0.0);
        let mut damage: Vec<_> = state
            .damage
            .iter()
            .map(|(&key, d)| (key, d.hits, d.layer))
            .collect();
        damage.sort_by_key(|d| d.0);
        let mut world = state.world.clone();
        world.drops.sort_by_key(|d| d.id);
        (world, players, damage, state.trades.clone())
    }

    #[test]
    fn replaying_the_events_of_apply_rebuilds_the_same_world() {
        let (me, you) = (PlayerId(1), PlayerId(2));
        let mut server = WorldState::new(generate("MIRROR"), 7);
        server.join(me);
        server.join(you);
        assert_eq!(
            server
                .players
                .get_mut(&you)
                .expect("joined")
                .inventory
                .add(STONE, 3),
            3
        );
        let door = server.world.door;
        let seed = seed_of(DIRT).expect("dirt has a seed");
        {
            let player = server.players.get_mut(&me).expect("joined");
            player.gems = claim_kind(SMALL_CLAIM).expect("listed").price;
            player.inventory.add(seed, 5);
            player.inventory.add(DIRT, 5);
        }
        let mut client = server.clone();

        let (x, y) = (door.0 + 1, door.1 - 1);
        let mut now = 0.0;
        let mut actions = vec![Action::Punch { x, y }; 3];
        actions.extend([
            Action::Place { x, y, item: DIRT },
            Action::Place {
                x: door.0 + 2,
                y: door.1,
                item: seed,
            },
            Action::Buy { item: SMALL_CLAIM },
            Action::Place {
                x: door.0 - 1,
                y: door.1,
                item: SMALL_CLAIM,
            },
            Action::SetClaimPublic {
                x: door.0 - 1,
                y: door.1,
                public: true,
            },
            Action::Drop {
                item: DIRT,
                count: 2,
                left: false,
            },
        ]);
        actions.extend(
            [Action::Punch {
                x: door.0 - 1,
                y: door.1,
            }; 8],
        );
        // Equipment: off, on again, and the last worn copy dropped (which takes it off).
        actions.extend([
            Action::Unequip { slot: Slot::Hair },
            Action::Equip { item: HAIR_BROWN },
            Action::Drop {
                item: SHIRT_RED,
                count: 1,
                left: true,
            },
        ]);
        // A whole trade: request, accept, offers, locks, the review delay, both confirms.
        let trade = [
            (me, Action::TradeRequest { to: you }),
            (you, Action::TradeRespond { accept: true }),
            (
                me,
                Action::TradeSetOffer {
                    slot: 0,
                    item: seed,
                    count: 1,
                },
            ),
            (
                you,
                Action::TradeSetOffer {
                    slot: 2,
                    item: STONE,
                    count: 3,
                },
            ),
            (me, Action::TradeLock { locked: true }),
            (you, Action::TradeLock { locked: true }),
            (me, Action::TradeConfirm),
            (you, Action::TradeConfirm),
        ];
        let mut steps: Vec<_> = actions.into_iter().map(|action| (me, action)).collect();
        steps.extend(trade);
        let mut applied = 0;
        let mut xp_events = 0;
        let mut traded = false;
        for (player, action) in steps {
            now += PUNCH_COOLDOWN + 0.01;
            if action == Action::TradeConfirm {
                now += TRADE_REVIEW_DELAY;
            }
            let result = apply(&mut server, player, action, now);
            let mut events = result.unwrap_or_default();
            applied += usize::from(!events.is_empty());
            xp_events += events
                .iter()
                .filter(|e| matches!(e, Event::XpChanged { .. }))
                .count();
            traded |= events
                .iter()
                .any(|e| matches!(e, Event::TradeCompleted { .. }));
            events.extend(collect_drops(&mut server, me));
            for event in &events {
                apply_event(&mut client, event, now);
            }
            assert_eq!(visible(&client), visible(&server), "after {action:?}");
        }
        assert!(
            applied >= 18,
            "the script must exercise the rules, {applied} applied"
        );
        assert!(
            xp_events > 0 && traded,
            "{xp_events} XP events, traded: {traded}"
        );
        assert!(server.players[&me].xp > 0);
        assert_eq!(server.players[&me].inventory.count(STONE), 3);
    }
}
