//! World Deeds (docs/knowledge/ux/world-key.md): a paper that hands over a world's World Claim.
//! Inventory slots carry no per-item data, so deeds live on a per-player ring (`Player::deeds`).
//! The server's `world_deeds` table is the truth; the ring mirrors it.

use crate::worldgen::MAX_WORLD_NAME_LEN;

/// A world name in a fixed buffer, so actions that carry one stay `Copy`. Upper-case ASCII
/// letters and digits, like `worldgen::validate_world_name`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WorldName {
    bytes: [u8; MAX_WORLD_NAME_LEN],
    len: u8,
}

impl WorldName {
    /// `None` unless `name` is a valid world name; stored upper-case.
    pub fn new(name: &str) -> Option<Self> {
        if !crate::worldgen::validate_world_name(name) {
            return None;
        }
        let mut bytes = [0; MAX_WORLD_NAME_LEN];
        for (slot, byte) in bytes.iter_mut().zip(name.bytes()) {
            *slot = byte.to_ascii_uppercase();
        }
        Some(Self {
            bytes,
            len: u8::try_from(name.len()).ok()?,
        })
    }

    pub fn as_str(&self) -> &str {
        // Only ASCII is ever stored (`new`); a corrupt buffer reads as empty rather than panicking.
        std::str::from_utf8(&self.bytes[..usize::from(self.len).min(MAX_WORLD_NAME_LEN)])
            .unwrap_or("")
    }
}

impl std::fmt::Debug for WorldName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WorldName({})", self.as_str())
    }
}

impl std::fmt::Display for WorldName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One deed on a ring. A void deed's world claim is gone: it can only be discarded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Deed {
    pub world: WorldName,
    pub void: bool,
}

/// Deeds one player may hold at once. Tuning; keeps a ring (and a trade row) readable.
pub const MAX_DEEDS: usize = 16;

use crate::rules::{Event, PlayerId, Reject, WorldState};
use crate::world::World;

fn ring_changed(state: &WorldState, player: PlayerId) -> Event {
    let deeds = state
        .players
        .get(&player)
        .map(|p| p.deeds.clone())
        .unwrap_or_default();
    Event::DeedsChanged { player, deeds }
}

/// Whether `player` offers a deed for `world` in their open trade: it cannot be used or torn up then.
fn offered(state: &WorldState, player: PlayerId, world: WorldName) -> bool {
    state.trades.iter().any(|trade| {
        trade
            .side(player)
            .and_then(|side| trade.deeds.get(side))
            .is_some_and(|deeds| deeds.contains(&world))
    })
}

/// The index on `player`'s ring of the deed for `world` that is (or is not) void.
fn find(state: &WorldState, player: PlayerId, world: WorldName, void: bool) -> Option<usize> {
    let ring = &state.players.get(&player)?.deeds;
    ring.iter()
        .position(|deed| deed.world == world && deed.void == void)
}

/// Owner only, on the world claim block: a deed for this world onto the owner's ring.
pub(crate) fn make(
    state: &mut WorldState,
    player: PlayerId,
    x: u32,
    y: u32,
) -> Result<Vec<Event>, Reject> {
    let key = World::tile_key(x, y).ok_or(Reject::Invalid)?;
    if state.world.world_claim != Some(key) {
        return Err(Reject::Invalid);
    }
    let claim = state.world.claim(key).ok_or(Reject::Invalid)?;
    if claim.owner != player {
        return Err(Reject::NotOwner);
    }
    if state.deed_out {
        return Err(Reject::DeedOut);
    }
    let world = WorldName::new(&state.world.name).ok_or(Reject::Invalid)?;
    let ring = &mut state.players.get_mut(&player).ok_or(Reject::Invalid)?.deeds;
    if ring.len() >= MAX_DEEDS {
        return Err(Reject::InventoryFull);
    }
    ring.push(Deed { world, void: false });
    state.deed_out = true;
    Ok(vec![
        ring_changed(state, player),
        Event::DeedOut { out: true },
    ])
}

/// The ring side of using a deed: it leaves the ring now (so it cannot be traded or used twice
/// while the server changes the other world). `restore` puts it back if that fails.
pub(crate) fn begin_use(
    state: &mut WorldState,
    player: PlayerId,
    world: WorldName,
) -> Result<Vec<Event>, Reject> {
    if find(state, player, world, true).is_some() && find(state, player, world, false).is_none() {
        return Err(Reject::DeedVoid);
    }
    let index = find(state, player, world, false).ok_or(Reject::NoItem)?;
    if offered(state, player, world) {
        return Err(Reject::Invalid);
    }
    let ring = &mut state.players.get_mut(&player).ok_or(Reject::Invalid)?.deeds;
    ring.remove(index);
    Ok(vec![ring_changed(state, player)])
}

/// A deed whose use failed goes back on the ring.
pub fn restore(state: &mut WorldState, player: PlayerId, deed: Deed) -> Vec<Event> {
    let Some(p) = state.players.get_mut(&player) else {
        return Vec::new();
    };
    p.deeds.push(deed);
    vec![ring_changed(state, player)]
}

