//! `/resetworld` (docs/knowledge/ux/reset-world.md): the owner regenerates a world. Prepare shows
//! what will go, a confirm within `RESET_CONFIRM_SECS` by the same caller carries it out inside
//! this world's task, so no Action interleaves. Claims go back to their owners: here at once,
//! elsewhere through `returned_claim`, paid out by `load_player` when they next join a world.

use game_protocol::ServerMsg;
use game_shared::claims::claim_kind;
use game_shared::commands::{RESET_CONFIRM_SECS, Reset};
use game_shared::items::{ItemId, item};
use game_shared::physics::spawn_body;
use game_shared::rules::{Event, PlayerId, RespawnCause};
use game_shared::trade::TradeEnd;
use game_shared::world::TileExtra;
use game_shared::worldgen::generate_seeded;

use crate::store::{Returned, encode_world};
use crate::world::{Out, Room};

/// A prepared reset: who asked, with which seed, until when (server time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending {
    by: PlayerId,
    seed: Option<u64>,
    until: f64,
}

/// What a reset wipes, counted from the live world.
struct Counts {
    claims: usize,
    owners: usize,
    trees: usize,
    drops: usize,
}

const NOTHING_TO_CONFIRM: &str = "Nothing to confirm. Run /resetworld again";

/// "small claim" -> "Small Claim".
fn title(item_id: ItemId) -> String {
    let name = item(item_id).map_or("claim", |def| def.name);
    name.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or(String::new(), |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Room {
    fn counts(&self) -> Counts {
        let world = &self.state.world;
        let mut owners: Vec<PlayerId> = Vec::new();
        let (mut claims, mut trees) = (0, 0);
        for extra in world.extras.values() {
            match extra {
                TileExtra::Claim(claim) => {
                    claims += 1;
                    if !owners.contains(&claim.owner) {
                        owners.push(claim.owner);
                    }
                }
                TileExtra::Tree(_) => trees += 1,
            }
        }
        Counts {
            claims,
            owners: owners.len(),
            trees,
            drops: world.drops.len(),
        }
    }

    /// One `/resetworld` step. `Ok`/`Err` carry the caller's reply, like `Room::run`.
    pub(crate) fn reset_command(
        &mut self,
        id: PlayerId,
        step: Reset,
        now: f64,
    ) -> Result<String, String> {
        let name = self.state.world.name.clone();
        match step {
            Reset::Prepare { world, seed } => {
                // ponytail: only the caller's own world; another world needs its task loaded and a
                // cross-world message. Add with world select if the owner asks for it.
                if let Some(other) = world.filter(|other| *other != name) {
                    return Err(format!(
                        "Join {other} first: /resetworld resets the world you are in"
                    ));
                }
                self.reset_pending = Some(Pending {
                    by: id,
                    seed,
                    until: now + RESET_CONFIRM_SECS,
                });
                let c = self.counts();
                Ok(format!(
                    "Reset {name}? This wipes {} claims (owned by {} players), {} trees, {} drops and \
                     every placed block. {} players are inside. Type /resetworld confirm within {}s.",
                    c.claims,
                    c.owners,
                    c.trees,
                    c.drops,
                    self.names.len(),
                    RESET_CONFIRM_SECS as u32,
                ))
            }
            Reset::Cancel => match self.reset_pending.take() {
                Some(pending) if pending.by == id => Ok(format!("Reset of {name} cancelled")),
                other => {
                    self.reset_pending = other;
                    Err("Nothing to cancel".into())
                }
            },
            Reset::Confirm => match self.reset_pending.take() {
                Some(pending) if pending.by == id && now <= pending.until => {
                    Ok(self.reset_world(pending.seed))
                }
                // Someone else's pending reset stays; a late one is dropped.
                other => {
                    self.reset_pending = other.filter(|p| p.by != id && now <= p.until);
                    Err(NOTHING_TO_CONFIRM.into())
                }
            },
        }
    }

    /// Carries out a confirmed reset and returns the caller's reply.
    fn reset_world(&mut self, seed: Option<u64>) -> String {
        let name = self.state.world.name.clone();
        let c = self.counts();
        self.backup = Some(encode_world(&self.state));

        // Open trades end; nothing moves.
        let ended: Vec<Event> = self
            .state
            .trades
            .drain(..)
            .map(|trade| Event::TradeCancelled {
                trade: trade.id,
                reason: TradeEnd::Invalid,
            })
            .collect();
        self.broadcast(&ended, None);

        // Claims go home.
        let claims: Vec<(ItemId, PlayerId)> = self
            .state
            .world
            .extras
            .iter()
            .filter_map(|(&key, extra)| match extra {
                TileExtra::Claim(claim) => {
                    let (x, y) = (
                        key % game_shared::WORLD_WIDTH,
                        key / game_shared::WORLD_WIDTH,
                    );
                    Some((self.state.world.get(x, y)?.fg, claim.owner))
                }
                TileExtra::Tree(_) => None,
            })
            .collect();
        for (claim_item, owner) in claims {
            let returned = Returned {
                player: owner,
                item: claim_item,
                price: claim_kind(claim_item).map_or(0, |kind| kind.price),
                notice: format!(
                    "Your {} in {name} was returned because the owner reset the world.",
                    title(claim_item)
                ),
            };
            if self.state.players.contains_key(&owner) {
                let events = self.pay(owner, &returned);
                self.broadcast(&events, None);
                self.system(owner, returned.notice);
            } else {
                self.returned.push(returned);
            }
        }

        // New terrain: no trees, drops, claims or placed blocks.
        self.state.world = generate_seeded(&name, seed);
        self.state.damage.clear();
        let door = spawn_body(&self.state.world);
        let mut moved = Vec::new();
        for player in self.state.players.values_mut() {
            moved.push(Event::Respawned {
                player: player.id,
                cause: RespawnCause::Home,
                from: player.body.pos,
            });
            player.body = door;
        }
        // Each member gets the new world whole (the client swaps it in and shows the toast),
        // then everyone sees everyone appear at the door.
        let members: Vec<PlayerId> = self.names.keys().copied().collect();
        for member in members {
            if let Some(snapshot) = self.snapshot_for(member) {
                let snapshot = Box::new(snapshot);
                self.send(member, Out::Reliable(ServerMsg::Joined { snapshot }));
            }
        }
        self.broadcast(&moved, None);
        self.save_due = true;
        self.touched = true;
        // The audit line has the seed in the command itself (none given: the world's own).
        format!(
            "{name} was reset ({} claims returned, {} trees, {} drops removed).",
            c.claims, c.trees, c.drops
        )
    }

    /// Puts a returned claim in `owner`'s backpack, or its price in gems when there is no room.
    fn pay(&mut self, owner: PlayerId, returned: &Returned) -> Vec<Event> {
        let Some(player) = self.state.players.get_mut(&owner) else {
            return Vec::new();
        };
        if player.inventory.add(returned.item, 1) == 1 {
            vec![Event::InventoryChanged {
                player: owner,
                item: returned.item,
                count: player.inventory.count(returned.item),
            }]
        } else {
            player.gems = player.gems.saturating_add(returned.price);
            vec![Event::GemsChanged {
                player: owner,
                gems: player.gems,
            }]
        }
    }
}
