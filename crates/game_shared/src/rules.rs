//! The Action / Event contract (docs/knowledge/09). `apply` is the only function that changes a
//! `WorldState` in response to a player: it validates everything first, then writes, then reports
//! every change as an `Event`. A `Reject` means nothing changed.
//!
//! Documented exemptions from "events are complete": `Player::last_punch` and the expiry of
//! `Damage` entries change without an event.

use std::collections::HashMap;

use crate::claims::{Claim, MAX_CLAIM_ACCESS, can_edit, claim_area, claim_kind};
use crate::farming::{
    SAPLING_HITS, TREE_SEED_CHANCE, TREE_YIELD_MAX, TREE_YIELD_MIN, Tree, is_grown, splice,
};
use crate::inventory::{Inventory, MAX_STACK, MAX_UPGRADES, SLOTS_PER_UPGRADE, upgrade_cost};
use crate::items::{
    Collision, EMPTY, ItemDef, ItemId, ItemKind, MAIN_DOOR, PERMANENT, SLOT_COUNT, STARTER_OUTFIT,
    Slot, UNBREAKABLE, UNTRADEABLE, equip_price, item, punch_power, seed_of, seed_parent, slot_of,
};
use crate::levels::{XP_BREAK_MIN, XP_HARVEST_PER_RARITY, XP_PLANT, XP_SPLICE, add_xp};
use crate::physics::{Body, hitbox_contains, hitbox_overlaps_tile, spawn_body};
use crate::rng::Rng;
use crate::trade::{Trade, TradeEnd};
use crate::world::{Drop, Tile, TileExtra, World};
use crate::{tile_to_world, world_to_tile};

/// Tiles. Reference. Chebyshev distance from the player's tile; the own tile is reachable too.
pub const REACH: u32 = 2;
/// Seconds between punches from one player. Tuning.
pub const PUNCH_COOLDOWN: f64 = 0.2;
/// Seconds without a punch after which a tile's damage returns to 0. Tuning.
pub const DAMAGE_RESET: f64 = 5.0;
/// Chance that a broken block drops itself. Tuning.
pub const DROP_BLOCK: f32 = 0.40;
/// Chance that a broken block drops its seed instead. Tuning.
pub const DROP_SEED: f32 = 0.20;
/// At the limit a break or harvest loses its drop and a `Drop` action is rejected. Tuning.
pub const MAX_DROPS_PER_WORLD: usize = 2000;
/// A break or harvest gives uniform `0 ..= GEMS_BASE_MAX + rarity / divisor` gems. Tuning.
pub const GEMS_BASE_MAX: u32 = 1;
pub const GEMS_BREAK_RARITY_DIVISOR: u16 = 4;
pub const GEMS_HARVEST_RARITY_DIVISOR: u16 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlayerId(pub u32);

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Player {
    pub id: PlayerId,
    pub body: Body,
    pub inventory: Inventory,
    /// Soft currency. A number, not an item: it cannot be dropped, traded or given.
    pub gems: u32,
    /// `now` of the last accepted punch.
    pub last_punch: f64,
    /// What the player wears and holds, indexed by `Slot as usize`. A worn item is also in the
    /// inventory, and leaves this array when its last copy leaves the inventory.
    pub equipped: [Option<ItemId>; SLOT_COUNT],
    /// `1..=MAX_LEVEL` (docs/knowledge/14). Public: it colours the name.
    pub level: u16,
    /// Progress within the current level, `0..xp_to_next(level)`. Private.
    pub xp: u32,
}