/// Throw a valid deed away; its world keeps its owner.
pub(crate) fn tear_up(
    state: &mut WorldState,
    player: PlayerId,
    world: WorldName,
) -> Result<Vec<Event>, Reject> {
    let index = find(state, player, world, false).ok_or(Reject::NoItem)?;
    if offered(state, player, world) {
        return Err(Reject::Invalid);
    }
    let ring = &mut state.players.get_mut(&player).ok_or(Reject::Invalid)?.deeds;
    ring.remove(index);
    let mut events = vec![ring_changed(state, player)];
    if world.as_str() == state.world.name {
        state.deed_out = false;
        events.push(Event::DeedOut { out: false });
    }
    Ok(events)
}

/// Throw a void deed away.
pub(crate) fn discard(
    state: &mut WorldState,
    player: PlayerId,
    world: WorldName,
) -> Result<Vec<Event>, Reject> {
    let index = find(state, player, world, true).ok_or(Reject::NoItem)?;
    let ring = &mut state.players.get_mut(&player).ok_or(Reject::Invalid)?.deeds;
    ring.remove(index);
    Ok(vec![ring_changed(state, player)])
}

/// A deed was used on this world: `new_owner` owns the world claim, its access list is cleared and
/// it is no longer public; no deed is out any more. `None` if the world has no world claim.
pub fn transfer(state: &mut WorldState, new_owner: PlayerId) -> Option<Vec<Event>> {
    let key = state.world.world_claim?;
    let mut claim = state.world.claim(key)?.clone();
    claim.owner = new_owner;
    claim.access.clear();
    claim.public = false;
    state.world.insert_claim(key, claim.clone());
    state.deed_out = false;
    let (x, y) = (key % crate::WORLD_WIDTH, key / crate::WORLD_WIDTH);
    Some(vec![
        Event::ClaimUpdated { x, y, claim },
        Event::DeedOut { out: false },
    ])
}

/// This world's deed is void (the world was reset or its world claim broke): holders here see it
/// greyed at once, holders elsewhere at their next join.
pub fn void_here(state: &mut WorldState) -> Vec<Event> {
    let Some(world) = WorldName::new(&state.world.name) else {
        return Vec::new();
    };
    state.deed_out = false;
    let mut events = vec![Event::DeedOut { out: false }];
    let holders: Vec<PlayerId> = state
        .players
        .values()
        .filter(|p| p.deeds.iter().any(|d| d.world == world && !d.void))
        .map(|p| p.id)
        .collect();
    for id in holders {
        if let Some(p) = state.players.get_mut(&id) {
            for deed in p.deeds.iter_mut().filter(|d| d.world == world) {
                deed.void = true;
            }
        }
        events.push(ring_changed(state, id));
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::Claim;
    use crate::rules::{Action, apply};

    const OWNER: PlayerId = PlayerId(1);
    const BUYER: PlayerId = PlayerId(2);

    /// FARM with a world claim owned by OWNER (block at the door's right), both players in.
    fn claimed() -> (WorldState, (u32, u32)) {
        let mut state = WorldState::new(crate::worldgen::generate("FARM"), 1);
        state.join(OWNER);
        state.join(BUYER);
        let at = (state.world.door.0 + 1, state.world.door.1);
        let key = World::tile_key(at.0, at.1).unwrap();
        let claim = Claim {
            owner: OWNER,
            access: vec![BUYER],
            public: true,
            tiles: vec![],
        };
        state.world.insert_claim(key, claim);
        (state, at)
    }

    fn farm() -> WorldName {
        WorldName::new("FARM").unwrap()
    }

    #[test]
    fn only_the_owner_makes_one_deed_per_world() {
        let (mut state, (x, y)) = claimed();
        assert_eq!(
            apply(&mut state, BUYER, Action::MakeDeed { x, y }, 0.0),
            Err(Reject::NotOwner)
        );
        let events = apply(&mut state, OWNER, Action::MakeDeed { x, y }, 0.0).unwrap();
        assert!(events.contains(&Event::DeedOut { out: true }));
        assert_eq!(
            state.players[&OWNER].deeds,
            [Deed {
                world: farm(),
                void: false
            }]
        );
        assert_eq!(
            apply(&mut state, OWNER, Action::MakeDeed { x, y }, 0.0),
            Err(Reject::DeedOut),
            "a second deed for the same world is refused"
        );
        // Tearing it up frees the world for a new deed.
        apply(&mut state, OWNER, Action::TearUpDeed { world: farm() }, 0.0).unwrap();
        assert!(!state.deed_out && state.players[&OWNER].deeds.is_empty());
        apply(&mut state, OWNER, Action::MakeDeed { x, y }, 0.0).unwrap();
    }

    #[test]
    fn a_used_deed_hands_over_the_world_clean() {
        let (mut state, _) = claimed();
        state.deed_out = true;
        let events = transfer(&mut state, BUYER).unwrap();
        let claim = state.world.claim(state.world.world_claim.unwrap()).unwrap();
        assert_eq!(claim.owner, BUYER);
        assert!(
            claim.access.is_empty() && !claim.public,
            "access list cleared, public off"
        );
        assert!(!state.deed_out && events.contains(&Event::DeedOut { out: false }));
    }

    #[test]
    fn a_void_deed_is_refused_and_can_only_be_discarded() {
        let (mut state, (x, y)) = claimed();
        apply(&mut state, OWNER, Action::MakeDeed { x, y }, 0.0).unwrap();
        void_here(&mut state);
        assert!(state.players[&OWNER].deeds[0].void);
        let world = farm();
        assert_eq!(
            apply(&mut state, OWNER, Action::UseDeed { world }, 0.0),
            Err(Reject::DeedVoid)
        );
        assert_eq!(
            apply(&mut state, OWNER, Action::TearUpDeed { world }, 0.0),
            Err(Reject::NoItem)
        );
        apply(&mut state, OWNER, Action::DiscardDeed { world }, 0.0).unwrap();
        assert!(state.players[&OWNER].deeds.is_empty());
    }

    #[test]
    fn using_takes_the_deed_off_the_ring_and_a_failure_puts_it_back() {
        let (mut state, (x, y)) = claimed();
        apply(&mut state, OWNER, Action::MakeDeed { x, y }, 0.0).unwrap();
        let world = farm();
        apply(&mut state, OWNER, Action::UseDeed { world }, 0.0).unwrap();
        assert!(
            state.players[&OWNER].deeds.is_empty(),
            "locked while in flight"
        );
        assert_eq!(
            apply(&mut state, OWNER, Action::UseDeed { world }, 0.0),
            Err(Reject::NoItem)
        );
        restore(&mut state, OWNER, Deed { world, void: false });
        assert_eq!(state.players[&OWNER].deeds.len(), 1);
    }

    #[test]
    fn world_names_are_checked_and_upper_cased() {
        assert_eq!(WorldName::new("myfarm").unwrap().as_str(), "MYFARM");
        assert_eq!(WorldName::new("MYFARM"), WorldName::new("myfarm"));
        assert!(WorldName::new("").is_none());
        assert!(WorldName::new("has space").is_none());
        assert!(WorldName::new(&"X".repeat(MAX_WORLD_NAME_LEN + 1)).is_none());
        assert_eq!(
            WorldName::new(&"X".repeat(MAX_WORLD_NAME_LEN))
                .unwrap()
                .as_str()
                .len(),
            MAX_WORLD_NAME_LEN
        );
    }

    #[test]
    fn deeds_change_hands_in_a_trade_and_an_offered_deed_is_locked() {
        let (mut state, _) = claimed();
        let (farm, mine) = (farm(), WorldName::new("MINE").unwrap());
        state.players.get_mut(&OWNER).unwrap().deeds.push(Deed {
            world: farm,
            void: false,
        });
        state.players.get_mut(&BUYER).unwrap().deeds.push(Deed {
            world: mine,
            void: false,
        });
        let act =
            |state: &mut WorldState, who, action, now| apply(state, who, action, now).unwrap();
        act(&mut state, OWNER, Action::TradeRequest { to: BUYER }, 0.0);
        act(
            &mut state,
            BUYER,
            Action::TradeRespond { accept: true },
            0.0,
        );
        act(
            &mut state,
            OWNER,
            Action::TradeDeed {
                world: farm,
                add: true,
            },
            0.0,
        );
        act(
            &mut state,
            BUYER,
            Action::TradeDeed {
                world: mine,
                add: true,
            },
            0.0,
        );
        // Offered: it cannot be used or torn up meanwhile (no "use it right before commit" scam).
        assert_eq!(
            apply(&mut state, OWNER, Action::UseDeed { world: farm }, 0.0),
            Err(Reject::Invalid)
        );
        assert_eq!(
            apply(&mut state, OWNER, Action::TearUpDeed { world: farm }, 0.0),
            Err(Reject::Invalid)
        );
        // Offering what you do not hold is refused.
        assert_eq!(
            apply(
                &mut state,
                OWNER,
                Action::TradeDeed {
                    world: mine,
                    add: true
                },
                0.0
            ),
            Err(Reject::NoItem)
        );
        act(&mut state, OWNER, Action::TradeLock { locked: true }, 1.0);
        act(&mut state, BUYER, Action::TradeLock { locked: true }, 1.0);
        act(&mut state, OWNER, Action::TradeConfirm, 5.0);
        let events = act(&mut state, BUYER, Action::TradeConfirm, 5.0);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::TradeCompleted { .. }))
        );
        assert_eq!(
            state.players[&OWNER].deeds,
            [Deed {
                world: mine,
                void: false
            }]
        );
        assert_eq!(
            state.players[&BUYER].deeds,
            [Deed {
                world: farm,
                void: false
            }]
        );
        let rings = events
            .iter()
            .filter(|e| matches!(e, Event::DeedsChanged { .. }))
            .count();
        assert_eq!(rings, 2, "both rings reported");
    }
}