impl Player {
    /// The item worn in `slot`, if any.
    pub fn worn(&self, slot: Slot) -> Option<ItemId> {
        self.equipped.get(slot as usize).copied().flatten()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Layer {
    Foreground,
    Background,
}

/// Break progress of one tile, shared by all players. Not persisted; expiry is checked lazily on the next punch.
#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Damage {
    pub hits: u8,
    pub layer: Layer,
    pub last_hit: f64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct WorldState {
    pub world: World,
    pub players: HashMap<PlayerId, Player>,
    /// By tile key (`World::tile_key`).
    pub damage: HashMap<u32, Damage>,
    pub rng: Rng,
    /// Open trade requests and trades; a player is in at most one (docs/knowledge/08).
    pub trades: Vec<Trade>,
    /// The id the next trade request gets.
    pub next_trade_id: u32,
}

impl WorldState {
    pub fn new(world: World, seed: u64) -> Self {
        Self {
            world,
            players: HashMap::new(),
            damage: HashMap::new(),
            rng: Rng::new(seed),
            trades: Vec::new(),
            next_trade_id: 0,
        }
    }

    /// Adds a player standing on the main door with a starting inventory, owning and wearing the
    /// starter outfit. No effect if already present.
    pub fn join(&mut self, id: PlayerId) {
        let body = spawn_body(&self.world);
        self.players.entry(id).or_insert_with(|| {
            let mut inventory = Inventory::default();
            let mut equipped = [None; SLOT_COUNT];
            for outfit in STARTER_OUTFIT {
                let slot = slot_of(outfit).and_then(|slot| equipped.get_mut(slot as usize));
                if let Some(slot) = slot.filter(|_| inventory.add(outfit, 1) == 1) {
                    *slot = Some(outfit);
                }
            }
            Player {
                id,
                body,
                inventory,
                gems: 0,
                last_punch: f64::NEG_INFINITY,
                equipped,
                level: 1,
                xp: 0,
            }
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Action {
    Punch {
        x: u32,
        y: u32,
    },
    /// Blocks, backgrounds and claims are placed; a seed is planted, or spliced into the sapling on the tile.
    Place {
        x: u32,
        y: u32,
        item: ItemId,
    },
    /// Onto the tile next to the player's tile, on the named side.
    Drop {
        item: ItemId,
        count: u16,
        left: bool,
    },
    Trash {
        item: ItemId,
        count: u16,
    },
    /// From the store's fixed price list.
    Buy {
        item: ItemId,
    },
    BuyBackpackUpgrade,
    /// Wear or hold an item from the inventory in its slot, replacing what was there (it stays in
    /// the inventory).
    Equip {
        item: ItemId,
    },
    /// Empty a slot. The item stays in the inventory.
    Unequip {
        slot: Slot,
    },
    /// Owner only: put `player` on, or take them off, the access list of the claim block at `(x, y)`.
    SetClaimAccess {
        x: u32,
        y: u32,
        player: PlayerId,
        allow: bool,
    },
    /// Owner only: let everyone, or only the owner and the access list, edit the claim's tiles.
    SetClaimPublic {
        x: u32,
        y: u32,
        public: bool,
    },
    /// Ask `to` for a trade (docs/knowledge/08, "Trade flow").
    TradeRequest {
        to: PlayerId,
    },
    /// By the asked player, on their pending request.
    TradeRespond {
        accept: bool,
    },
    /// Put `count` of `item` in slot `slot` of the own offer; count 0 clears the slot.
    TradeSetOffer {
        slot: u8,
        item: ItemId,
        count: u16,
    },
    TradeLock {
        locked: bool,
    },
    /// In review, after `TRADE_REVIEW_DELAY`.
    TradeConfirm,
    TradeCancel,
}

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Event {
    TileDamaged {
        x: u32,
        y: u32,
        layer: Layer,
        damage: u8,
    },
    TileChanged {
        x: u32,
        y: u32,
        tile: Tile,
    },
    /// A new drop, or an existing one (same id) whose count grew by a merge.
    ItemDropped {
        drop: Drop,
    },
    /// `count` items left the drop; it is gone when none remain.
    DropTaken {
        drop_id: u32,
        by: PlayerId,
        count: u16,
    },
    /// `count` is the new total.
    InventoryChanged {
        player: PlayerId,
        item: ItemId,
        count: u16,
    },
    /// `gems` is the new total.
    GemsChanged {
        player: PlayerId,
        gems: u32,
    },
    /// `capacity` is the new slot count.
    BackpackUpgraded {
        player: PlayerId,
        capacity: u16,
    },
    TreePlanted {
        x: u32,
        y: u32,
        tree: Tree,
    },
    /// The sapling of `a` took a seed of `b` and is now `tree`.
    TreeSpliced {
        x: u32,
        y: u32,
        tree: Tree,
        a: ItemId,
        b: ItemId,
    },
    /// Followed by the `ItemDropped` and `GemsChanged` events of the harvest.
    TreeHarvested {
        x: u32,
        y: u32,
    },
    ClaimPlaced {
        x: u32,
        y: u32,
        claim: Claim,
    },
    /// The claim block broke; its tiles are free again.
    ClaimRemoved {
        x: u32,
        y: u32,
    },
    /// Access list or public flag changed.
    ClaimUpdated {
        x: u32,
        y: u32,
        claim: Claim,
    },
    /// The player punched the main door.
    LeftWorld {
        player: PlayerId,
    },
    /// `item` is now in `slot` (`None`: the slot is empty). Public: everyone in the world sees it.
    Equipped {
        player: PlayerId,
        slot: Slot,
        item: Option<ItemId>,
    },
    /// Private to the two players, like every trade event.
    TradeRequested {
        trade: u32,
        from: PlayerId,
        to: PlayerId,
    },
    /// The full trade after any change: accepted, offer, lock, review, one confirm.
    TradeUpdated {
        trade: Trade,
    },
    /// Followed by `InventoryChanged` per changed item (and `Equipped` for a worn last copy).
    TradeCompleted {
        trade: u32,
    },
    TradeCancelled {
        trade: u32,
        reason: TradeEnd,
    },
    /// Private: the player's level and XP after an action that gained XP.
    XpChanged {
        player: PlayerId,
        level: u16,
        xp: u32,
    },
    /// Public: the player reached `level` (the final one, once per action).
    LevelUp {
        player: PlayerId,
        level: u16,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Reject {
    OutOfRange,
    /// A claim protects the tile and does not admit this player.
    NoPermission,
    /// Only the claim's owner may break it or change who it admits.
    NotOwner,
    Occupied,
    NoItem,
    Unbreakable,
    Cooldown,
    /// The two seeds are not a recipe. Nothing is consumed.
    NoRecipe,
    /// The tree was spliced before. Nothing is consumed.
    AlreadySpliced,
    /// A seed needs a solid block directly below.
    NeedsGround,
    /// The world holds `MAX_DROPS_PER_WORLD` drops and this one would not merge.
    DropLimit,
    NotEnoughGems,
    InventoryFull,
    /// One world claim per world.
    WorldAlreadyClaimed,
    /// Someone else owns an area claim here, so the world cannot be claimed.
    ForeignClaims,
    Invalid,
}

/// Whether tile `(x, y)` is within punch and build range of `body`. The one range rule; the client
/// uses it for the hover tint.
pub fn in_reach(body: &Body, x: u32, y: u32) -> bool {
    world_to_tile(body.pos.0, body.pos.1)
        .is_some_and(|(px, py)| px.abs_diff(x).max(py.abs_diff(y)) <= REACH)
}

/// Rejects with nothing changed, or applies the action and returns every change it made.
pub fn apply(
    state: &mut WorldState,
    player: PlayerId,
    action: Action,
    now: f64,
) -> Result<Vec<Event>, Reject> {
    match action {
        Action::Punch { x, y } => punch(state, player, x, y, now),
        Action::Place { x, y, item } => place(state, player, x, y, item, now),
        Action::Drop { item, count, left } => discard(state, player, item, count, Some(left)),
        Action::Trash { item, count } => discard(state, player, item, count, None),
        Action::Buy { item } => buy(state, player, item),
        Action::BuyBackpackUpgrade => buy_backpack_upgrade(state, player),
        Action::Equip { item } => equip(state, player, item),
        Action::Unequip { slot } => unequip(state, player, slot),
        Action::SetClaimAccess {
            x,
            y,
            player: other,
            allow,
        } => update_claim(state, player, x, y, |claim| {
            let listed = claim.access.contains(&other);
            if allow
                && (other == claim.owner || (!listed && claim.access.len() >= MAX_CLAIM_ACCESS))
            {
                return Err(Reject::Invalid);
            }
            if allow && !listed {
                claim.access.push(other);
            } else if !allow {
                claim.access.retain(|&listed| listed != other);
            }
            Ok(())
        }),
        Action::SetClaimPublic { x, y, public } => update_claim(state, player, x, y, |claim| {
            claim.public = public;
            Ok(())
        }),
        Action::TradeRequest { to } => crate::trade::request(state, player, to),
        Action::TradeRespond { accept } => crate::trade::respond(state, player, accept),
        Action::TradeSetOffer { slot, item, count } => {
            crate::trade::set_offer(state, player, slot, item, count)
        }
        Action::TradeLock { locked } => crate::trade::lock(state, player, locked, now),
        Action::TradeConfirm => crate::trade::confirm(state, player, now),
        Action::TradeCancel => crate::trade::cancel(state, player),
    }
}

pub use crate::trade::end_trades_of;

/// Automatic pickup: moves every drop whose point is inside the player's hitbox into the
/// inventory, as far as it fits. Not an `Action` (nobody asks for it); call it after each physics
/// tick. Drops are not protected by claims.
pub fn collect_drops(state: &mut WorldState, id: PlayerId) -> Vec<Event> {
    let mut events = Vec::new();
    let Some(player) = state.players.get_mut(&id) else {
        return events;
    };
    // ponytail: scans every drop each tick (at most MAX_DROPS_PER_WORLD); bucket drops by tile if a profile shows it.
    state.world.drops.retain_mut(|drop| {
        if hitbox_contains(player.body.pos, drop_position(drop)) {
            let taken = player.inventory.add(drop.item, drop.count);
            if taken > 0 {
                drop.count -= taken;
                events.push(Event::DropTaken {
                    drop_id: drop.id,
                    by: id,
                    count: taken,
                });
                events.push(Event::InventoryChanged {
                    player: id,
                    item: drop.item,
                    count: player.inventory.count(drop.item),
                });
            }
        }
        drop.count > 0
    });
    events
}

/// A drop is a point at the centre of its tile, world px.
pub fn drop_position(drop: &Drop) -> (f32, f32) {
    tile_to_world(drop.x, drop.y)
}

/// The tile at `(x, y)` and its key, if it exists and `id` can reach it.
fn reachable_tile(state: &WorldState, id: PlayerId, x: u32, y: u32) -> Result<(Tile, u32), Reject> {
    let player = state.players.get(&id).ok_or(Reject::Invalid)?;
    let tile = state
        .world
        .get(x, y)
        .zip(World::tile_key(x, y))
        .ok_or(Reject::OutOfRange)?;
    if in_reach(&player.body, x, y) {
        Ok(tile)
    } else {
        Err(Reject::OutOfRange)
    }
}

fn punch(
    state: &mut WorldState,
    id: PlayerId,
    x: u32,
    y: u32,
    now: f64,
) -> Result<Vec<Event>, Reject> {
    let (tile, key) = reachable_tile(state, id, x, y)?;
    let player = state.players.get(&id).ok_or(Reject::Invalid)?;
    if now - player.last_punch < PUNCH_COOLDOWN {
        return Err(Reject::Cooldown);
    }

    // Foreground first: the block must go before the wall behind it.
    let (layer, target) = if tile.fg != EMPTY {
        (Layer::Foreground, tile.fg)
    } else {
        (Layer::Background, tile.bg)
    };
    // The main door is usable by everyone, even in a claimed world.
    if target == MAIN_DOOR {
        return Ok(vec![Event::LeftWorld { player: id }]);
    }
    // Permission before anything about the tile is revealed. A claim block answers to its owner alone.
    let claim_owner = match layer {
        Layer::Foreground => state.world.claim(key).map(|claim| claim.owner),
        Layer::Background => None,
    };
    if claim_owner != Some(id) {
        if !can_edit(&state.world, id, key) {
            return Err(Reject::NoPermission);
        }
        if claim_owner.is_some() {
            return Err(Reject::NotOwner);
        }
    }
    if target == EMPTY {
        // A miss still uses up the swing.
        set_last_punch(state, id, now);
        return Ok(Vec::new());
    }
    let def = item(target).ok_or(Reject::Invalid)?;
    if def.has(UNBREAKABLE) {
        return Err(Reject::Unbreakable);
    }
    let tree = match layer {
        Layer::Foreground => state.world.tree(x, y).copied(),
        Layer::Background => None,
    };
    let grown = tree.filter(|tree| is_grown(tree, now));
    let needed = if tree.is_some() {
        SAPLING_HITS
    } else {
        def.hits
    };
    let earlier = state.damage.get(&key);
    let fresh = earlier.filter(|d| d.layer == layer && now - d.last_hit <= DAMAGE_RESET);
    // The pickaxe in the hand decides how much one punch adds (docs/knowledge/03, "Punch power").
    let power = punch_power(player.worn(Slot::Hand));
    let hits = fresh.map_or(0, |d| d.hits).saturating_add(power);
    // A claim always drops itself, so its last punch waits until the world can hold the drop.
    if claim_owner.is_some() && hits >= needed && !can_drop(&state.world, target, 1, x, y) {
        return Err(Reject::DropLimit);
    }

    // Everything is validated; from here on the punch succeeds.
    set_last_punch(state, id, now);
    if let Some(tree) = grown {
        return Ok(harvest(state, id, x, y, tile, tree));
    }
    if hits < needed {
        state.damage.insert(
            key,
            Damage {
                hits,
                layer,
                last_hit: now,
            },
        );
        return Ok(vec![Event::TileDamaged {
            x,
            y,
            layer,
            damage: hits,
        }]);
    }

    let broken = match layer {
        Layer::Foreground => clear_foreground(state, x, y, tile),
        Layer::Background => {
            let broken = Tile { bg: EMPTY, ..tile };
            state.world.set(x, y, broken);
            state.damage.remove(&key);
            broken
        }
    };
    let mut events = vec![Event::TileChanged { x, y, tile: broken }];
    if claim_owner.is_some() {
        // No roll and no gems: otherwise placing and breaking a claim would mint gems.
        events.push(Event::ClaimRemoved { x, y });
        events.extend(
            put_drop(&mut state.world, target, 1, x, y).map(|drop| Event::ItemDropped { drop }),
        );
    } else if tree.is_none() {
        // A destroyed sapling gives nothing; a block gives its one drop roll and gems.
        if let Some(dropped) = roll_drop(&mut state.rng, def) {
            events.extend(
                put_drop(&mut state.world, dropped, 1, x, y)
                    .map(|drop| Event::ItemDropped { drop }),
            );
        }
        events.extend(award_gems(state, id, def.rarity, GEMS_BREAK_RARITY_DIVISOR));
        events.extend(award_xp(state, id, u32::from(def.rarity).max(XP_BREAK_MIN)));
    }
    Ok(events)
}

/// Empties the foreground of `(x, y)` together with everything that hangs on it (tree or claim,
/// damage). Every foreground removal goes through here, so no extra outlives its tile.
fn clear_foreground(state: &mut WorldState, x: u32, y: u32, tile: Tile) -> Tile {
    let cleared = Tile { fg: EMPTY, ..tile };
    state.world.set(x, y, cleared);
    if let Some(key) = World::tile_key(x, y) {
        state.world.remove_extra(key);
        state.damage.remove(&key);
    }
    cleared
}

/// One punch on a grown tree: the stored fruit, maybe a seed, gems; the tree is gone.
fn harvest(
    state: &mut WorldState,
    id: PlayerId,
    x: u32,
    y: u32,
    tile: Tile,
    tree: Tree,
) -> Vec<Event> {
    let cleared = clear_foreground(state, x, y, tile);
    let mut events = vec![
        Event::TileChanged {
            x,
            y,
            tile: cleared,
        },
        Event::TreeHarvested { x, y },
    ];
    let seed = seed_of(tree.item).filter(|_| state.rng.chance(TREE_SEED_CHANCE));
    for (dropped, count) in [(Some(tree.item), u16::from(tree.fruit)), (seed, 1)] {
        let drop = dropped
            .filter(|_| count > 0)
            .and_then(|dropped| put_drop(&mut state.world, dropped, count, x, y));
        events.extend(drop.map(|drop| Event::ItemDropped { drop }));
    }
    let rarity = item(tree.item).map_or(0, |def| def.rarity);
    events.extend(award_gems(state, id, rarity, GEMS_HARVEST_RARITY_DIVISOR));
    events.extend(award_xp(
        state,
        id,
        XP_HARVEST_PER_RARITY.saturating_mul(u32::from(rarity)),
    ));
    events
}

fn set_last_punch(state: &mut WorldState, id: PlayerId, now: f64) {
    if let Some(player) = state.players.get_mut(&id) {
        player.last_punch = now;
    }
}

/// One roll, exactly one outcome: the block, its seed, or nothing. A block is never duplicated.
fn roll_drop(rng: &mut Rng, broken: &ItemDef) -> Option<ItemId> {
    let roll = rng.next_f32();
    if roll < DROP_BLOCK {
        Some(broken.id)
    } else if roll < DROP_BLOCK + DROP_SEED {
        // `NO_SEED` items turn the seed outcome into the block outcome.
        Some(seed_of(broken.id).unwrap_or(broken.id))
    } else {
        None
    }
}

/// Uniform `0 ..= GEMS_BASE_MAX + rarity / divisor`.
fn roll_gems(rng: &mut Rng, rarity: u16, divisor: u16) -> u32 {
    rng.range(0, GEMS_BASE_MAX + u32::from(rarity / divisor.max(1)))
}

/// Credits the gems straight to the player. No event when the roll is zero.
fn award_gems(state: &mut WorldState, id: PlayerId, rarity: u16, divisor: u16) -> Option<Event> {
    let amount = roll_gems(&mut state.rng, rarity, divisor);
    let player = state.players.get_mut(&id).filter(|_| amount > 0)?;
    let before = player.gems;
    player.gems = before.saturating_add(amount);
    (player.gems != before).then_some(Event::GemsChanged {
        player: id,
        gems: player.gems,
    })
}

/// Credits XP to the player: one `XpChanged` with the final values and, if any level was gained,
/// one `LevelUp` with the final level. Nothing when nothing changed (e.g. at the cap).
fn award_xp(state: &mut WorldState, id: PlayerId, gained: u32) -> Vec<Event> {
    let Some(player) = state.players.get_mut(&id) else {
        return Vec::new();
    };
    let before = (player.level, player.xp);
    let levels = add_xp(&mut player.level, &mut player.xp, gained);
    let mut events = Vec::new();
    if (player.level, player.xp) != before {
        events.push(Event::XpChanged {
            player: id,
            level: player.level,
            xp: player.xp,
        });
    }
    if levels > 0 {
        events.push(Event::LevelUp {
            player: id,
            level: player.level,
        });
    }
    events
}

fn place(
    state: &mut WorldState,
    id: PlayerId,
    x: u32,
    y: u32,
    placed: ItemId,
    now: f64,
) -> Result<Vec<Event>, Reject> {
    let (tile, key) = reachable_tile(state, id, x, y)?;
    if !can_edit(&state.world, id, key) {
        return Err(Reject::NoPermission);
    }
    let def = item(placed).ok_or(Reject::Invalid)?;
    let new_tile = match def.kind {
        ItemKind::Seed => return plant(state, id, x, y, tile, placed, now),
        ItemKind::Block | ItemKind::Claim if tile.fg == EMPTY => Tile { fg: placed, ..tile },
        ItemKind::Background if tile.bg == EMPTY => Tile { bg: placed, ..tile },
        ItemKind::Block | ItemKind::Claim | ItemKind::Background => return Err(Reject::Occupied),
        ItemKind::Tool | ItemKind::Equip(_) | ItemKind::Resource => return Err(Reject::Invalid),
    };
    let overlaps_a_player = || {
        let mut bodies = state.players.values().map(|player| player.body.pos);
        bodies.any(|pos| hitbox_overlaps_tile(pos, x, y))
    };
    if (x, y) == state.world.door || (def.collision == Collision::Solid && overlaps_a_player()) {
        return Err(Reject::Occupied);
    }
    let claim = match def.kind {
        ItemKind::Claim => Some(new_claim(&state.world, id, x, y, placed)?),
        _ => None,
    };

    let count = take_one(state, id, placed)?;
    state.world.set(x, y, new_tile);
    let mut events = vec![Event::TileChanged {
        x,
        y,
        tile: new_tile,
    }];
    if let Some(claim) = claim {
        state.world.insert_claim(key, claim.clone());
        events.push(Event::ClaimPlaced { x, y, claim });
    }
    events.push(Event::InventoryChanged {
        player: id,
        item: placed,
        count,
    });
    Ok(events)
}

/// The claim that placing `placed` at `(x, y)` would create. Changes nothing.
fn new_claim(
    world: &World,
    owner: PlayerId,
    x: u32,
    y: u32,
    placed: ItemId,
) -> Result<Claim, Reject> {
    let kind = claim_kind(placed).ok_or(Reject::Invalid)?;
    let tiles = match kind.tiles {
        Some(count) => {
            let tiles = claim_area(world, x, y, count);
            // Every tile already belongs to an area claim: there is nothing left to own.
            if tiles.is_empty() {
                return Err(Reject::Occupied);
            }
            tiles
        }
        None => {
            if world.world_claim.is_some() {
                return Err(Reject::WorldAlreadyClaimed);
            }
            let foreign = |extra: &TileExtra| matches!(extra, TileExtra::Claim(claim) if claim.owner != owner && !claim.tiles.is_empty());
            if world.extras.values().any(foreign) {
                return Err(Reject::ForeignClaims);
            }
            Vec::new()
        }
    };
    Ok(Claim {
        owner,
        access: Vec::new(),
        public: false,
        tiles,
    })
}

/// Removes one `taken` from the player's inventory and returns the new total. `remove` is all or
/// nothing, so calling this as the first write keeps a failure free of side effects.
fn take_one(state: &mut WorldState, id: PlayerId, taken: ItemId) -> Result<u16, Reject> {
    let inventory = &mut state.players.get_mut(&id).ok_or(Reject::Invalid)?.inventory;
    if inventory.remove(taken, 1) {
        Ok(inventory.count(taken))
    } else {
        Err(Reject::NoItem)
    }
}

/// A seed on an empty tile above solid ground is planted; on an unspliced sapling of another item
/// it is spliced. The caller has checked reach and permission.
fn plant(
    state: &mut WorldState,
    id: PlayerId,
    x: u32,
    y: u32,
    tile: Tile,
    seed: ItemId,
    now: f64,
) -> Result<Vec<Event>, Reject> {
    let grows = seed_parent(seed).ok_or(Reject::Invalid)?;
    let key = World::tile_key(x, y).ok_or(Reject::OutOfRange)?;
    let player = state.players.get(&id).ok_or(Reject::Invalid)?;
    if player.inventory.count(seed) == 0 {
        return Err(Reject::NoItem);
    }

    if let Some(&sapling) = state.world.tree(x, y) {
        if sapling.spliced {
            return Err(Reject::AlreadySpliced);
        }
        // The same seed on its own sapling is not a recipe either.
        let result = splice(sapling.item, grows).ok_or(Reject::NoRecipe)?;
        let result_seed = seed_of(result).ok_or(Reject::NoRecipe)?;

        let count = take_one(state, id, seed)?;
        let tree = Tree {
            item: result,
            planted_at: now,
            fruit: sapling.fruit,
            spliced: true,
        };
        let new_tile = Tile {
            fg: result_seed,
            ..tile
        };
        state.world.set(x, y, new_tile);
        state.world.extras.insert(key, TileExtra::Tree(tree));
        state.damage.remove(&key); // a new sapling starts undamaged
        let mut events = vec![
            Event::TileChanged {
                x,
                y,
                tile: new_tile,
            },
            Event::TreeSpliced {
                x,
                y,
                tree,
                a: sapling.item,
                b: grows,
            },
            Event::InventoryChanged {
                player: id,
                item: seed,
                count,
            },
        ];
        events.extend(award_xp(state, id, XP_SPLICE));
        return Ok(events);
    }

    if tile.fg != EMPTY {
        return Err(Reject::Occupied);
    }
    if !y
        .checked_sub(1)
        .is_some_and(|below| is_solid(&state.world, x, below))
    {
        return Err(Reject::NeedsGround);
    }

    let count = take_one(state, id, seed)?;
    // Fruit is rolled now and stored, so nothing can re-roll it later.
    let fruit = state
        .rng
        .range(u32::from(TREE_YIELD_MIN), u32::from(TREE_YIELD_MAX));
    let tree = Tree {
        item: grows,
        planted_at: now,
        fruit: u8::try_from(fruit).unwrap_or(TREE_YIELD_MAX),
        spliced: false,
    };
    let new_tile = Tile { fg: seed, ..tile };
    state.world.set(x, y, new_tile);
    state.world.extras.insert(key, TileExtra::Tree(tree));
    let mut events = vec![
        Event::TileChanged {
            x,
            y,
            tile: new_tile,
        },
        Event::TreePlanted { x, y, tree },
        Event::InventoryChanged {
            player: id,
            item: seed,
            count,
        },
    ];
    events.extend(award_xp(state, id, XP_PLANT));
    Ok(events)
}

/// `Drop` (`side` given) puts the items on the tile beside the player; `Trash` destroys them.
fn discard(
    state: &mut WorldState,
    id: PlayerId,
    discarded: ItemId,
    count: u16,
    side: Option<bool>,
) -> Result<Vec<Event>, Reject> {
    let player = state.players.get(&id).ok_or(Reject::Invalid)?;
    let def = item(discarded).ok_or(Reject::Invalid)?;
    if !(1..=MAX_STACK).contains(&count) || def.has(UNTRADEABLE | PERMANENT) {
        return Err(Reject::Invalid);
    }
    if player.inventory.count(discarded) < count {
        return Err(Reject::NoItem);
    }
    let landing = match side {
        Some(left) => {
            let (px, y) =
                world_to_tile(player.body.pos.0, player.body.pos.1).ok_or(Reject::Invalid)?;
            let x = if left {
                px.checked_sub(1)
            } else {
                px.checked_add(1)
            };
            let x = x.filter(|&x| state.world.get(x, y).is_some() && !is_solid(&state.world, x, y));
            let x = x.ok_or(Reject::Occupied)?;
            if !can_drop(&state.world, discarded, count, x, y) {
                return Err(Reject::DropLimit);
            }
            Some((x, y))
        }
        None => None,
    };

    // `remove` is all or nothing, so a failure here is still before the first write.
    let inventory = &mut state.players.get_mut(&id).ok_or(Reject::Invalid)?.inventory;
    if !inventory.remove(discarded, count) {
        return Err(Reject::NoItem);
    }
    let player = state.players.get_mut(&id).ok_or(Reject::Invalid)?;
    let count_left = player.inventory.count(discarded);
    let mut events = vec![Event::InventoryChanged {
        player: id,
        item: discarded,
        count: count_left,
    }];
    // The last copy of a worn item takes it off in the same action.
    let worn = slot_of(discarded).filter(|&slot| player.worn(slot) == Some(discarded));
    if let Some(slot) = worn.filter(|_| count_left == 0) {
        if let Some(entry) = player.equipped.get_mut(slot as usize) {
            *entry = None;
        }
        events.push(Event::Equipped {
            player: id,
            slot,
            item: None,
        });
    }
    if let Some((x, y)) = landing {
        events.extend(
            put_drop(&mut state.world, discarded, count, x, y)
                .map(|drop| Event::ItemDropped { drop }),
        );
    }
    Ok(events)
}

/// The store: a fixed price list (`claims::CLAIMS`, then equipment by `items::equip_price`). Gems and
/// room are checked, then gems are taken and the item added. Buying equipment does not equip it.
fn buy(state: &mut WorldState, id: PlayerId, bought: ItemId) -> Result<Vec<Event>, Reject> {
    let player = state.players.get_mut(&id).ok_or(Reject::Invalid)?;
    let price = claim_kind(bought)
        .map(|kind| kind.price)
        .or_else(|| equip_price(bought))
        .ok_or(Reject::Invalid)?;
    let gems = player
        .gems
        .checked_sub(price)
        .ok_or(Reject::NotEnoughGems)?;
    if player.inventory.room_for(bought) == 0 {
        return Err(Reject::InventoryFull);
    }

    player.gems = gems;
    player.inventory.add(bought, 1);
    Ok(vec![
        Event::GemsChanged { player: id, gems },
        Event::InventoryChanged {
            player: id,
            item: bought,
            count: player.inventory.count(bought),
        },
    ])
}

/// Puts an inventory item into its slot. Equipping what is already worn changes nothing.
fn equip(state: &mut WorldState, id: PlayerId, worn: ItemId) -> Result<Vec<Event>, Reject> {
    let player = state.players.get_mut(&id).ok_or(Reject::Invalid)?;
    let slot = slot_of(worn).ok_or(Reject::Invalid)?;
    if player.inventory.count(worn) == 0 {
        return Err(Reject::NoItem);
    }
    let entry = player
        .equipped
        .get_mut(slot as usize)
        .ok_or(Reject::Invalid)?;
    if *entry == Some(worn) {
        return Ok(Vec::new());
    }
    *entry = Some(worn);
    Ok(vec![Event::Equipped {
        player: id,
        slot,
        item: Some(worn),
    }])
}

/// Empties a slot. An empty slot stays empty without an event.
fn unequip(state: &mut WorldState, id: PlayerId, slot: Slot) -> Result<Vec<Event>, Reject> {
    let player = state.players.get_mut(&id).ok_or(Reject::Invalid)?;
    let entry = player
        .equipped
        .get_mut(slot as usize)
        .ok_or(Reject::Invalid)?;
    if entry.take().is_none() {
        return Ok(Vec::new());
    }
    Ok(vec![Event::Equipped {
        player: id,
        slot,
        item: None,
    }])
}

fn buy_backpack_upgrade(state: &mut WorldState, id: PlayerId) -> Result<Vec<Event>, Reject> {
    let player = state.players.get_mut(&id).ok_or(Reject::Invalid)?;
    let bought = player.inventory.upgrades();
    if bought >= MAX_UPGRADES {
        return Err(Reject::Invalid);
    }
    let gems = player
        .gems
        .checked_sub(upgrade_cost(bought))
        .ok_or(Reject::NotEnoughGems)?;

    player.gems = gems;
    player.inventory.capacity = player.inventory.capacity.saturating_add(SLOTS_PER_UPGRADE);
    Ok(vec![
        Event::GemsChanged { player: id, gems },
        Event::BackpackUpgraded {
            player: id,
            capacity: player.inventory.capacity,
        },
    ])
}

/// Owner-only change to the claim whose block is at `(x, y)`, which must be in reach. `change`
/// edits a copy; the claim's tiles never change, so the index stays as it is.
fn update_claim(
    state: &mut WorldState,
    id: PlayerId,
    x: u32,
    y: u32,
    change: impl FnOnce(&mut Claim) -> Result<(), Reject>,
) -> Result<Vec<Event>, Reject> {
    let (_, key) = reachable_tile(state, id, x, y)?;
    let current = state.world.claim(key).ok_or(Reject::Invalid)?;
    if current.owner != id {
        return Err(Reject::NotOwner);
    }
    let mut claim = current.clone();
    change(&mut claim)?;
    if claim == *current {
        return Ok(Vec::new());
    }
    state
        .world
        .extras
        .insert(key, TileExtra::Claim(claim.clone()));
    Ok(vec![Event::ClaimUpdated { x, y, claim }])
}

fn is_solid(world: &World, x: u32, y: u32) -> bool {
    let fg = world.get(x, y).and_then(|tile| item(tile.fg));
    fg.is_some_and(|def| def.collision == Collision::Solid)
}

/// Same item on the same tile merges into one stack, unless the count would overflow.
fn merges(drop: &Drop, dropped: ItemId, count: u16, x: u32, y: u32) -> bool {
    (drop.item, drop.x, drop.y) == (dropped, x, y) && drop.count.checked_add(count).is_some()
}

fn can_drop(world: &World, dropped: ItemId, count: u16, x: u32, y: u32) -> bool {
    world.drops.len() < MAX_DROPS_PER_WORLD
        || world
            .drops
            .iter()
            .any(|drop| merges(drop, dropped, count, x, y))
}

/// Merges into a stack on the tile or adds a new drop. `None`: the world is at its drop limit and
/// the items are lost.
fn put_drop(world: &mut World, dropped: ItemId, count: u16, x: u32, y: u32) -> Option<Drop> {
    if let Some(drop) = world
        .drops
        .iter_mut()
        .find(|drop| merges(drop, dropped, count, x, y))
    {
        drop.count += count;
        return Some(*drop);
    }
    if world.drops.len() >= MAX_DROPS_PER_WORLD {
        return None;
    }
    let drop = Drop {
        id: world.next_drop_id,
        item: dropped,
        count,
        x,
        y,
    };
    // ponytail: ids wrap after 2^32 drops in one world; widen to u64 if a world ever lives that long.
    world.next_drop_id = world.next_drop_id.wrapping_add(1);
    world.drops.push(drop);
    Some(drop)
}

/// Admin commands (docs/knowledge/13, `commands::Admin`). The caller has already checked the
/// caller's role; this validates the command against the world, then writes, then reports every
/// change as an `Event`, exactly like `apply`. A `Reject` means nothing changed.
pub fn apply_admin(
    state: &mut WorldState,
    by: PlayerId,
    admin: crate::commands::Admin,
    _now: f64,
) -> Result<Vec<Event>, Reject> {
    use crate::commands::Admin;
    let in_range = |count: u16| (1..=MAX_STACK).contains(&count);
    match admin {
        Admin::Give {
            to,
            item: given,
            count,
        } => {
            item(given).ok_or(Reject::Invalid)?;
            if !in_range(count) {
                return Err(Reject::Invalid);
            }
            let player = state.players.get_mut(&to).ok_or(Reject::Invalid)?;
            // Clamped to the stack cap and free slots; the new total tells the caller how many fit.
            if player.inventory.add(given, count) == 0 {
                return Err(Reject::InventoryFull);
            }
            Ok(vec![Event::InventoryChanged {
                player: to,
                item: given,
                count: player.inventory.count(given),
            }])
        }
        Admin::Take {
            from,
            item: taken,
            count,
        } => {
            let def = item(taken).ok_or(Reject::Invalid)?;
            if !in_range(count) || def.has(PERMANENT) {
                return Err(Reject::Invalid);
            }
            let player = state.players.get_mut(&from).ok_or(Reject::Invalid)?;
            // More than held takes what is there.
            let amount = count.min(player.inventory.count(taken));
            if amount == 0 || !player.inventory.remove(taken, amount) {
                return Err(Reject::NoItem);
            }
            let left = player.inventory.count(taken);
            let mut events = vec![Event::InventoryChanged {
                player: from,
                item: taken,
                count: left,
            }];
            let worn = slot_of(taken).filter(|&slot| player.worn(slot) == Some(taken));
            if let Some(slot) = worn.filter(|_| left == 0) {
                if let Some(entry) = player.equipped.get_mut(slot as usize) {
                    *entry = None;
                }
                events.push(Event::Equipped {
                    player: from,
                    slot,
                    item: None,
                });
            }
            Ok(events)
        }
        Admin::Spawn {
            item: spawned,
            count,
        } => {
            let def = item(spawned).ok_or(Reject::Invalid)?;
            if !in_range(count) || def.has(UNTRADEABLE | PERMANENT) {
                return Err(Reject::Invalid);
            }
            let pos = state.players.get(&by).ok_or(Reject::Invalid)?.body.pos;
            let (x, y) = world_to_tile(pos.0, pos.1).ok_or(Reject::Invalid)?;
            if !can_drop(&state.world, spawned, count, x, y) {
                return Err(Reject::DropLimit);
            }
            let drop = put_drop(&mut state.world, spawned, count, x, y).ok_or(Reject::DropLimit)?;
            Ok(vec![Event::ItemDropped { drop }])
        }
        Admin::Gems { who, delta } => {
            let player = state.players.get_mut(&who).ok_or(Reject::Invalid)?;
            let gems = (i64::from(player.gems).saturating_add(delta)).clamp(0, i64::from(u32::MAX));
            let gems = u32::try_from(gems).unwrap_or(u32::MAX);
            if gems == player.gems {
                return Ok(Vec::new());
            }
            player.gems = gems;
            Ok(vec![Event::GemsChanged { player: who, gems }])
        }
        Admin::SetBlock { x, y, item: set } => {
            let def = item(set).ok_or(Reject::Invalid)?;
            let (tile, key) = admin_tile(state, x, y)?;
            // No new fixtures: doors and bedrock come from world generation only.
            if def.has(UNBREAKABLE) {
                return Err(Reject::Invalid);
            }
            let new_tile = match def.kind {
                ItemKind::Block => Tile { fg: set, ..tile },
                ItemKind::Background => Tile { bg: set, ..tile },
                _ => return Err(Reject::Invalid),
            };
            let traps_a_player = def.kind == ItemKind::Block
                && def.collision == Collision::Solid
                && state
                    .players
                    .values()
                    .any(|player| hitbox_overlaps_tile(player.body.pos, x, y));
            if traps_a_player {
                return Err(Reject::Occupied);
            }
            Ok(admin_write(state, x, y, key, tile, new_tile))
        }
        Admin::ClearInv { from } => {
            if !state.players.contains_key(&from) {
                return Err(Reject::Invalid);
            }
            // An open trade ends first; nothing in it has moved.
            let mut events = end_trades_of(state, from);
            let player = state.players.get_mut(&from).ok_or(Reject::Invalid)?;
            for slot in Slot::ALL {
                if player.equipped[slot as usize].take().is_some() {
                    events.push(Event::Equipped {
                        player: from,
                        slot,
                        item: None,
                    });
                }
            }
            let held: Vec<(ItemId, u16)> = player.inventory.slots().to_vec();
            for (item, count) in held {
                // `remove` refuses permanent items (fist, wrench): they stay.
                if player.inventory.remove(item, count) {
                    events.push(Event::InventoryChanged {
                        player: from,
                        item,
                        count: 0,
                    });
                }
            }
            Ok(events)
        }
        Admin::GiveAll { to } => {
            let player = state.players.get_mut(&to).ok_or(Reject::Invalid)?;
            let mut events = Vec::new();
            let biggest = crate::inventory::STARTING_SLOTS + MAX_UPGRADES * SLOTS_PER_UPGRADE;
            if player.inventory.capacity < biggest {
                player.inventory.capacity = biggest;
                events.push(Event::BackpackUpgraded {
                    player: to,
                    capacity: biggest,
                });
            }
            // Fixtures (door, bedrock) are never held; the fist and wrench are already there.
            for def in crate::items::ITEMS
                .iter()
                .filter(|d| !d.has(UNBREAKABLE | PERMANENT))
            {
                let one = matches!(
                    def.kind,
                    ItemKind::Equip(_) | ItemKind::Claim | ItemKind::Tool
                );
                let want = if one { 1 } else { MAX_STACK };
                let held = player.inventory.count(def.id);
                if held < want && player.inventory.add(def.id, want - held) > 0 {
                    events.push(Event::InventoryChanged {
                        player: to,
                        item: def.id,
                        count: player.inventory.count(def.id),
                    });
                }
            }
            Ok(events)
        }
        Admin::ClearBlock { x, y } => {
            let (tile, key) = admin_tile(state, x, y)?;
            if tile == Tile::default() {
                return Ok(Vec::new());
            }
            Ok(admin_write(state, x, y, key, tile, Tile::default()))
        }
    }
}

/// The tile an admin command edits, refusing out-of-bounds and fixtures (main door, bedrock).
fn admin_tile(state: &WorldState, x: u32, y: u32) -> Result<(Tile, u32), Reject> {
    let tile = state.world.get(x, y).ok_or(Reject::OutOfRange)?;
    let key = World::tile_key(x, y).ok_or(Reject::OutOfRange)?;
    let fixture = |id: ItemId| item(id).is_some_and(|def| def.has(UNBREAKABLE));
    if (x, y) == state.world.door || fixture(tile.fg) || fixture(tile.bg) {
        return Err(Reject::Unbreakable);
    }
    Ok((tile, key))
}

/// Writes an admin tile edit. A replaced foreground takes its tree or claim with it, like a break.
fn admin_write(
    state: &mut WorldState,
    x: u32,
    y: u32,
    key: u32,
    old: Tile,
    new: Tile,
) -> Vec<Event> {
    let had_claim = old.fg != new.fg && state.world.claim(key).is_some();
    if old.fg != new.fg {
        clear_foreground(state, x, y, old);
    }
    state.world.set(x, y, new);
    state.damage.remove(&key);
    let mut events = vec![Event::TileChanged { x, y, tile: new }];
    if had_claim {
        events.push(Event::ClaimRemoved { x, y });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::farming::grow_seconds;
    use crate::inventory::STARTING_SLOTS;
    use crate::items::{
        BEDROCK, CAVE_WALL, CRATE, DIRT, EQUIP_ID_BASE, EQUIPMENT, FIST, GRAVEL, HAIR_BROWN,
        LARGE_CLAIM, MEDIUM_CLAIM, PANTS_JEANS_BLUE, SAND, SHIRT_RED, SHOES_BOOTS_BROWN,
        SMALL_CLAIM, STONE, STONE_BRICKS, WORLD_CLAIM, WRENCH,
    };
    use crate::physics::HITBOX_HEIGHT;
    use crate::{TILE_SIZE, WORLD_HEIGHT, WORLD_WIDTH};

    const ME: PlayerId = PlayerId(1);
    /// The player's tile in the fixture; the head reaches into the tile above.
    const HOME: (u32, u32) = (10, 1);
    const DOOR: (u32, u32) = (30, 1);
    /// Empty tile on the floor, in reach of `HOME`: a valid place to plant.
    const PLOT: (u32, u32) = (11, 1);
    const DIRT_SEED: ItemId = ItemId(1001);
    const STONE_SEED: ItemId = ItemId(1002);
    const SAND_SEED: ItemId = ItemId(1005);
    const GRAVEL_SEED: ItemId = ItemId(1009);

    fn fg(item: ItemId) -> Tile {
        Tile {
            fg: item,
            bg: EMPTY,
        }
    }

    fn stand_on(state: &mut WorldState, id: PlayerId, (x, y): (u32, u32)) {
        let body = &mut state.players.get_mut(&id).unwrap().body;
        body.pos = (
            (x as f32 + 0.5) * TILE_SIZE,
            y as f32 * TILE_SIZE + HITBOX_HEIGHT / 2.0,
        );
    }

    /// Bedrock floor on row 0, a door at `DOOR`, and one player standing on the floor at `HOME`.
    fn fixture() -> WorldState {
        let mut world = World::empty("T".into());
        for x in 0..WORLD_WIDTH {
            world.set(x, 0, fg(BEDROCK));
        }
        world.set(DOOR.0, DOOR.1, fg(MAIN_DOOR));
        world.door = DOOR;
        let mut state = WorldState::new(world, 7);
        state.join(ME);
        stand_on(&mut state, ME, HOME);
        state
    }

    fn give(state: &mut WorldState, item: ItemId, count: u16) {
        assert_eq!(
            state
                .players
                .get_mut(&ME)
                .unwrap()
                .inventory
                .add(item, count),
            count
        );
    }

    fn held(state: &WorldState, item: ItemId) -> u16 {
        state.players[&ME].inventory.count(item)
    }

    /// The equipment id of an `assets/items` folder name.
    fn equipment(name: &str) -> ItemId {
        let index = EQUIPMENT.iter().position(|&n| n == name).unwrap();
        ItemId(EQUIP_ID_BASE + index as u16)
    }

    fn worn(state: &WorldState, slot: Slot) -> Option<ItemId> {
        state.players[&ME].worn(slot)
    }

    fn punch_at(
        state: &mut WorldState,
        (x, y): (u32, u32),
        now: f64,
    ) -> Result<Vec<Event>, Reject> {
        apply(state, ME, Action::Punch { x, y }, now)
    }

    fn place_at(
        state: &mut WorldState,
        (x, y): (u32, u32),
        item: ItemId,
        now: f64,
    ) -> Result<Vec<Event>, Reject> {
        apply(state, ME, Action::Place { x, y, item }, now)
    }

    fn dropped_items(events: &[Event]) -> Vec<(ItemId, u16)> {
        let drops = events.iter().filter_map(|event| match event {
            Event::ItemDropped { drop } => Some((drop.item, drop.count)),
            _ => None,
        });
        drops.collect()
    }

    /// Fill the world's drop list to the limit with stacks far from the player.
    fn fill_drops(state: &mut WorldState) {
        while state.world.drops.len() < MAX_DROPS_PER_WORLD {
            let id = state.world.next_drop_id;
            state.world.next_drop_id += 1;
            state.world.drops.push(Drop {
                id,
                item: STONE,
                count: 1,
                x: 50,
                y: 20,
            });
        }
    }

    /// A seed id sits in a foreground exactly where a tree extra exists, a claim block exactly
    /// where a claim extra exists, and the claim index equals the one rebuilt from the extras.
    fn assert_trees_match_tiles(state: &WorldState) {
        let (mut trees, mut claims) = (0, 0);
        for y in 0..WORLD_HEIGHT {
            for x in 0..WORLD_WIDTH {
                let tile = state.world.get(x, y).unwrap();
                let kind = item(tile.fg).map(|def| def.kind);
                let tree = state.world.tree(x, y);
                let claim = state.world.claim(World::tile_key(x, y).unwrap());
                assert_eq!(
                    kind == Some(ItemKind::Seed),
                    tree.is_some(),
                    "({x},{y}) {tile:?} {tree:?}"
                );
                assert_eq!(
                    kind == Some(ItemKind::Claim),
                    claim.is_some(),
                    "({x},{y}) {tile:?} {claim:?}"
                );
                if let Some(tree) = tree {
                    assert_eq!(seed_of(tree.item), Some(tile.fg), "({x},{y})");
                    trees += 1;
                }
                if let Some(claim) = claim {
                    let is_world = claim_kind(tile.fg).unwrap().tiles.is_none();
                    assert_eq!(claim.tiles.is_empty(), is_world, "({x},{y})");
                    claims += 1;
                }
            }
        }
        assert_eq!(state.world.extras.len(), trees + claims);
        let index = (state.world.claim_of.clone(), state.world.world_claim);
        assert_eq!(state.world.claim_index_from_extras(), index);
    }

    #[test]
    fn new_player_stands_on_the_door_with_fist_wrench_and_the_starter_outfit_worn() {
        let mut state = fixture();
        state.join(PlayerId(2));
        let player = &state.players[&PlayerId(2)];
        assert_eq!(player.body, spawn_body(&state.world));
        let mut expected = Inventory::default();
        for outfit in STARTER_OUTFIT {
            expected.add(outfit, 1);
        }
        assert_eq!(player.inventory, expected);
        assert_eq!(
            player.equipped,
            [
                Some(HAIR_BROWN),
                None,
                None,
                Some(SHIRT_RED),
                Some(PANTS_JEANS_BLUE),
                Some(SHOES_BOOTS_BROWN),
                None,
                None
            ]
        );
        assert_eq!(player.gems, 0);
        // Joining twice keeps the existing player.
        let before = state.clone();
        state.join(ME);
        assert_eq!(state, before);
    }

    #[test]
    fn three_hit_block_breaks_on_the_third_punch_unless_damage_expired() {
        let target = (12, 1);
        let mut state = fixture();
        state.world.set(target.0, target.1, fg(DIRT));
        let damaged = |damage| Event::TileDamaged {
            x: 12,
            y: 1,
            layer: Layer::Foreground,
            damage,
        };
        assert_eq!(punch_at(&mut state, target, 0.0), Ok(vec![damaged(1)]));
        assert_eq!(punch_at(&mut state, target, 1.0), Ok(vec![damaged(2)]));
        let events = punch_at(&mut state, target, 2.0).unwrap();
        assert_eq!(
            events[0],
            Event::TileChanged {
                x: 12,
                y: 1,
                tile: Tile::default()
            }
        );
        assert_eq!(state.world.get(12, 1), Some(Tile::default()));
        assert!(state.damage.is_empty());

        // Twice, wait past DAMAGE_RESET, once more: back to one hit.
        state.world.set(target.0, target.1, fg(DIRT));
        assert_eq!(punch_at(&mut state, target, 10.0), Ok(vec![damaged(1)]));
        assert_eq!(punch_at(&mut state, target, 11.0), Ok(vec![damaged(2)]));
        assert_eq!(
            punch_at(&mut state, target, 11.0 + DAMAGE_RESET + 0.1),
            Ok(vec![damaged(1)])
        );
        assert_eq!(state.world.get(12, 1), Some(fg(DIRT)));
    }

    #[test]
    fn damage_is_shared_between_players() {
        let other = PlayerId(2);
        let mut state = fixture();
        state.join(other);
        stand_on(&mut state, other, (13, 1));
        state.world.set(12, 1, fg(DIRT));
        let punch = Action::Punch { x: 12, y: 1 };
        assert!(apply(&mut state, ME, punch, 0.0).is_ok());
        assert!(apply(&mut state, other, punch, 0.0).is_ok());
        assert_eq!(apply(&mut state, ME, punch, 0.1), Err(Reject::Cooldown));
        assert!(apply(&mut state, ME, punch, 0.3).is_ok());
        assert_eq!(state.world.get(12, 1), Some(Tile::default()));
    }

    #[test]
    fn reach_is_chebyshev_0_to_2() {
        let mut state = fixture();
        let body = state.players[&ME].body;
        let in_range = (0..WORLD_HEIGHT)
            .flat_map(|y| (0..WORLD_WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| in_reach(&body, x, y))
            .count();
        assert_eq!(in_range, 5 * 5 - 5); // the 5x5 square minus the row that would be below the world
        assert!(in_reach(&body, HOME.0, HOME.1));
        assert!(in_reach(&body, 12, 3) && in_reach(&body, 8, 0) && in_reach(&body, 10, 2));
        assert!(!in_reach(&body, 13, 1) && !in_reach(&body, 10, 4) && !in_reach(&body, 7, 1));

        // Distance 0: a background behind yourself, and punching it away again.
        give(&mut state, CAVE_WALL, 1);
        assert!(place_at(&mut state, HOME, CAVE_WALL, 0.0).is_ok());
        assert_eq!(
            state.world.get(HOME.0, HOME.1),
            Some(Tile {
                fg: EMPTY,
                bg: CAVE_WALL
            })
        );
        assert!(punch_at(&mut state, HOME, 0.0).is_ok());
        // Distance 2 and 3.
        state.world.set(12, 3, fg(DIRT));
        state.world.set(13, 1, fg(DIRT));
        assert!(punch_at(&mut state, (12, 3), 1.0).is_ok());
        assert_eq!(punch_at(&mut state, (13, 1), 2.0), Err(Reject::OutOfRange));
        give(&mut state, STONE, 2);
        assert!(place_at(&mut state, (8, 3), STONE, 0.0).is_ok());
        assert_eq!(
            place_at(&mut state, (7, 1), STONE, 0.0),
            Err(Reject::OutOfRange)
        );
    }

    #[test]
    fn bedrock_is_unbreakable_and_the_door_leaves_the_world() {
        let mut state = fixture();
        stand_on(&mut state, ME, (DOOR.0 - 1, DOOR.1));
        let before = state.clone();
        for now in [0.0, 1.0, 2.0, 3.0] {
            assert_eq!(
                punch_at(&mut state, (DOOR.0, 0), now),
                Err(Reject::Unbreakable)
            );
            assert_eq!(
                punch_at(&mut state, DOOR, now),
                Ok(vec![Event::LeftWorld { player: ME }])
            );
        }
        assert_eq!(state, before);
    }

    #[test]
    fn punching_nothing_is_a_miss_that_uses_the_swing() {
        let mut state = fixture();
        assert_eq!(punch_at(&mut state, (11, 2), 0.0), Ok(Vec::new()));
        assert_eq!(punch_at(&mut state, (11, 2), 0.1), Err(Reject::Cooldown));
        assert_eq!(
            punch_at(&mut state, (11, 2), PUNCH_COOLDOWN),
            Ok(Vec::new())
        );
    }

    #[test]
    fn foreground_breaks_before_background() {
        let target = (11, 1);
        let mut state = fixture();
        state.world.set(
            11,
            1,
            Tile {
                fg: DIRT,
                bg: CAVE_WALL,
            },
        );
        for now in [0.0, 1.0, 2.0] {
            punch_at(&mut state, target, now).unwrap();
        }
        assert_eq!(
            state.world.get(11, 1),
            Some(Tile {
                fg: EMPTY,
                bg: CAVE_WALL
            })
        );
        // The wall needs its own hits, starting from zero.
        let first = punch_at(&mut state, target, 3.0).unwrap();
        assert_eq!(
            first,
            vec![Event::TileDamaged {
                x: 11,
                y: 1,
                layer: Layer::Background,
                damage: 1
            }]
        );
        punch_at(&mut state, target, 4.0).unwrap();
        assert_eq!(state.world.get(11, 1), Some(Tile::default()));
    }

    #[test]
    fn solid_blocks_cannot_be_placed_on_a_player_but_backgrounds_can() {
        let head = (HOME.0, HOME.1 + 1);
        let mut state = fixture();
        give(&mut state, DIRT, 3);
        give(&mut state, CAVE_WALL, 1);
        assert_eq!(place_at(&mut state, head, DIRT, 0.0), Err(Reject::Occupied));
        assert_eq!(place_at(&mut state, HOME, DIRT, 0.0), Err(Reject::Occupied));
        assert!(place_at(&mut state, head, CAVE_WALL, 0.0).is_ok());
        assert_eq!(
            state.world.get(head.0, head.1),
            Some(Tile {
                fg: EMPTY,
                bg: CAVE_WALL
            })
        );
        // One tile higher the hitbox is clear.
        assert!(place_at(&mut state, (head.0, head.1 + 1), DIRT, 0.0).is_ok());
    }

    #[test]
    fn placing_takes_exactly_one_item_and_needs_one() {
        let mut state = fixture();
        give(&mut state, DIRT, 5);
        let events = place_at(&mut state, (12, 2), DIRT, 0.0).unwrap();
        assert_eq!(
            events,
            vec![
                Event::TileChanged {
                    x: 12,
                    y: 2,
                    tile: fg(DIRT)
                }, // mid-air: no neighbour needed
                Event::InventoryChanged {
                    player: ME,
                    item: DIRT,
                    count: 4
                },
            ]
        );
        assert_eq!(held(&state, DIRT), 4);
        assert_eq!(
            place_at(&mut state, (12, 2), DIRT, 0.0),
            Err(Reject::Occupied)
        );
        assert_eq!(
            place_at(&mut state, (12, 3), STONE, 0.0),
            Err(Reject::NoItem)
        );
        assert_eq!(held(&state, DIRT), 4);
    }

    #[test]
    fn every_reject_leaves_the_state_unchanged() {
        let mut state = fixture();
        state.world.set(12, 1, fg(DIRT));
        give(&mut state, DIRT, 5);
        give(&mut state, CAVE_WALL, 5);
        give(&mut state, DIRT_SEED, 5);
        give(&mut state, STONE_SEED, 5);
        give(&mut state, SAND_SEED, 5);
        let pickaxe = equipment("pickaxe_iron");
        give(&mut state, pickaxe, 1);
        punch_at(&mut state, (12, 1), 10.0).unwrap();
        // Player two tiles left of the door, a dirt block between them, a wall to the far left.
        let (px, py) = (DOOR.0 - 2, DOOR.1);
        stand_on(&mut state, ME, (px, py));
        state.world.set(px + 1, py, fg(DIRT));
        state.world.set(px - 1, py, fg(STONE));
        // Two ledges beside the player's head: a spliced sapling (gravel) on one, a dirt sapling on the other.
        state.world.set(px - 1, py + 1, fg(BEDROCK));
        state.world.set(px + 1, py + 1, fg(BEDROCK));
        place_at(&mut state, (px - 1, py + 2), DIRT_SEED, 10.0).unwrap();
        place_at(&mut state, (px - 1, py + 2), STONE_SEED, 10.0).unwrap();
        place_at(&mut state, (px + 1, py + 2), DIRT_SEED, 10.0).unwrap();
        fill_drops(&mut state);
        assert_trees_match_tiles(&state);

        let far = u32::MAX;
        let nobody = PlayerId(99);
        let (spliced, sapling) = ((px - 1, py + 2), (px + 1, py + 2));
        let place = |(x, y): (u32, u32), item| Action::Place { x, y, item };
        let cases = [
            (
                ME,
                Action::Punch { x: far, y: far },
                20.0,
                Reject::OutOfRange,
            ),
            (ME, Action::Punch { x: far, y: 1 }, 20.0, Reject::OutOfRange),
            (
                ME,
                Action::Punch {
                    x: px,
                    y: WORLD_HEIGHT,
                },
                20.0,
                Reject::OutOfRange,
            ),
            (
                ME,
                Action::Punch { x: px + 3, y: py },
                20.0,
                Reject::OutOfRange,
            ),
            (
                ME,
                Action::Punch { x: px + 1, y: py },
                10.1,
                Reject::Cooldown,
            ),
            (
                ME,
                Action::Punch { x: px + 1, y: 0 },
                20.0,
                Reject::Unbreakable,
            ),
            (
                nobody,
                Action::Punch { x: px + 1, y: py },
                20.0,
                Reject::Invalid,
            ),
            (ME, place((far, far), DIRT), 20.0, Reject::OutOfRange),
            (ME, place((0, far), DIRT), 20.0, Reject::OutOfRange),
            (ME, place((px + 1, py), DIRT), 20.0, Reject::Occupied),
            (ME, place(DOOR, CAVE_WALL), 20.0, Reject::Occupied), // door tile
            (ME, place((px, py + 1), DIRT), 20.0, Reject::Occupied), // own head
            (ME, place((px, py), DIRT), 20.0, Reject::Occupied),  // own tile
            (ME, place(sapling, DIRT), 20.0, Reject::Occupied),   // a tree is in the way
            (ME, place((px, py + 2), STONE), 20.0, Reject::NoItem),
            (ME, place((px, py + 2), FIST), 20.0, Reject::Invalid),
            (ME, place((px, py + 2), EMPTY), 20.0, Reject::Invalid),
            (
                ME,
                place((px, py + 2), ItemId(u16::MAX)),
                20.0,
                Reject::Invalid,
            ),
            (nobody, place((px, py + 2), DIRT), 20.0, Reject::Invalid),
            // Seeds.
            (
                ME,
                place((px + 2, py + 2), DIRT_SEED),
                20.0,
                Reject::NeedsGround,
            ), // nothing solid below
            (ME, place((px + 1, py), DIRT_SEED), 20.0, Reject::Occupied), // a block is there
            (ME, place((px, py + 2), GRAVEL_SEED), 20.0, Reject::NoItem),
            (ME, place(sapling, DIRT_SEED), 20.0, Reject::NoRecipe), // its own seed
            (ME, place(spliced, SAND_SEED), 20.0, Reject::AlreadySpliced),
            (ME, place((far, 0), DIRT_SEED), 20.0, Reject::OutOfRange),
            (nobody, place(sapling, STONE_SEED), 20.0, Reject::Invalid),
            // Drop and trash.
            (
                ME,
                Action::Drop {
                    item: DIRT,
                    count: 0,
                    left: true,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Drop {
                    item: DIRT,
                    count: u16::MAX,
                    left: true,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Drop {
                    item: DIRT,
                    count: MAX_STACK + 1,
                    left: false,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Drop {
                    item: DIRT,
                    count: 6,
                    left: false,
                },
                20.0,
                Reject::NoItem,
            ),
            (
                ME,
                Action::Drop {
                    item: STONE,
                    count: 1,
                    left: false,
                },
                20.0,
                Reject::NoItem,
            ),
            (
                ME,
                Action::Drop {
                    item: FIST,
                    count: 1,
                    left: false,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Drop {
                    item: WRENCH,
                    count: 1,
                    left: true,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Drop {
                    item: EMPTY,
                    count: 1,
                    left: true,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                nobody,
                Action::Drop {
                    item: DIRT,
                    count: 1,
                    left: true,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Drop {
                    item: DIRT,
                    count: 1,
                    left: true,
                },
                20.0,
                Reject::Occupied,
            ), // stone wall
            (
                ME,
                Action::Drop {
                    item: DIRT,
                    count: 1,
                    left: false,
                },
                20.0,
                Reject::Occupied,
            ), // dirt block
            (
                ME,
                Action::Trash {
                    item: DIRT,
                    count: 0,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Trash {
                    item: DIRT,
                    count: u16::MAX,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Trash {
                    item: DIRT,
                    count: 6,
                },
                20.0,
                Reject::NoItem,
            ),
            (
                ME,
                Action::Trash {
                    item: FIST,
                    count: 1,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Trash {
                    item: WRENCH,
                    count: 1,
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Trash {
                    item: ItemId(u16::MAX),
                    count: 1,
                },
                20.0,
                Reject::Invalid,
            ),
            // Equipment.
            (ME, place((px, py + 2), SHIRT_RED), 20.0, Reject::Invalid),
            (ME, place((px, py + 2), pickaxe), 20.0, Reject::Invalid),
            (ME, Action::Equip { item: DIRT }, 20.0, Reject::Invalid),
            (ME, Action::Equip { item: FIST }, 20.0, Reject::Invalid),
            (ME, Action::Equip { item: EMPTY }, 20.0, Reject::Invalid),
            (
                ME,
                Action::Equip {
                    item: ItemId(u16::MAX),
                },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Equip {
                    item: equipment("wings_bone"),
                },
                20.0,
                Reject::NoItem,
            ),
            (
                nobody,
                Action::Equip { item: HAIR_BROWN },
                20.0,
                Reject::Invalid,
            ),
            (
                nobody,
                Action::Unequip { slot: Slot::Hair },
                20.0,
                Reject::Invalid,
            ),
            (
                ME,
                Action::Buy {
                    item: equipment("wings_bone"),
                },
                20.0,
                Reject::NotEnoughGems,
            ),
        ];
        let before = state.clone();
        for (player, action, now, reject) in cases {
            assert_eq!(
                apply(&mut state, player, action, now),
                Err(reject),
                "{action:?}"
            );
            assert_eq!(state, before, "{action:?} changed the state");
            assert_eq!(xp_of(&state), xp_of(&before), "{action:?} changed XP");
        }

        // With the way clear, the full world still refuses a new drop.
        state.world.set(px - 1, py, Tile::default());
        let before = state.clone();
        let drop = Action::Drop {
            item: DIRT,
            count: 1,
            left: true,
        };
        assert_eq!(apply(&mut state, ME, drop, 20.0), Err(Reject::DropLimit));
        assert_eq!(state, before);

        // Trades: first with none open, then inside an open one, then in review.
        let you = PlayerId(2);
        state.join(you);
        stand_on(&mut state, you, (50, 1));
        let offer = |slot, item, count| Action::TradeSetOffer { slot, item, count };
        let check = |state: &mut WorldState, cases: &[(PlayerId, Action, Reject)]| {
            let before = state.clone();
            for &(player, action, reject) in cases {
                assert_eq!(
                    apply(state, player, action, 20.5),
                    Err(reject),
                    "{action:?}"
                );
                assert_eq!(*state, before, "{action:?} changed the state");
                assert_eq!(xp_of(state), xp_of(&before), "{action:?} changed XP");
            }
        };
        check(
            &mut state,
            &[
                (ME, Action::TradeRequest { to: ME }, Reject::Invalid),
                (ME, Action::TradeRequest { to: nobody }, Reject::Invalid),
                (nobody, Action::TradeRequest { to: ME }, Reject::Invalid),
                (ME, Action::TradeRespond { accept: true }, Reject::Invalid),
                (ME, offer(0, DIRT, 1), Reject::Invalid),
                (ME, Action::TradeLock { locked: true }, Reject::Invalid),
                (ME, Action::TradeConfirm, Reject::Invalid),
                (ME, Action::TradeCancel, Reject::Invalid),
            ],
        );
        apply(&mut state, ME, Action::TradeRequest { to: you }, 20.0).unwrap();
        check(
            &mut state,
            &[
                (ME, Action::TradeRespond { accept: true }, Reject::Invalid),
                (ME, offer(0, DIRT, 1), Reject::Invalid),
                (you, Action::TradeRequest { to: ME }, Reject::Invalid),
            ],
        );
        apply(&mut state, you, Action::TradeRespond { accept: true }, 20.0).unwrap();
        let dirt = held(&state, DIRT);
        check(
            &mut state,
            &[
                (ME, offer(0, FIST, 1), Reject::Invalid),
                (ME, offer(0, WRENCH, 1), Reject::Invalid),
                (ME, offer(0, EMPTY, 1), Reject::Invalid),
                (ME, offer(0, DIRT, MAX_STACK + 1), Reject::Invalid),
                (ME, offer(0, DIRT, dirt + 1), Reject::NoItem),
                (ME, offer(4, DIRT, 1), Reject::Invalid),
                (ME, Action::TradeConfirm, Reject::Invalid),
                (you, Action::TradeRespond { accept: true }, Reject::Invalid),
                (ME, Action::TradeRequest { to: you }, Reject::Invalid),
            ],
        );
        apply(&mut state, ME, offer(0, DIRT, 1), 20.0).unwrap();
        check(&mut state, &[(ME, offer(1, DIRT, 1), Reject::Invalid)]);
        apply(&mut state, ME, Action::TradeLock { locked: true }, 20.0).unwrap();
        apply(&mut state, you, Action::TradeLock { locked: true }, 20.0).unwrap();
        check(
            &mut state,
            &[
                (ME, Action::TradeConfirm, Reject::Cooldown),
                (you, Action::TradeConfirm, Reject::Cooldown),
                (ME, offer(1, CAVE_WALL, 1), Reject::Invalid),
                (ME, Action::TradeLock { locked: false }, Reject::Invalid),
            ],
        );
    }

    #[test]
    fn equip_needs_the_item_replaces_the_slot_and_unequip_empties_it() {
        let mut state = fixture();
        let blue = equipment("hair_bob_blue");
        assert_eq!(
            apply(&mut state, ME, Action::Equip { item: blue }, 0.0),
            Err(Reject::NoItem)
        );
        give(&mut state, blue, 1);
        let equipped = |item| Event::Equipped {
            player: ME,
            slot: Slot::Hair,
            item,
        };
        // Into the filled hair slot: the brown hair comes off but stays in the inventory.
        assert_eq!(
            apply(&mut state, ME, Action::Equip { item: blue }, 0.0),
            Ok(vec![equipped(Some(blue))])
        );
        assert_eq!(worn(&state, Slot::Hair), Some(blue));
        assert_eq!((held(&state, blue), held(&state, HAIR_BROWN)), (1, 1));
        // Again: nothing changes.
        assert_eq!(
            apply(&mut state, ME, Action::Equip { item: blue }, 0.0),
            Ok(vec![])
        );
        // Off, and off again.
        let off = Action::Unequip { slot: Slot::Hair };
        assert_eq!(apply(&mut state, ME, off, 0.0), Ok(vec![equipped(None)]));
        assert_eq!(worn(&state, Slot::Hair), None);
        assert_eq!(held(&state, blue), 1);
        let before = state.clone();
        assert_eq!(apply(&mut state, ME, off, 0.0), Ok(vec![]));
        assert_eq!(state, before);
        // Other slots are untouched throughout.
        assert_eq!(worn(&state, Slot::Shirt), Some(SHIRT_RED));
    }

    #[test]
    fn losing_the_last_worn_copy_unequips_it_in_the_same_result() {
        let mut state = fixture();
        let off = Event::Equipped {
            player: ME,
            slot: Slot::Shirt,
            item: None,
        };
        // Two copies: dropping one keeps the shirt on.
        give(&mut state, SHIRT_RED, 1);
        let drop = Action::Drop {
            item: SHIRT_RED,
            count: 1,
            left: false,
        };
        let events = apply(&mut state, ME, drop, 0.0).unwrap();
        assert!(!events.contains(&off), "{events:?}");
        assert_eq!(worn(&state, Slot::Shirt), Some(SHIRT_RED));
        // The last copy: off in the same result, after the inventory change.
        let events = apply(&mut state, ME, drop, 0.0).unwrap();
        assert_eq!(
            events[..2],
            [
                Event::InventoryChanged {
                    player: ME,
                    item: SHIRT_RED,
                    count: 0
                },
                off
            ]
        );
        assert!(matches!(events[2], Event::ItemDropped { .. }));
        assert_eq!(worn(&state, Slot::Shirt), None);

        // Trash does the same.
        let trash = Action::Trash {
            item: PANTS_JEANS_BLUE,
            count: 1,
        };
        assert_eq!(
            apply(&mut state, ME, trash, 0.0),
            Ok(vec![
                Event::InventoryChanged {
                    player: ME,
                    item: PANTS_JEANS_BLUE,
                    count: 0
                },
                Event::Equipped {
                    player: ME,
                    slot: Slot::Pants,
                    item: None
                },
            ])
        );
        assert_eq!(worn(&state, Slot::Pants), None);
        // Losing an item that is owned but not worn says nothing about slots.
        give(&mut state, equipment("hat_straw"), 1);
        let trash = Action::Trash {
            item: equipment("hat_straw"),
            count: 1,
        };
        assert_eq!(apply(&mut state, ME, trash, 0.0).map(|e| e.len()), Ok(1));
    }

    #[test]
    fn equipment_cannot_be_placed() {
        let mut state = fixture();
        give(&mut state, equipment("pickaxe_wood"), 1);
        let before = state.clone();
        for item in [HAIR_BROWN, SHIRT_RED, equipment("pickaxe_wood")] {
            assert_eq!(place_at(&mut state, PLOT, item, 0.0), Err(Reject::Invalid));
        }
        assert_eq!(state, before);
    }

    #[test]
    fn a_pickaxe_adds_its_power_to_every_punch() {
        let target = (12, 1);
        let damaged = |damage| {
            Ok(vec![Event::TileDamaged {
                x: target.0,
                y: target.1,
                layer: Layer::Foreground,
                damage,
            }])
        };
        let breaks = |state: &mut WorldState| {
            state.world.set(target.0, target.1, fg(STONE));
            let mut now = 0.0;
            let mut punches = 0;
            while state.world.get(target.0, target.1) == Some(fg(STONE)) {
                now += 1.0;
                punches += 1;
                punch_at(state, target, now).unwrap();
            }
            punches
        };
        assert_eq!(item(STONE).unwrap().hits, 6);

        // No pickaxe: the fist, 6 punches.
        let mut state = fixture();
        state.players.get_mut(&ME).unwrap().equipped = [None; SLOT_COUNT];
        assert_eq!(breaks(&mut state), 6);

        // Power 4: 2 punches, and the damage reported is the accumulated total.
        let mut state = fixture();
        let royal = equipment("pickaxe_royal");
        give(&mut state, royal, 1);
        apply(&mut state, ME, Action::Equip { item: royal }, 0.0).unwrap();
        assert_eq!(breaks(&mut state), 2);
        state.world.set(target.0, target.1, fg(STONE));
        assert_eq!(punch_at(&mut state, target, 100.0), damaged(4));
        assert_eq!(state.damage.values().next().map(|d| d.hits), Some(4));

        // Power 2 on dirt (3 hits): 2, then broken. Backgrounds take the power too.
        let mut state = fixture();
        let iron = equipment("pickaxe_iron");
        give(&mut state, iron, 1);
        apply(&mut state, ME, Action::Equip { item: iron }, 0.0).unwrap();
        state.world.set(
            target.0,
            target.1,
            Tile {
                fg: DIRT,
                bg: CAVE_WALL,
            },
        );
        assert_eq!(punch_at(&mut state, target, 1.0), damaged(2));
        assert!(punch_at(&mut state, target, 2.0).is_ok());
        assert_eq!(
            state.world.get(target.0, target.1).map(|t| t.fg),
            Some(EMPTY)
        );
        // The cave wall has 2 hits: one punch.
        assert!(punch_at(&mut state, target, 3.0).is_ok());
        assert_eq!(state.world.get(target.0, target.1), Some(Tile::default()));

        // A sapling (3 hits) goes in 2 punches; a grown tree still in one.
        give(&mut state, DIRT_SEED, 2);
        place_at(&mut state, PLOT, DIRT_SEED, 10.0).unwrap();
        assert!(punch_at(&mut state, PLOT, 11.0).is_ok());
        assert!(punch_at(&mut state, PLOT, 12.0).is_ok());
        assert!(state.world.tree(PLOT.0, PLOT.1).is_none());
        place_at(&mut state, PLOT, DIRT_SEED, 20.0).unwrap();
        let grown = 20.0 + grow_seconds(1) as f64;
        let events = punch_at(&mut state, PLOT, grown).unwrap();
        assert!(events.contains(&Event::TreeHarvested {
            x: PLOT.0,
            y: PLOT.1
        }));
    }

    #[test]
    fn buying_equipment_charges_the_table_price_and_does_not_equip() {
        let mut state = fixture();
        let wings = equipment("wings_bone");
        let pickaxe = equipment("pickaxe_gold");
        state.players.get_mut(&ME).unwrap().gems = 14_000;
        let worn_before = state.players[&ME].equipped;
        assert_eq!(
            apply(&mut state, ME, Action::Buy { item: wings }, 0.0),
            Ok(vec![
                Event::GemsChanged {
                    player: ME,
                    gems: 4_000
                },
                Event::InventoryChanged {
                    player: ME,
                    item: wings,
                    count: 1
                },
            ])
        );
        assert!(apply(&mut state, ME, Action::Buy { item: pickaxe }, 0.0).is_ok());
        assert_eq!(state.players[&ME].gems, 0);
        assert_eq!((held(&state, wings), held(&state, pickaxe)), (1, 1));
        assert_eq!(state.players[&ME].equipped, worn_before);
        assert_eq!(
            apply(&mut state, ME, Action::Buy { item: HAIR_BROWN }, 0.0),
            Err(Reject::NotEnoughGems)
        );
    }

    #[test]
    fn dropping_at_a_world_edge_is_refused() {
        let mut state = fixture();
        give(&mut state, DIRT, 2);
        stand_on(&mut state, ME, (0, 1));
        let before = state.clone();
        let drop = |left| Action::Drop {
            item: DIRT,
            count: 1,
            left,
        };
        assert_eq!(
            apply(&mut state, ME, drop(true), 0.0),
            Err(Reject::Occupied)
        );
        assert_eq!(state, before);
        stand_on(&mut state, ME, (WORLD_WIDTH - 1, 1));
        assert_eq!(
            apply(&mut state, ME, drop(false), 0.0),
            Err(Reject::Occupied)
        );
        assert!(apply(&mut state, ME, drop(true), 0.0).is_ok());
    }

    #[test]
    fn one_roll_per_break_with_the_documented_chances() {
        const BREAKS: u32 = 10_000;
        let target = (12, 1);
        let mut state = fixture();
        let (mut blocks, mut seeds, mut nothing) = (0u32, 0u32, 0u32);
        for i in 0..BREAKS {
            state.world.drops.clear(); // keep the merged stack below the u16 limit
            state.world.set(target.0, target.1, fg(SAND)); // 2 hits
            let now = f64::from(i);
            punch_at(&mut state, target, now).unwrap();
            let events = punch_at(&mut state, target, now + 0.5).unwrap();
            match dropped_items(&events).as_slice() {
                [] => nothing += 1,
                [(item, 1)] if *item == SAND => blocks += 1,
                [(item, 1)] if Some(*item) == seed_of(SAND) => seeds += 1,
                other => panic!("break {i} dropped {other:?}"),
            }
            let on_ground: u32 = state.world.drops.iter().map(|d| u32::from(d.count)).sum();
            assert_eq!(on_ground, dropped_items(&events).len() as u32);
        }
        let share = |n: u32| n as f32 / BREAKS as f32;
        assert!((share(blocks) - DROP_BLOCK).abs() < 0.02, "blocks {blocks}");
        assert!((share(seeds) - DROP_SEED).abs() < 0.02, "seeds {seeds}");
        assert!(
            (share(nothing) - (1.0 - DROP_BLOCK - DROP_SEED)).abs() < 0.02,
            "nothing {nothing}"
        );
    }

    #[test]
    fn no_seed_items_turn_the_seed_outcome_into_the_block() {
        let no_seed = item(BEDROCK).unwrap();
        let mut rng = Rng::new(3);
        let (mut blocks, total) = (0, 10_000);
        for _ in 0..total {
            if let Some(id) = roll_drop(&mut rng, no_seed) {
                assert_eq!(id, BEDROCK);
                blocks += 1;
            }
        }
        assert!((blocks as f32 / total as f32 - (DROP_BLOCK + DROP_SEED)).abs() < 0.02);
    }

    #[test]
    fn a_drop_lands_beside_the_player_and_waits_to_be_walked_onto() {
        for (left, x) in [(true, HOME.0 - 1), (false, HOME.0 + 1)] {
            let mut state = fixture();
            give(&mut state, DIRT, 30);
            let events = apply(
                &mut state,
                ME,
                Action::Drop {
                    item: DIRT,
                    count: 30,
                    left,
                },
                0.0,
            )
            .unwrap();
            let drop = Drop {
                id: 0,
                item: DIRT,
                count: 30,
                x,
                y: HOME.1,
            };
            assert_eq!(
                events,
                vec![
                    Event::InventoryChanged {
                        player: ME,
                        item: DIRT,
                        count: 0
                    },
                    Event::ItemDropped { drop },
                ]
            );
            assert_eq!(state.world.drops, vec![drop]);

            // Standing still does not take it back.
            assert!(collect_drops(&mut state, ME).is_empty());
            assert_eq!(held(&state, DIRT), 0);
            // Walking onto the tile does.
            stand_on(&mut state, ME, (x, HOME.1));
            assert_eq!(
                collect_drops(&mut state, ME),
                vec![
                    Event::DropTaken {
                        drop_id: 0,
                        by: ME,
                        count: 30
                    },
                    Event::InventoryChanged {
                        player: ME,
                        item: DIRT,
                        count: 30
                    },
                ]
            );
            assert_eq!(held(&state, DIRT), 30);
            assert!(state.world.drops.is_empty());
            assert!(collect_drops(&mut state, PlayerId(99)).is_empty());
        }
    }

    #[test]
    fn pickup_needs_the_drop_point_inside_the_hitbox() {
        let mut state = fixture();
        state.world.drops.push(Drop {
            id: 0,
            item: DIRT,
            count: 1,
            x: 12,
            y: 1,
        });
        let centre = (12.5 * TILE_SIZE, 1.5 * TILE_SIZE);
        let feet_y = TILE_SIZE + HITBOX_HEIGHT / 2.0;
        // The hitbox is 20 px wide: 11 px to the side misses, 9 px takes it.
        state.players.get_mut(&ME).unwrap().body.pos = (centre.0 - 11.0, feet_y);
        assert!(collect_drops(&mut state, ME).is_empty());
        state.players.get_mut(&ME).unwrap().body.pos = (centre.0 - 9.0, feet_y);
        assert_eq!(collect_drops(&mut state, ME).len(), 2);
    }

    #[test]
    fn pickup_is_partial_under_the_stack_cap() {
        let mut state = fixture();
        give(&mut state, DIRT, 190);
        state.world.drops.push(Drop {
            id: 5,
            item: DIRT,
            count: 30,
            x: HOME.0,
            y: HOME.1,
        });
        assert_eq!(
            collect_drops(&mut state, ME),
            vec![
                Event::DropTaken {
                    drop_id: 5,
                    by: ME,
                    count: 10
                },
                Event::InventoryChanged {
                    player: ME,
                    item: DIRT,
                    count: MAX_STACK
                },
            ]
        );
        assert_eq!(
            state.world.drops,
            vec![Drop {
                id: 5,
                item: DIRT,
                count: 20,
                x: HOME.0,
                y: HOME.1
            }]
        );
        assert!(collect_drops(&mut state, ME).is_empty());
    }

    #[test]
    fn drops_of_the_same_item_on_the_same_tile_merge() {
        let mut state = fixture();
        give(&mut state, DIRT, 10);
        give(&mut state, STONE, 10);
        for (item, count, left) in [
            (DIRT, 3, true),
            (DIRT, 4, true),
            (STONE, 2, true),
            (DIRT, 1, false),
        ] {
            apply(&mut state, ME, Action::Drop { item, count, left }, 0.0).unwrap();
        }
        let stacks: Vec<_> = state
            .world
            .drops
            .iter()
            .map(|d| (d.id, d.item, d.count, d.x))
            .collect();
        assert_eq!(
            stacks,
            vec![(0, DIRT, 7, 9), (1, STONE, 2, 9), (2, DIRT, 1, 11)]
        );
        assert_eq!(
            drop_position(&state.world.drops[0]),
            (9.5 * TILE_SIZE, 1.5 * TILE_SIZE)
        );
    }

    #[test]
    fn trash_destroys_items() {
        let mut state = fixture();
        give(&mut state, DIRT, 10);
        let events = apply(
            &mut state,
            ME,
            Action::Trash {
                item: DIRT,
                count: 4,
            },
            0.0,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![Event::InventoryChanged {
                player: ME,
                item: DIRT,
                count: 6
            }]
        );
        assert!(state.world.drops.is_empty());
    }

    #[test]
    fn at_the_drop_limit_breaks_succeed_without_a_drop_and_drop_actions_are_refused() {
        let mut state = fixture();
        give(&mut state, DIRT, 10);
        give(&mut state, STONE, 10);
        apply(
            &mut state,
            ME,
            Action::Drop {
                item: DIRT,
                count: 1,
                left: true,
            },
            0.0,
        )
        .unwrap();
        fill_drops(&mut state);

        let before = state.clone();
        let stone = Action::Drop {
            item: STONE,
            count: 1,
            left: true,
        };
        assert_eq!(apply(&mut state, ME, stone, 0.0), Err(Reject::DropLimit));
        assert_eq!(state, before);
        // Merging into the stack that is already there is always allowed.
        assert!(
            apply(
                &mut state,
                ME,
                Action::Drop {
                    item: DIRT,
                    count: 1,
                    left: true
                },
                0.0
            )
            .is_ok()
        );
        assert_eq!(state.world.drops.len(), MAX_DROPS_PER_WORLD);

        // Every break goes through, none leaves a drop, and the random stream keeps moving.
        let target = (12, 1);
        let mut gems_seen = false;
        for i in 0..200 {
            state.world.set(target.0, target.1, fg(SAND));
            let now = f64::from(i);
            punch_at(&mut state, target, now).unwrap();
            let rng_before = state.rng.clone();
            let events = punch_at(&mut state, target, now + 0.5).unwrap();
            assert_eq!(
                events[0],
                Event::TileChanged {
                    x: 12,
                    y: 1,
                    tile: Tile::default()
                }
            );
            assert!(dropped_items(&events).is_empty());
            assert_ne!(state.rng, rng_before);
            gems_seen |= events
                .iter()
                .any(|event| matches!(event, Event::GemsChanged { .. }));
        }
        assert!(gems_seen);
        assert_eq!(state.world.drops.len(), MAX_DROPS_PER_WORLD);
    }

    #[test]
    fn planting_needs_an_empty_tile_on_solid_ground() {
        let mut state = fixture();
        give(&mut state, DIRT_SEED, 3);
        // In mid-air, and above a tile that is not solid (the tree planted below).
        assert_eq!(
            place_at(&mut state, (11, 2), DIRT_SEED, 5.0),
            Err(Reject::NeedsGround)
        );
        let events = place_at(&mut state, PLOT, DIRT_SEED, 5.0).unwrap();
        let tree = *state.world.tree(PLOT.0, PLOT.1).unwrap();
        assert_eq!(
            (tree.item, tree.planted_at, tree.spliced),
            (DIRT, 5.0, false)
        );
        assert!((TREE_YIELD_MIN..=TREE_YIELD_MAX).contains(&tree.fruit));
        assert_eq!(
            events,
            vec![
                Event::TileChanged {
                    x: PLOT.0,
                    y: PLOT.1,
                    tile: fg(DIRT_SEED)
                },
                Event::TreePlanted {
                    x: PLOT.0,
                    y: PLOT.1,
                    tree
                },
                Event::InventoryChanged {
                    player: ME,
                    item: DIRT_SEED,
                    count: 2
                },
                Event::XpChanged {
                    player: ME,
                    level: 1,
                    xp: 1
                },
            ]
        );
        assert_eq!(
            place_at(&mut state, (11, 2), DIRT_SEED, 5.0),
            Err(Reject::NeedsGround)
        );
        assert_eq!(held(&state, DIRT_SEED), 2);
        assert_trees_match_tiles(&state);
        // Players walk through trees: the player's own tile works too.
        assert!(place_at(&mut state, HOME, DIRT_SEED, 5.0).is_ok());
    }

    #[test]
    fn fruit_is_rolled_uniformly_between_2_and_5_at_planting() {
        let mut state = fixture();
        give(&mut state, DIRT_SEED, 200);
        let mut seen = [0u32; 8];
        for _ in 0..200 {
            place_at(&mut state, PLOT, DIRT_SEED, 0.0).unwrap();
            seen[usize::from(state.world.tree(PLOT.0, PLOT.1).unwrap().fruit)] += 1;
            let tile = state.world.get(PLOT.0, PLOT.1).unwrap();
            clear_foreground(&mut state, PLOT.0, PLOT.1, tile);
        }
        assert_eq!(seen[0] + seen[1] + seen[6] + seen[7], 0);
        assert!(seen[2..=5].iter().all(|&n| n > 20), "{seen:?}");
    }

    #[test]
    fn splice_works_in_both_orders_and_keeps_the_fruit() {
        for (first, second) in [(DIRT_SEED, STONE_SEED), (STONE_SEED, DIRT_SEED)] {
            let mut state = fixture();
            give(&mut state, DIRT_SEED, 2);
            give(&mut state, STONE_SEED, 2);
            place_at(&mut state, PLOT, first, 5.0).unwrap();
            let sapling = *state.world.tree(PLOT.0, PLOT.1).unwrap();
            punch_at(&mut state, PLOT, 6.0).unwrap(); // one hit of damage, wiped by the splice

            let events = place_at(&mut state, PLOT, second, 50.0).unwrap();
            let tree = Tree {
                item: GRAVEL,
                planted_at: 50.0,
                fruit: sapling.fruit,
                spliced: true,
            };
            assert_eq!(state.world.tree(PLOT.0, PLOT.1), Some(&tree));
            assert_eq!(state.world.get(PLOT.0, PLOT.1), Some(fg(GRAVEL_SEED)));
            assert!(state.damage.is_empty());
            assert_eq!(
                events,
                vec![
                    Event::TileChanged {
                        x: PLOT.0,
                        y: PLOT.1,
                        tile: fg(GRAVEL_SEED)
                    },
                    Event::TreeSpliced {
                        x: PLOT.0,
                        y: PLOT.1,
                        tree,
                        a: seed_parent(first).unwrap(),
                        b: seed_parent(second).unwrap(),
                    },
                    Event::InventoryChanged {
                        player: ME,
                        item: second,
                        count: 1
                    },
                    // 1 for planting the first seed, 2 for the splice.
                    Event::XpChanged {
                        player: ME,
                        level: 1,
                        xp: 3
                    },
                ]
            );
            assert_eq!((held(&state, first), held(&state, second)), (1, 1));
            // The grow timer restarted and uses the result's rarity (2: 68 s).
            assert!(punch_at(&mut state, PLOT, 50.0 + 67.0).unwrap().contains(
                &Event::TileDamaged {
                    x: PLOT.0,
                    y: PLOT.1,
                    layer: Layer::Foreground,
                    damage: 1
                }
            ));
            assert_trees_match_tiles(&state);
        }
    }

    #[test]
    fn a_non_recipe_and_a_third_seed_consume_nothing() {
        let mut state = fixture();
        give(&mut state, DIRT_SEED, 2);
        give(&mut state, STONE_SEED, 2);
        give(&mut state, GRAVEL_SEED, 1);
        give(&mut state, SAND_SEED, 1);
        place_at(&mut state, PLOT, GRAVEL_SEED, 0.0).unwrap();
        let before = state.clone();
        // gravel + dirt is not a recipe; neither is gravel on gravel.
        assert_eq!(
            place_at(&mut state, PLOT, DIRT_SEED, 1.0),
            Err(Reject::NoRecipe)
        );
        give(&mut state, GRAVEL_SEED, 1);
        assert_eq!(
            place_at(&mut state, PLOT, GRAVEL_SEED, 1.0),
            Err(Reject::NoRecipe)
        );
        assert!(
            state
                .players
                .get_mut(&ME)
                .unwrap()
                .inventory
                .remove(GRAVEL_SEED, 1)
        );
        assert_eq!(state, before);

        // gravel + stone = stone bricks; after that the tree takes nothing more.
        assert!(place_at(&mut state, PLOT, STONE_SEED, 2.0).is_ok());
        assert_eq!(state.world.tree(PLOT.0, PLOT.1).unwrap().item, STONE_BRICKS);
        let before = state.clone();
        for third in [SAND_SEED, DIRT_SEED, STONE_SEED] {
            assert_eq!(
                place_at(&mut state, PLOT, third, 3.0),
                Err(Reject::AlreadySpliced)
            );
        }
        assert_eq!(state, before);
    }

    #[test]
    fn harvest_gives_exactly_the_stored_fruit_and_removes_the_tree() {
        let grow = grow_seconds(1) as f64;
        let (mut with_seed, mut without_seed) = (0, 0);
        for round in 0..200u32 {
            let mut state = fixture();
            state.rng = Rng::new(u64::from(round));
            give(&mut state, DIRT_SEED, 1);
            place_at(&mut state, PLOT, DIRT_SEED, 100.0).unwrap();
            let fruit = state.world.tree(PLOT.0, PLOT.1).unwrap().fruit;

            // Not grown yet: the punch only damages the sapling and yields nothing.
            let early = punch_at(&mut state, PLOT, 100.0 + grow - 1.0).unwrap();
            assert_eq!(
                early,
                vec![Event::TileDamaged {
                    x: PLOT.0,
                    y: PLOT.1,
                    layer: Layer::Foreground,
                    damage: 1
                }]
            );
            assert!(state.world.drops.is_empty() && state.world.tree(PLOT.0, PLOT.1).is_some());

            // Grown: one punch harvests, damage or not.
            let events = punch_at(&mut state, PLOT, 100.0 + grow).unwrap();
            assert_eq!(
                events[0],
                Event::TileChanged {
                    x: PLOT.0,
                    y: PLOT.1,
                    tile: Tile::default()
                }
            );
            assert_eq!(
                events[1],
                Event::TreeHarvested {
                    x: PLOT.0,
                    y: PLOT.1
                }
            );
            match dropped_items(&events).as_slice() {
                [(DIRT, n)] if *n == u16::from(fruit) => without_seed += 1,
                [(DIRT, n), (DIRT_SEED, 1)] if *n == u16::from(fruit) => with_seed += 1,
                other => panic!("fruit {fruit}, dropped {other:?}"),
            }
            assert!(
                state
                    .world
                    .drops
                    .iter()
                    .all(|drop| (drop.x, drop.y) == PLOT)
            );
            assert_eq!(state.world.get(PLOT.0, PLOT.1), Some(Tile::default()));
            assert!(state.world.extras.is_empty() && state.damage.is_empty());
            let gems = state.players[&ME].gems;
            assert!(gems <= 1, "rarity 1 harvest gave {gems}");
            assert_eq!(
                events.contains(&Event::GemsChanged { player: ME, gems }),
                gems > 0
            );
        }
        // TREE_SEED_CHANCE = 0.20 of 200 harvests.
        assert!(
            (20..=60).contains(&with_seed),
            "{with_seed} of {}",
            with_seed + without_seed
        );
    }

    #[test]
    fn a_sapling_is_destroyed_by_three_punches_and_gives_nothing() {
        let mut state = fixture();
        give(&mut state, DIRT_SEED, 1);
        place_at(&mut state, PLOT, DIRT_SEED, 0.0).unwrap();
        let rng = state.rng.clone();
        for (i, now) in [1.0, 2.0, 3.0].into_iter().enumerate() {
            let events = punch_at(&mut state, PLOT, now).unwrap();
            if usize::from(SAPLING_HITS) == i + 1 {
                assert_eq!(
                    events,
                    vec![Event::TileChanged {
                        x: PLOT.0,
                        y: PLOT.1,
                        tile: Tile::default()
                    }]
                );
            } else {
                assert_eq!(events.len(), 1);
            }
        }
        assert!(
            state.world.extras.is_empty()
                && state.world.drops.is_empty()
                && state.damage.is_empty()
        );
        assert_eq!(state.players[&ME].gems, 0);
        assert_eq!(state.rng, rng); // no roll happened
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn harvest_at_the_drop_limit_succeeds_and_loses_the_fruit() {
        let mut state = fixture();
        give(&mut state, DIRT_SEED, 1);
        place_at(&mut state, PLOT, DIRT_SEED, 0.0).unwrap();
        fill_drops(&mut state);
        let events = punch_at(&mut state, PLOT, 1000.0).unwrap();
        assert!(events.contains(&Event::TreeHarvested {
            x: PLOT.0,
            y: PLOT.1
        }));
        assert!(dropped_items(&events).is_empty());
        assert_eq!(state.world.drops.len(), MAX_DROPS_PER_WORLD);
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn gem_rolls_cover_the_whole_range_and_never_exceed_it() {
        let mut rng = Rng::new(11);
        for (rarity, divisor, max) in [
            (1, GEMS_BREAK_RARITY_DIVISOR, 1),
            (3, GEMS_BREAK_RARITY_DIVISOR, 1),
            (4, GEMS_BREAK_RARITY_DIVISOR, 2),
            (7, GEMS_BREAK_RARITY_DIVISOR, 2),
            (1, GEMS_HARVEST_RARITY_DIVISOR, 1),
            (6, GEMS_HARVEST_RARITY_DIVISOR, 4),
            (7, GEMS_HARVEST_RARITY_DIVISOR, 4),
            (0, GEMS_HARVEST_RARITY_DIVISOR, 1),
        ] {
            let rolls: Vec<u32> = (0..2000)
                .map(|_| roll_gems(&mut rng, rarity, divisor))
                .collect();
            assert_eq!(
                rolls.iter().max(),
                Some(&max),
                "rarity {rarity} / {divisor}"
            );
            assert_eq!(rolls.iter().min(), Some(&0), "rarity {rarity} / {divisor}");
            // Uniform: every value shows up about equally often.
            for value in 0..=max {
                let share =
                    rolls.iter().filter(|&&roll| roll == value).count() as f32 / rolls.len() as f32;
                assert!(
                    (share - 1.0 / (max + 1) as f32).abs() < 0.05,
                    "rarity {rarity}: {value} -> {share}"
                );
            }
        }
    }

    #[test]
    fn breaking_and_harvesting_credit_gems_to_the_player() {
        // Breaking crates (rarity 6): 0 ..= 1 + 6 / 4 = 2 per break, fg or bg alike.
        let mut state = fixture();
        let target = (12, 1);
        let mut seen = [false; 3];
        let mut total = 0;
        for i in 0..300 {
            state.world.drops.clear();
            state.damage.clear();
            state.world.set(target.0, target.1, fg(CRATE));
            state.damage.insert(
                World::tile_key(12, 1).unwrap(),
                Damage {
                    hits: 4,
                    layer: Layer::Foreground,
                    last_hit: f64::from(i),
                },
            );
            let events = punch_at(&mut state, target, f64::from(i)).unwrap();
            let gained = state.players[&ME].gems - total;
            assert!(gained <= 2, "{gained}");
            seen[gained as usize] = true;
            total += gained;
            let announced = events.contains(&Event::GemsChanged {
                player: ME,
                gems: total,
            });
            assert_eq!(announced, gained > 0);
        }
        assert_eq!(seen, [true; 3]);

        // A background break pays too.
        let before = state.players[&ME].gems;
        for i in 0..40 {
            state.world.set(
                target.0,
                target.1,
                Tile {
                    fg: EMPTY,
                    bg: CAVE_WALL,
                },
            );
            punch_at(&mut state, target, 1000.0 + f64::from(i)).unwrap();
            punch_at(&mut state, target, 1000.5 + f64::from(i)).unwrap();
        }
        assert!(state.players[&ME].gems > before);

        // Saturating: a full purse stays full and announces nothing.
        state.players.get_mut(&ME).unwrap().gems = u32::MAX;
        for i in 0..20 {
            state.world.set(
                target.0,
                target.1,
                Tile {
                    fg: EMPTY,
                    bg: CAVE_WALL,
                },
            );
            punch_at(&mut state, target, 2000.0 + f64::from(i)).unwrap();
            let events = punch_at(&mut state, target, 2000.5 + f64::from(i)).unwrap();
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, Event::GemsChanged { .. }))
            );
        }
        assert_eq!(state.players[&ME].gems, u32::MAX);
    }

    fn xp_of(state: &WorldState) -> (u16, u32) {
        let me = &state.players[&ME];
        (me.level, me.xp)
    }

    fn xp_events(events: &[Event]) -> Vec<Event> {
        let xp = |e: &&Event| matches!(e, Event::XpChanged { .. } | Event::LevelUp { .. });
        events.iter().filter(xp).cloned().collect()
    }

    /// Breaks `tile` at (12, 1) with one punch (damage preloaded) and returns the XP events.
    fn break_once(state: &mut WorldState, tile: Tile, layer: Layer, now: f64) -> Vec<Event> {
        let target = item(match layer {
            Layer::Foreground => tile.fg,
            Layer::Background => tile.bg,
        })
        .unwrap();
        state.world.set(12, 1, tile);
        state.damage.insert(
            World::tile_key(12, 1).unwrap(),
            Damage {
                hits: target.hits - 1,
                layer,
                last_hit: now,
            },
        );
        let events = punch_at(state, (12, 1), now).unwrap();
        assert_eq!(state.world.get(12, 1), Some(Tile::default()), "{events:?}");
        xp_events(&events)
    }

    #[test]
    fn xp_is_credited_per_the_table() {
        let mut state = fixture();
        let xp = |level, xp| Event::XpChanged {
            player: ME,
            level,
            xp,
        };
        // Breaking: max(1, rarity), foreground or background.
        assert_eq!(
            break_once(&mut state, fg(STONE), Layer::Foreground, 1.0),
            vec![xp(1, 1)]
        );
        assert_eq!(
            break_once(&mut state, fg(STONE_BRICKS), Layer::Foreground, 2.0),
            vec![xp(1, 4)]
        );
        let wall = Tile {
            fg: EMPTY,
            bg: CAVE_WALL,
        };
        assert_eq!(
            break_once(&mut state, wall, Layer::Background, 3.0),
            vec![xp(1, 5)]
        );

        // Planting 1, harvesting a rarity-2 tree (gravel) 2 × 2 = 4.
        give(&mut state, GRAVEL_SEED, 1);
        let events = place_at(&mut state, PLOT, GRAVEL_SEED, 10.0).unwrap();
        assert_eq!(xp_events(&events), vec![xp(1, 6)]);
        let grown = 10.0 + grow_seconds(2) as f64 + 1.0;
        let events = punch_at(&mut state, PLOT, grown).unwrap();
        assert!(events.contains(&Event::TreeHarvested {
            x: PLOT.0,
            y: PLOT.1
        }));
        assert_eq!(xp_events(&events), vec![xp(1, 10)]);

        // Splicing 2 (on top of the 1 for planting the first seed).
        give(&mut state, DIRT_SEED, 2);
        give(&mut state, STONE_SEED, 1);
        let at = grown + 1.0;
        place_at(&mut state, PLOT, DIRT_SEED, at).unwrap();
        let events = place_at(&mut state, PLOT, STONE_SEED, at).unwrap();
        assert_eq!(xp_events(&events), vec![xp(1, 13)]);

        // Destroying a sapling gives nothing.
        let (x, y) = (PLOT.0 + 1, PLOT.1);
        place_at(&mut state, (x, y), DIRT_SEED, at).unwrap();
        let before = xp_of(&state);
        for i in 0..SAPLING_HITS {
            let events = punch_at(&mut state, (x, y), at + 1.0 + f64::from(i)).unwrap();
            assert_eq!(xp_events(&events), vec![]);
        }
        assert_eq!(state.world.tree(x, y), None);
        assert_eq!(xp_of(&state), before);

        // Breaking a claim gives nothing.
        let claim = (12, 1);
        give(&mut state, SMALL_CLAIM, 1);
        place_at(&mut state, claim, SMALL_CLAIM, 100.0).unwrap();
        for i in 0..8 {
            let events = punch_at(&mut state, claim, 101.0 + f64::from(i) * 0.5).unwrap();
            assert_eq!(xp_events(&events), vec![]);
        }
        assert_eq!(state.world.get(claim.0, claim.1), Some(Tile::default()));
        assert_eq!(xp_of(&state), before);
    }

    #[test]
    fn a_level_up_emits_one_level_up_with_the_final_values() {
        let mut state = fixture();
        state.players.get_mut(&ME).unwrap().xp = 99;
        // Crate, rarity 6: 99 + 6 = 105 → level 2 with 5 carried over.
        let events = break_once(&mut state, fg(CRATE), Layer::Foreground, 1.0);
        assert_eq!(
            events,
            vec![
                Event::XpChanged {
                    player: ME,
                    level: 2,
                    xp: 5
                },
                Event::LevelUp {
                    player: ME,
                    level: 2
                },
            ]
        );
        assert_eq!(xp_of(&state), (2, 5));

        // At the cap nothing changes and nothing is announced.
        let me = state.players.get_mut(&ME).unwrap();
        (me.level, me.xp) = (crate::levels::MAX_LEVEL, 0);
        assert_eq!(
            break_once(&mut state, fg(CRATE), Layer::Foreground, 2.0),
            vec![]
        );
        assert_eq!(xp_of(&state), (crate::levels::MAX_LEVEL, 0));
    }

    #[test]
    fn no_seed_tile_without_a_tree_and_no_tree_without_its_tile() {
        let mut state = fixture();
        for seed in [DIRT_SEED, STONE_SEED, SAND_SEED] {
            give(&mut state, seed, 20);
        }
        give(&mut state, DIRT, 20);
        give(&mut state, CAVE_WALL, 20);
        let floor = (11, 0);
        let script: Vec<(Action, f64)> = vec![
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: DIRT_SEED,
                },
                0.0,
            ), // plant
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: CAVE_WALL,
                },
                0.0,
            ), // wall behind a tree
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: DIRT,
                },
                0.0,
            ), // refused: tree in the way
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: STONE_SEED,
                },
                1.0,
            ), // splice
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: SAND_SEED,
                },
                2.0,
            ), // refused: already spliced
            (Action::Punch { x: 11, y: 1 }, 3.0), // sapling damage
            (Action::Punch { x: 11, y: 1 }, 4.0),
            (Action::Punch { x: 11, y: 1 }, 5.0), // destroyed
            (Action::Punch { x: 11, y: 1 }, 6.0), // now hits the wall behind
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: SAND_SEED,
                },
                7.0,
            ), // plant again
            (
                Action::Place {
                    x: 12,
                    y: 1,
                    item: DIRT_SEED,
                },
                7.0,
            ),
            (
                Action::Place {
                    x: 9,
                    y: 1,
                    item: DIRT_SEED,
                },
                7.0,
            ),
            (
                Action::Place {
                    x: 12,
                    y: 2,
                    item: DIRT,
                },
                8.0,
            ), // a block right above a tree
            (Action::Punch { x: 12, y: 1 }, 9.0), // one hit on a sapling, left to expire
            (Action::Punch { x: 11, y: 1 }, 100.0), // harvest (sand: 31 s)
            (Action::Punch { x: 11, y: 1 }, 101.0), // the wall behind takes the next punches
            (Action::Punch { x: 11, y: 1 }, 102.0),
            (Action::Punch { x: 11, y: 1 }, 103.0), // miss
            (
                Action::Place {
                    x: 11,
                    y: 1,
                    item: DIRT,
                },
                104.0,
            ), // a block where the tree was
            (
                Action::Place {
                    x: 11,
                    y: 2,
                    item: STONE_SEED,
                },
                105.0,
            ), // plant on that block
            (Action::Punch { x: 11, y: 1 }, 106.0), // break the block under the tree...
            (Action::Punch { x: 11, y: 1 }, 107.0),
            (Action::Punch { x: 11, y: 1 }, 108.0), // ...the tree stays
            (
                Action::Place {
                    x: 11,
                    y: 2,
                    item: DIRT_SEED,
                },
                109.0,
            ), // splice it in mid-air
            (Action::Punch { x: 12, y: 1 }, 500.0), // harvests
            (Action::Punch { x: 9, y: 1 }, 501.0),
            (Action::Punch { x: 11, y: 2 }, 502.0),
            (
                Action::Drop {
                    item: DIRT_SEED,
                    count: 3,
                    left: true,
                },
                503.0,
            ), // seeds on the ground are not trees
            (
                Action::Trash {
                    item: STONE_SEED,
                    count: 2,
                },
                504.0,
            ),
        ];
        let mut accepted = 0;
        for (step, (action, now)) in script.into_iter().enumerate() {
            accepted += u32::from(apply(&mut state, ME, action, now).is_ok());
            assert_trees_match_tiles(&state);
            collect_drops(&mut state, ME);
            assert_trees_match_tiles(&state);
            assert!(
                state.world.get(floor.0, floor.1) == Some(fg(BEDROCK)),
                "step {step}"
            );
        }
        assert_eq!(
            accepted, 27,
            "the script should hit both accepted and rejected paths"
        );
        assert!(state.world.extras.is_empty());
        assert_eq!(state.world.tree(11, 2), None);
    }

    const OTHER: PlayerId = PlayerId(2);
    const STRANGER: PlayerId = PlayerId(3);
    /// Where OTHER and STRANGER stand in `claim_fixture`: outside ME's small claim at `PLOT`, in reach of its edge.
    const OUTSIDE: (u32, u32) = (13, 1);

    /// `fixture` plus two more players at `OUTSIDE`, and building material for everyone.
    fn claim_fixture() -> WorldState {
        let mut state = fixture();
        for id in [OTHER, STRANGER] {
            state.join(id);
            stand_on(&mut state, id, OUTSIDE);
        }
        for player in state.players.values_mut() {
            player.inventory.add(DIRT, 50);
            player.inventory.add(CAVE_WALL, 50);
        }
        give(&mut state, SMALL_CLAIM, 5);
        state
    }

    fn act(
        state: &mut WorldState,
        id: PlayerId,
        action: Action,
        now: f64,
    ) -> Result<Vec<Event>, Reject> {
        apply(state, id, action, now)
    }

    fn claim_at(state: &WorldState, (x, y): (u32, u32)) -> Claim {
        state
            .world
            .claim(World::tile_key(x, y).unwrap())
            .unwrap()
            .clone()
    }

    #[test]
    fn placing_a_claim_takes_its_tiles_and_reports_them() {
        for (item, tiles) in [(SMALL_CLAIM, 10), (MEDIUM_CLAIM, 48), (LARGE_CLAIM, 200)] {
            let mut state = fixture();
            give(&mut state, item, 1);
            let events = place_at(&mut state, PLOT, item, 0.0).unwrap();
            let claim = claim_at(&state, PLOT);
            assert_eq!(
                (claim.owner, claim.public, claim.access.len()),
                (ME, false, 0)
            );
            assert_eq!(claim.tiles.len(), tiles);
            assert_eq!(claim.tiles[0], World::tile_key(PLOT.0, PLOT.1).unwrap());
            assert_eq!(state.world.claim_of.len(), tiles);
            assert_eq!(
                events,
                vec![
                    Event::TileChanged {
                        x: PLOT.0,
                        y: PLOT.1,
                        tile: fg(item)
                    },
                    Event::ClaimPlaced {
                        x: PLOT.0,
                        y: PLOT.1,
                        claim
                    },
                    Event::InventoryChanged {
                        player: ME,
                        item,
                        count: 0
                    },
                ]
            );
            assert_trees_match_tiles(&state);
        }
    }

    #[test]
    fn a_claim_admits_its_owner_then_the_access_list_then_everyone() {
        let mut state = claim_fixture();
        place_at(&mut state, PLOT, SMALL_CLAIM, 0.0).unwrap();
        state.world.set(12, 1, fg(DIRT)); // a block inside the claim
        let punch = Action::Punch { x: 12, y: 1 };
        let place = Action::Place {
            x: 12,
            y: 2,
            item: DIRT,
        };
        let wall = Action::Place {
            x: 12,
            y: 2,
            item: CAVE_WALL,
        };

        // Strangers: nothing works inside, everything works just outside.
        for id in [OTHER, STRANGER] {
            for action in [punch, place, wall] {
                assert_eq!(
                    act(&mut state, id, action, 1.0),
                    Err(Reject::NoPermission),
                    "{action:?}"
                );
            }
        }
        assert!(
            act(
                &mut state,
                OTHER,
                Action::Place {
                    x: 14,
                    y: 3,
                    item: DIRT
                },
                1.0
            )
            .is_ok()
        );
        // The owner's punch and placement succeed.
        assert!(act(&mut state, ME, punch, 1.0).is_ok());
        assert!(act(&mut state, ME, wall, 1.0).is_ok());

        // Access list.
        let grant = Action::SetClaimAccess {
            x: PLOT.0,
            y: PLOT.1,
            player: OTHER,
            allow: true,
        };
        let events = act(&mut state, ME, grant, 2.0).unwrap();
        let claim = claim_at(&state, PLOT);
        assert_eq!(claim.access, vec![OTHER]);
        assert_eq!(
            events,
            vec![Event::ClaimUpdated {
                x: PLOT.0,
                y: PLOT.1,
                claim
            }]
        );
        assert_eq!(act(&mut state, ME, grant, 2.0), Ok(Vec::new())); // already listed: nothing changes
        assert!(act(&mut state, OTHER, punch, 2.0).is_ok());
        assert!(act(&mut state, OTHER, place, 2.0).is_ok());
        assert_eq!(
            act(&mut state, STRANGER, punch, 2.0),
            Err(Reject::NoPermission)
        );

        // Public.
        let open = Action::SetClaimPublic {
            x: PLOT.0,
            y: PLOT.1,
            public: true,
        };
        assert_eq!(act(&mut state, ME, open, 3.0).unwrap().len(), 1);
        assert!(act(&mut state, STRANGER, punch, 3.0).is_ok());
        assert!(
            act(
                &mut state,
                STRANGER,
                Action::Place {
                    x: 12,
                    y: 0,
                    item: CAVE_WALL
                },
                3.0
            )
            .is_ok()
        );

        // Closed again and OTHER taken off the list: both are locked out once more.
        let close = Action::SetClaimPublic {
            x: PLOT.0,
            y: PLOT.1,
            public: false,
        };
        let revoke = Action::SetClaimAccess {
            x: PLOT.0,
            y: PLOT.1,
            player: OTHER,
            allow: false,
        };
        act(&mut state, ME, close, 4.0).unwrap();
        act(&mut state, ME, revoke, 4.0).unwrap();
        for id in [OTHER, STRANGER] {
            assert_eq!(act(&mut state, id, punch, 5.0), Err(Reject::NoPermission));
        }
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn trees_inside_a_claim_are_protected_but_drops_are_not() {
        let mut state = claim_fixture();
        place_at(&mut state, PLOT, SMALL_CLAIM, 0.0).unwrap();
        give(&mut state, DIRT_SEED, 2);
        state
            .players
            .get_mut(&OTHER)
            .unwrap()
            .inventory
            .add(STONE_SEED, 5);
        place_at(&mut state, (12, 1), DIRT_SEED, 0.0).unwrap();
        let before = state.clone();
        // No splicing, no sapling damage, no harvest, no planting by a stranger.
        let tree = (12, 1);
        assert_eq!(
            act(
                &mut state,
                OTHER,
                Action::Place {
                    x: 12,
                    y: 1,
                    item: STONE_SEED
                },
                1.0
            ),
            Err(Reject::NoPermission)
        );
        assert_eq!(
            act(
                &mut state,
                OTHER,
                Action::Punch {
                    x: tree.0,
                    y: tree.1
                },
                1.0
            ),
            Err(Reject::NoPermission)
        );
        assert_eq!(
            act(
                &mut state,
                OTHER,
                Action::Punch {
                    x: tree.0,
                    y: tree.1
                },
                1000.0
            ),
            Err(Reject::NoPermission)
        );
        assert_eq!(
            act(
                &mut state,
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: STONE_SEED
                },
                1.0
            ),
            Err(Reject::NoPermission)
        );
        assert_eq!(state, before);

        // The owner harvests; the fruit lies on a claimed tile and the stranger may walk in and take it.
        let events = punch_at(&mut state, tree, 1000.0).unwrap();
        assert!(!dropped_items(&events).is_empty());
        stand_on(&mut state, OTHER, tree);
        assert!(!collect_drops(&mut state, OTHER).is_empty());
        assert!(state.world.drops.is_empty());
    }

    #[test]
    fn only_the_owner_breaks_a_claim_and_it_always_drops_itself() {
        let hits = item(SMALL_CLAIM).unwrap().hits;
        let mut state = claim_fixture();
        place_at(&mut state, PLOT, SMALL_CLAIM, 0.0).unwrap();
        let block = Action::Punch {
            x: PLOT.0,
            y: PLOT.1,
        };
        // A stranger is locked out; a player on the access list may build here but not remove the claim.
        assert_eq!(
            act(&mut state, OTHER, block, 1.0),
            Err(Reject::NoPermission)
        );
        act(
            &mut state,
            ME,
            Action::SetClaimAccess {
                x: PLOT.0,
                y: PLOT.1,
                player: OTHER,
                allow: true,
            },
            1.0,
        )
        .unwrap();
        assert_eq!(act(&mut state, OTHER, block, 1.0), Err(Reject::NotOwner));
        let take_over = Action::SetClaimPublic {
            x: PLOT.0,
            y: PLOT.1,
            public: true,
        };
        assert_eq!(
            act(&mut state, OTHER, take_over, 1.0),
            Err(Reject::NotOwner)
        );
        let invite = Action::SetClaimAccess {
            x: PLOT.0,
            y: PLOT.1,
            player: STRANGER,
            allow: true,
        };
        assert_eq!(act(&mut state, OTHER, invite, 1.0), Err(Reject::NotOwner));
        act(
            &mut state,
            ME,
            Action::SetClaimAccess {
                x: PLOT.0,
                y: PLOT.1,
                player: OTHER,
                allow: false,
            },
            1.0,
        )
        .unwrap();

        let rng = state.rng.clone();
        for hit in 1..hits {
            let events = act(&mut state, ME, block, f64::from(hit)).unwrap();
            assert_eq!(
                events,
                vec![Event::TileDamaged {
                    x: PLOT.0,
                    y: PLOT.1,
                    layer: Layer::Foreground,
                    damage: hit
                }]
            );
        }
        let events = act(&mut state, ME, block, f64::from(hits)).unwrap();
        let drop = Drop {
            id: 0,
            item: SMALL_CLAIM,
            count: 1,
            x: PLOT.0,
            y: PLOT.1,
        };
        assert_eq!(
            events,
            vec![
                Event::TileChanged {
                    x: PLOT.0,
                    y: PLOT.1,
                    tile: Tile::default()
                },
                Event::ClaimRemoved {
                    x: PLOT.0,
                    y: PLOT.1
                },
                Event::ItemDropped { drop },
            ]
        );
        // Exactly one claim item, no roll, no gems; the tiles are free again.
        assert_eq!(state.world.drops, vec![drop]);
        assert_eq!(state.rng, rng);
        assert_eq!(state.players[&ME].gems, 0);
        assert!(state.world.claim_of.is_empty() && state.world.extras.is_empty());
        assert!(
            act(
                &mut state,
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: DIRT
                },
                20.0
            )
            .is_ok()
        );
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn a_claim_is_not_lost_to_the_drop_limit() {
        let hits = item(SMALL_CLAIM).unwrap().hits;
        let mut state = claim_fixture();
        place_at(&mut state, PLOT, SMALL_CLAIM, 0.0).unwrap();
        for hit in 1..hits {
            punch_at(&mut state, PLOT, f64::from(hit)).unwrap();
        }
        fill_drops(&mut state);
        let before = state.clone();
        assert_eq!(
            punch_at(&mut state, PLOT, f64::from(hits)),
            Err(Reject::DropLimit)
        );
        assert_eq!(state, before);
        state.world.drops.pop();
        assert!(punch_at(&mut state, PLOT, f64::from(hits)).is_ok());
        assert_eq!(
            state.world.drops.last().map(|drop| drop.item),
            Some(SMALL_CLAIM)
        );
    }

    #[test]
    fn adjacent_claims_of_two_players_stay_disjoint() {
        let mut state = claim_fixture();
        state
            .players
            .get_mut(&OTHER)
            .unwrap()
            .inventory
            .add(SMALL_CLAIM, 1);
        place_at(&mut state, PLOT, SMALL_CLAIM, 0.0).unwrap();
        // (13, 1) is OTHER's own tile and (14, 1) is free ground right next to ME's claim.
        act(
            &mut state,
            OTHER,
            Action::Place {
                x: 14,
                y: 1,
                item: SMALL_CLAIM,
            },
            0.0,
        )
        .unwrap();
        let (mine, theirs) = (claim_at(&state, PLOT), claim_at(&state, (14, 1)));
        assert_eq!((mine.tiles.len(), theirs.tiles.len()), (10, 10));
        assert!(mine.tiles.iter().all(|tile| !theirs.tiles.contains(tile)));
        assert_eq!(state.world.claim_of.len(), 20);
        // Inside someone else's claim there is no placing a claim either.
        state
            .players
            .get_mut(&OTHER)
            .unwrap()
            .inventory
            .add(SMALL_CLAIM, 1);
        assert_eq!(
            act(
                &mut state,
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: SMALL_CLAIM
                },
                0.0
            ),
            Err(Reject::NoPermission)
        );
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn world_claim_rules() {
        let mut state = claim_fixture();
        give(&mut state, WORLD_CLAIM, 2);
        let others = &mut state.players.get_mut(&OTHER).unwrap().inventory;
        others.add(SMALL_CLAIM, 2);
        others.add(WORLD_CLAIM, 1);
        let their_plot = Action::Place {
            x: 14,
            y: 1,
            item: SMALL_CLAIM,
        };
        act(&mut state, OTHER, their_plot, 0.0).unwrap();

        // Someone else owns an area claim: the world cannot be claimed.
        let before = state.clone();
        assert_eq!(
            place_at(&mut state, PLOT, WORLD_CLAIM, 0.0),
            Err(Reject::ForeignClaims)
        );
        assert_eq!(state, before);
        // Their own area claim does not stop them, but give the world to ME instead: remove theirs first.
        let hits = item(SMALL_CLAIM).unwrap().hits;
        for hit in 1..=hits {
            act(
                &mut state,
                OTHER,
                Action::Punch { x: 14, y: 1 },
                f64::from(hit),
            )
            .unwrap();
        }
        stand_on(&mut state, OTHER, (14, 1));
        collect_drops(&mut state, OTHER);
        stand_on(&mut state, OTHER, OUTSIDE);

        let events = place_at(&mut state, PLOT, WORLD_CLAIM, 20.0).unwrap();
        let claim = claim_at(&state, PLOT);
        assert!(claim.tiles.is_empty() && state.world.claim_of.is_empty());
        assert_eq!(state.world.world_claim, World::tile_key(PLOT.0, PLOT.1));
        assert!(events.contains(&Event::ClaimPlaced {
            x: PLOT.0,
            y: PLOT.1,
            claim
        }));

        // One world claim per world, for the owner and for everyone else.
        let before = state.clone();
        assert_eq!(
            place_at(&mut state, (12, 2), WORLD_CLAIM, 20.0),
            Err(Reject::WorldAlreadyClaimed)
        );
        assert_eq!(state, before);

        // A stranger cannot edit any tile...
        state.world.set(14, 2, fg(DIRT));
        let attempts = [
            Action::Punch { x: 14, y: 2 },
            Action::Punch { x: 14, y: 3 },
            Action::Place {
                x: 15,
                y: 3,
                item: DIRT,
            },
            Action::Place {
                x: 15,
                y: 3,
                item: CAVE_WALL,
            },
            Action::Place {
                x: 15,
                y: 3,
                item: WORLD_CLAIM,
            },
            their_plot,
        ];
        for action in attempts {
            assert_eq!(
                act(&mut state, OTHER, action, 30.0),
                Err(Reject::NoPermission),
                "{action:?}"
            );
        }
        for (x, y) in [(0, 0), (50, 25), (WORLD_WIDTH - 1, WORLD_HEIGHT - 1), DOOR] {
            assert!(!can_edit(
                &state.world,
                OTHER,
                World::tile_key(x, y).unwrap()
            ));
        }
        // ...but still uses the main door.
        stand_on(&mut state, OTHER, (DOOR.0 - 1, DOOR.1));
        assert_eq!(
            act(
                &mut state,
                OTHER,
                Action::Punch {
                    x: DOOR.0,
                    y: DOOR.1
                },
                30.0
            ),
            Ok(vec![Event::LeftWorld { player: OTHER }])
        );
        stand_on(&mut state, OTHER, OUTSIDE);

        // An area claim inside the claimed world overrides the world claim for its tiles: the world
        // owner lets a friend in, the friend claims a plot, and from then on the plot is the friend's.
        act(
            &mut state,
            ME,
            Action::SetClaimAccess {
                x: PLOT.0,
                y: PLOT.1,
                player: OTHER,
                allow: true,
            },
            40.0,
        )
        .unwrap();
        act(&mut state, OTHER, their_plot, 40.0).unwrap();
        act(
            &mut state,
            ME,
            Action::SetClaimAccess {
                x: PLOT.0,
                y: PLOT.1,
                player: OTHER,
                allow: false,
            },
            40.0,
        )
        .unwrap();
        stand_on(&mut state, ME, (12, 1));
        let inside = Action::Place {
            x: 13,
            y: 2,
            item: CAVE_WALL,
        }; // in OTHER's plot, in reach of both
        assert_eq!(act(&mut state, ME, inside, 41.0), Err(Reject::NoPermission));
        assert!(act(&mut state, OTHER, inside, 41.0).is_ok());
        let outside_plot = Action::Place {
            x: 11,
            y: 3,
            item: CAVE_WALL,
        };
        assert_eq!(
            act(&mut state, OTHER, outside_plot, 41.0),
            Err(Reject::NoPermission)
        );
        assert!(act(&mut state, ME, outside_plot, 41.0).is_ok());
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn the_store_sells_claims_for_their_exact_price() {
        let mut state = fixture();
        let buy = |item| Action::Buy { item };
        let before = state.clone();
        assert_eq!(
            apply(&mut state, ME, buy(SMALL_CLAIM), 0.0),
            Err(Reject::NotEnoughGems)
        );
        state.players.get_mut(&ME).unwrap().gems = 49;
        assert_eq!(
            apply(&mut state, ME, buy(SMALL_CLAIM), 0.0),
            Err(Reject::NotEnoughGems)
        );
        state.players.get_mut(&ME).unwrap().gems = 0;
        assert_eq!(state, before);

        state.players.get_mut(&ME).unwrap().gems = 2800;
        for (item, left) in [
            (SMALL_CLAIM, 2750),
            (MEDIUM_CLAIM, 2550),
            (LARGE_CLAIM, 2050),
            (WORLD_CLAIM, 50),
        ] {
            let events = apply(&mut state, ME, buy(item), 0.0).unwrap();
            assert_eq!(
                events,
                vec![
                    Event::GemsChanged {
                        player: ME,
                        gems: left
                    },
                    Event::InventoryChanged {
                        player: ME,
                        item,
                        count: 1
                    },
                ]
            );
            assert_eq!((state.players[&ME].gems, held(&state, item)), (left, 1));
        }
        let events = apply(&mut state, ME, buy(SMALL_CLAIM), 0.0).unwrap();
        assert!(events.contains(&Event::InventoryChanged {
            player: ME,
            item: SMALL_CLAIM,
            count: 2
        }));
        assert_eq!(state.players[&ME].gems, 0);

        // Only what is on the price list, and only with room for it.
        let mut state = fixture();
        let player = state.players.get_mut(&ME).unwrap();
        player.gems = 10_000;
        // The starting items fill it.
        player.inventory.capacity = player.inventory.slots().len() as u16;
        let before = state.clone();
        for (item, reject) in [
            (SMALL_CLAIM, Reject::InventoryFull),
            (DIRT, Reject::Invalid),
            (FIST, Reject::Invalid),
            (EMPTY, Reject::Invalid),
            (ItemId(u16::MAX), Reject::Invalid),
        ] {
            assert_eq!(
                apply(&mut state, ME, buy(item), 0.0),
                Err(reject),
                "{item:?}"
            );
        }
        assert_eq!(
            apply(&mut state, PlayerId(99), buy(SMALL_CLAIM), 0.0),
            Err(Reject::Invalid)
        );
        assert_eq!(state, before);
        // A full stack is no room either.
        state.players.get_mut(&ME).unwrap().inventory.capacity += 1;
        give(&mut state, SMALL_CLAIM, MAX_STACK);
        assert_eq!(
            apply(&mut state, ME, buy(SMALL_CLAIM), 0.0),
            Err(Reject::InventoryFull)
        );
    }

    #[test]
    fn backpack_upgrades_add_ten_slots_up_to_38_times() {
        let mut state = fixture();
        let before = state.clone();
        assert_eq!(
            apply(&mut state, ME, Action::BuyBackpackUpgrade, 0.0),
            Err(Reject::NotEnoughGems)
        );
        assert_eq!(state, before);

        state.players.get_mut(&ME).unwrap().gems = 1000;
        let events = apply(&mut state, ME, Action::BuyBackpackUpgrade, 0.0).unwrap();
        let cost = upgrade_cost(0);
        assert_eq!(
            events,
            vec![
                Event::GemsChanged {
                    player: ME,
                    gems: 1000 - cost
                },
                Event::BackpackUpgraded {
                    player: ME,
                    capacity: 26
                },
            ]
        );
        assert_eq!(state.players[&ME].inventory.capacity, STARTING_SLOTS + 10);
        assert_eq!(state.players[&ME].gems, 1000 - cost);

        // All 38, each for its own price; the 39th is rejected.
        state.players.get_mut(&ME).unwrap().gems = u32::MAX;
        let mut spent = 0u64;
        for bought in 1..MAX_UPGRADES {
            assert!(apply(&mut state, ME, Action::BuyBackpackUpgrade, 0.0).is_ok());
            spent += u64::from(upgrade_cost(bought));
        }
        let player = &state.players[&ME];
        assert_eq!(
            (player.inventory.capacity, player.inventory.upgrades()),
            (396, MAX_UPGRADES)
        );
        assert_eq!(u64::from(u32::MAX - player.gems), spent);
        let before = state.clone();
        assert_eq!(
            apply(&mut state, ME, Action::BuyBackpackUpgrade, 0.0),
            Err(Reject::Invalid)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn claim_and_store_rejects_leave_the_state_unchanged() {
        let mut state = claim_fixture();
        give(&mut state, WORLD_CLAIM, 1);
        place_at(&mut state, PLOT, SMALL_CLAIM, 0.0).unwrap();
        state.world.set(12, 1, fg(DIRT));
        let others = &mut state.players.get_mut(&OTHER).unwrap().inventory;
        others.add(SMALL_CLAIM, 1);
        others.add(WORLD_CLAIM, 1);
        others.add(DIRT_SEED, 1);
        // The access list is full of other people.
        for n in 0..MAX_CLAIM_ACCESS as u32 {
            let listed = Action::SetClaimAccess {
                x: PLOT.0,
                y: PLOT.1,
                player: PlayerId(100 + n),
                allow: true,
            };
            apply(&mut state, ME, listed, 0.0).unwrap();
        }
        // STRANGER can pay for anything but has no free slot.
        let stranger = state.players.get_mut(&STRANGER).unwrap();
        stranger.gems = 10_000;
        stranger.inventory.capacity = stranger.inventory.slots().len() as u16;
        let far = u32::MAX;
        let set_public = |x, y| Action::SetClaimPublic { x, y, public: true };
        let allow = |x, y, player| Action::SetClaimAccess {
            x,
            y,
            player,
            allow: true,
        };
        let cases = [
            (OTHER, Action::Punch { x: 12, y: 1 }, Reject::NoPermission),
            (OTHER, Action::Punch { x: 12, y: 2 }, Reject::NoPermission), // an empty protected tile
            (
                OTHER,
                Action::Punch {
                    x: PLOT.0,
                    y: PLOT.1,
                },
                Reject::NoPermission,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: DIRT,
                },
                Reject::NoPermission,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: CAVE_WALL,
                },
                Reject::NoPermission,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: SMALL_CLAIM,
                },
                Reject::NoPermission,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: DIRT_SEED,
                },
                Reject::NoPermission,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: FIST,
                },
                Reject::NoPermission,
            ), // nothing is revealed first
            (
                OTHER,
                Action::Place {
                    x: 14,
                    y: 2,
                    item: WORLD_CLAIM,
                },
                Reject::ForeignClaims,
            ),
            (OTHER, set_public(PLOT.0, PLOT.1), Reject::NotOwner),
            (OTHER, allow(PLOT.0, PLOT.1, OTHER), Reject::NotOwner),
            (STRANGER, allow(PLOT.0, PLOT.1, STRANGER), Reject::NotOwner),
            (ME, allow(PLOT.0, PLOT.1, STRANGER), Reject::Invalid), // the list is full
            (ME, allow(PLOT.0, PLOT.1, ME), Reject::Invalid),       // the owner needs no entry
            (ME, set_public(12, 1), Reject::Invalid),               // not a claim block
            (ME, set_public(12, 3), Reject::Invalid),               // an empty tile
            (ME, set_public(far, far), Reject::OutOfRange),
            (ME, allow(far, 0, OTHER), Reject::OutOfRange),
            (PlayerId(99), set_public(PLOT.0, PLOT.1), Reject::Invalid),
            (
                ME,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: MEDIUM_CLAIM,
                },
                Reject::NoItem,
            ),
            (
                ME,
                Action::Place {
                    x: 12,
                    y: 1,
                    item: SMALL_CLAIM,
                },
                Reject::Occupied,
            ),
            (
                ME,
                Action::Place {
                    x: HOME.0,
                    y: HOME.1,
                    item: SMALL_CLAIM,
                },
                Reject::Occupied,
            ), // on a player
            (ME, Action::Buy { item: WORLD_CLAIM }, Reject::NotEnoughGems),
            (ME, Action::Buy { item: DIRT }, Reject::Invalid),
            (
                STRANGER,
                Action::Buy { item: SMALL_CLAIM },
                Reject::InventoryFull,
            ),
            (ME, Action::BuyBackpackUpgrade, Reject::NotEnoughGems),
            (PlayerId(99), Action::BuyBackpackUpgrade, Reject::Invalid),
        ];
        let before = state.clone();
        for (player, action, reject) in cases {
            assert_eq!(
                apply(&mut state, player, action, 50.0),
                Err(reject),
                "{player:?} {action:?}"
            );
            assert_eq!(state, before, "{action:?} changed the state");
        }

        // Out of reach of the claim block, even its owner cannot change it.
        stand_on(&mut state, ME, (PLOT.0 + 3, 1));
        let before = state.clone();
        assert_eq!(
            apply(&mut state, ME, set_public(PLOT.0, PLOT.1), 50.0),
            Err(Reject::OutOfRange)
        );
        assert_eq!(state, before);
        stand_on(&mut state, ME, HOME);

        // A second world claim, by its owner or anyone else.
        apply(
            &mut state,
            ME,
            Action::SetClaimAccess {
                x: PLOT.0,
                y: PLOT.1,
                player: PlayerId(100),
                allow: false,
            },
            50.0,
        )
        .unwrap();
        place_at(&mut state, (9, 2), WORLD_CLAIM, 50.0).unwrap();
        give(&mut state, WORLD_CLAIM, 1);
        apply(
            &mut state,
            ME,
            Action::SetClaimPublic {
                x: 9,
                y: 2,
                public: true,
            },
            50.0,
        )
        .unwrap();
        let before = state.clone();
        assert_eq!(
            place_at(&mut state, (9, 3), WORLD_CLAIM, 50.0),
            Err(Reject::WorldAlreadyClaimed)
        );
        assert_eq!(
            apply(
                &mut state,
                OTHER,
                Action::Place {
                    x: 14,
                    y: 2,
                    item: WORLD_CLAIM
                },
                50.0
            ),
            Err(Reject::WorldAlreadyClaimed)
        );
        assert_eq!(state, before);
        assert_trees_match_tiles(&state);
    }

    #[test]
    fn claims_and_their_index_stay_in_step_through_a_scripted_session() {
        let hits = item(SMALL_CLAIM).unwrap().hits;
        let mut state = claim_fixture();
        give(&mut state, MEDIUM_CLAIM, 1);
        give(&mut state, WORLD_CLAIM, 1);
        give(&mut state, DIRT_SEED, 3);
        state
            .players
            .get_mut(&OTHER)
            .unwrap()
            .inventory
            .add(SMALL_CLAIM, 1);
        let mut script: Vec<(PlayerId, Action, f64)> = vec![
            (
                ME,
                Action::Place {
                    x: 11,
                    y: 1,
                    item: SMALL_CLAIM,
                },
                0.0,
            ),
            (
                ME,
                Action::Place {
                    x: 12,
                    y: 1,
                    item: DIRT_SEED,
                },
                0.0,
            ), // a tree inside the claim
            (
                OTHER,
                Action::Place {
                    x: 15,
                    y: 1,
                    item: SMALL_CLAIM,
                },
                0.0,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: DIRT,
                },
                0.0,
            ), // refused
            (
                ME,
                Action::Place {
                    x: 9,
                    y: 1,
                    item: MEDIUM_CLAIM,
                },
                0.0,
            ), // bends around the first
            (
                ME,
                Action::SetClaimAccess {
                    x: 11,
                    y: 1,
                    player: OTHER,
                    allow: true,
                },
                1.0,
            ),
            (
                ME,
                Action::SetClaimPublic {
                    x: 9,
                    y: 1,
                    public: true,
                },
                1.0,
            ),
            (
                OTHER,
                Action::Place {
                    x: 12,
                    y: 2,
                    item: DIRT,
                },
                2.0,
            ), // now allowed
            (
                ME,
                Action::Place {
                    x: 10,
                    y: 3,
                    item: WORLD_CLAIM,
                },
                2.0,
            ), // refused: OTHER has a claim
        ];
        // OTHER removes their claim, ME removes the small one (the tree in it stays), then claims the world.
        for hit in 1..=hits {
            script.push((OTHER, Action::Punch { x: 15, y: 1 }, 10.0 + f64::from(hit)));
            script.push((ME, Action::Punch { x: 11, y: 1 }, 10.0 + f64::from(hit)));
        }
        script.extend([
            (
                ME,
                Action::Place {
                    x: 10,
                    y: 3,
                    item: WORLD_CLAIM,
                },
                30.0,
            ),
            (OTHER, Action::Punch { x: 12, y: 1 }, 31.0), // the tree is world-claimed now: refused
            (ME, Action::Punch { x: 12, y: 1 }, 1000.0),  // harvest
            (
                ME,
                Action::Place {
                    x: 11,
                    y: 1,
                    item: DIRT,
                },
                1001.0,
            ), // a block where the claim stood
        ]);
        let mut rejected = 0;
        for (step, (player, action, now)) in script.into_iter().enumerate() {
            rejected += u32::from(apply(&mut state, player, action, now).is_err());
            assert_trees_match_tiles(&state);
            assert!(
                state
                    .world
                    .claim_of
                    .values()
                    .all(|key| state.world.claim(*key).is_some()),
                "step {step}"
            );
        }
        assert_eq!(rejected, 3);
        assert_eq!(state.world.claim_of.len(), 48); // only the medium claim is left as an area
        assert!(state.world.world_claim.is_some());
        assert_eq!(state.world.extras.len(), 2);
    }
}

#[cfg(test)]
mod admin_tests {
    use super::*;
    use crate::commands::Admin;
    use crate::items::{BEDROCK, DIRT, SMALL_CLAIM};
    use crate::mirror::apply_event;
    use crate::worldgen::generate;

    const ME: PlayerId = PlayerId(1);

    fn world() -> WorldState {
        let mut state = WorldState::new(generate("ADMIN"), 1);
        state.join(ME);
        state
    }

    /// Runs `admin`; a refusal must leave the state untouched, a success must replay on a copy.
    fn run(state: &mut WorldState, admin: Admin) -> Result<Vec<Event>, Reject> {
        let before = state.clone();
        let result = apply_admin(state, ME, admin, 0.0);
        match &result {
            Err(_) => assert_eq!(*state, before, "{admin:?} refused but changed the state"),
            Ok(events) => {
                let mut copy = before;
                for event in events {
                    apply_event(&mut copy, event, 0.0);
                }
                assert_eq!(copy.world, state.world, "{admin:?}: events incomplete");
                assert_eq!(copy.players[&ME].inventory, state.players[&ME].inventory);
                assert_eq!(copy.players[&ME].gems, state.players[&ME].gems);
                assert_eq!(copy.players[&ME].equipped, state.players[&ME].equipped);
            }
        }
        result
    }

    #[test]
    fn give_past_the_cap_adds_only_what_fits() {
        let mut state = world();
        let give = |count| Admin::Give {
            to: ME,
            item: DIRT,
            count,
        };
        assert_eq!(
            run(&mut state, give(150)),
            Ok(vec![Event::InventoryChanged {
                player: ME,
                item: DIRT,
                count: 150
            }])
        );
        // 50 of the 120 fit; the new total says so.
        assert_eq!(
            run(&mut state, give(120)),
            Ok(vec![Event::InventoryChanged {
                player: ME,
                item: DIRT,
                count: MAX_STACK
            }])
        );
        assert_eq!(run(&mut state, give(1)), Err(Reject::InventoryFull));
        assert_eq!(run(&mut state, give(0)), Err(Reject::Invalid));
        assert_eq!(
            run(
                &mut state,
                Admin::Give {
                    to: PlayerId(99),
                    item: DIRT,
                    count: 1
                }
            ),
            Err(Reject::Invalid)
        );
    }

    #[test]
    fn take_more_than_held_takes_what_is_there_and_unequips() {
        let mut state = world();
        run(
            &mut state,
            Admin::Give {
                to: ME,
                item: DIRT,
                count: 5,
            },
        )
        .expect("given");
        assert_eq!(
            run(
                &mut state,
                Admin::Take {
                    from: ME,
                    item: DIRT,
                    count: 9
                }
            ),
            Ok(vec![Event::InventoryChanged {
                player: ME,
                item: DIRT,
                count: 0
            }])
        );
        assert_eq!(
            run(
                &mut state,
                Admin::Take {
                    from: ME,
                    item: DIRT,
                    count: 1
                }
            ),
            Err(Reject::NoItem)
        );
        // The fist never leaves.
        assert_eq!(
            run(
                &mut state,
                Admin::Take {
                    from: ME,
                    item: crate::items::FIST,
                    count: 1
                }
            ),
            Err(Reject::Invalid)
        );
        // Taking a worn starter item takes it off too.
        let shirt = state.players[&ME].worn(Slot::Shirt).expect("starter shirt");
        let events = run(
            &mut state,
            Admin::Take {
                from: ME,
                item: shirt,
                count: 1,
            },
        )
        .expect("taken");
        assert!(events.contains(&Event::Equipped {
            player: ME,
            slot: Slot::Shirt,
            item: None
        }));
    }

    #[test]
    fn giveall_fills_the_biggest_backpack() {
        let mut state = world();
        let events = run(&mut state, Admin::GiveAll { to: ME }).expect("given");
        let player = &state.players[&ME];
        assert_eq!(player.inventory.capacity, 396);
        assert_eq!(player.inventory.count(DIRT), MAX_STACK);
        assert_eq!(player.inventory.count(SMALL_CLAIM), 1);
        assert_eq!(player.inventory.count(crate::items::FIST), 1);
        assert_eq!(
            player.inventory.count(crate::items::MAIN_DOOR),
            0,
            "no fixtures"
        );
        assert!(events.len() > 100);
        // Running it again changes nothing.
        assert_eq!(run(&mut state, Admin::GiveAll { to: ME }), Ok(vec![]));
    }

    #[test]
    fn clearinv_leaves_only_the_fist_and_wrench() {
        let mut state = world();
        run(&mut state, Admin::GiveAll { to: ME }).expect("given");
        state.players.get_mut(&ME).expect("joined").gems = 77;
        let events = run(&mut state, Admin::ClearInv { from: ME }).expect("cleared");
        let player = &state.players[&ME];
        assert_eq!(
            player.inventory.slots(),
            &[(crate::items::FIST, 1), (crate::items::WRENCH, 1)]
        );
        assert!(player.equipped.iter().all(Option::is_none), "nothing worn");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Equipped { item: None, .. }))
        );
        assert_eq!(
            (player.gems, player.inventory.capacity),
            (77, 396),
            "gems and capacity stay"
        );
    }

    #[test]
    fn gems_saturate_at_both_ends() {
        let mut state = world();
        assert_eq!(
            run(&mut state, Admin::Gems { who: ME, delta: -5 }),
            Ok(vec![])
        );
        assert_eq!(
            run(
                &mut state,
                Admin::Gems {
                    who: ME,
                    delta: i64::MAX
                }
            ),
            Ok(vec![Event::GemsChanged {
                player: ME,
                gems: u32::MAX
            }])
        );
        assert_eq!(
            run(
                &mut state,
                Admin::Gems {
                    who: ME,
                    delta: i64::MIN
                }
            ),
            Ok(vec![Event::GemsChanged {
                player: ME,
                gems: 0
            }])
        );
    }

    #[test]
    fn spawn_drops_at_the_feet() {
        let mut state = world();
        let events = run(
            &mut state,
            Admin::Spawn {
                item: SMALL_CLAIM,
                count: 3,
            },
        )
        .expect("spawned");
        let door = state.world.door;
        assert!(matches!(events.as_slice(), [Event::ItemDropped { drop }]
            if drop.item == SMALL_CLAIM && drop.count == 3 && (drop.x, drop.y) == door));
        assert_eq!(
            run(
                &mut state,
                Admin::Spawn {
                    item: crate::items::FIST,
                    count: 1
                }
            ),
            Err(Reject::Invalid)
        );
        assert_eq!(
            run(
                &mut state,
                Admin::Spawn {
                    item: DIRT,
                    count: 201
                }
            ),
            Err(Reject::Invalid)
        );
    }

    #[test]
    fn setblock_and_clearblock_respect_fixtures_and_bounds() {
        let mut state = world();
        let (dx, dy) = state.world.door;
        let set = |x, y, item| Admin::SetBlock { x, y, item };
        assert_eq!(run(&mut state, set(dx, dy, DIRT)), Err(Reject::Unbreakable));
        assert_eq!(
            run(&mut state, Admin::ClearBlock { x: dx, y: dy }),
            Err(Reject::Unbreakable)
        );
        assert_eq!(
            run(&mut state, set(crate::WORLD_WIDTH, 0, DIRT)),
            Err(Reject::OutOfRange)
        );
        assert_eq!(run(&mut state, set(5, 0, DIRT)), Err(Reject::Unbreakable)); // bedrock row
        assert_eq!(run(&mut state, set(5, 30, BEDROCK)), Err(Reject::Invalid));
        // Solid on the player's own tile would trap them.
        assert_eq!(
            run(&mut state, set(dx, dy + 1, DIRT)),
            Err(Reject::Occupied)
        );

        // Air above the surface, far from the player: set, then clear.
        let (x, y) = (dx + 10, dy + 3);
        let events = run(&mut state, set(x, y, DIRT)).expect("set");
        assert!(matches!(events.as_slice(), [Event::TileChanged { tile, .. }] if tile.fg == DIRT));
        assert_eq!(
            run(&mut state, Admin::ClearBlock { x, y }).map(|e| e.len()),
            Ok(1)
        );
        assert_eq!(run(&mut state, Admin::ClearBlock { x, y }), Ok(vec![]));
    }

    #[test]
    fn setblock_over_a_claim_removes_it() {
        let mut state = world();
        let (x, y) = (state.world.door.0 + 1, state.world.door.1);
        state
            .players
            .get_mut(&ME)
            .expect("joined")
            .inventory
            .add(SMALL_CLAIM, 1);
        apply(
            &mut state,
            ME,
            Action::Place {
                x,
                y,
                item: SMALL_CLAIM,
            },
            0.0,
        )
        .expect("claim placed");
        let key = World::tile_key(x, y).expect("in world");
        assert!(state.world.claim(key).is_some());
        // A player stands next to it, not on it; a background does not trap anyone anyway.
        let events = run(&mut state, Admin::ClearBlock { x, y }).expect("cleared");
        assert!(events.contains(&Event::ClaimRemoved { x, y }));
        assert!(state.world.claim(key).is_none() && state.world.claim_of.is_empty());
    }
}
