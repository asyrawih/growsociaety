//! The trade state machine (docs/knowledge/08, "Trade flow" and "Commit algorithm"). It runs
//! through `rules::apply` like every other action: validate first, then write, then report.
//!
//! ```text
//! Requested --accept--> Editing --both lock--> Review --both confirm--> commit (removed)
//! ```
//! A decline, a cancel, a player leaving or a failed commit removes the trade.

use crate::inventory::MAX_STACK;
use crate::items::{ItemId, PERMANENT, UNTRADEABLE, item, slot_of};
use crate::rules::{Event, PlayerId, Reject, WorldState};

/// Stacks per side. Reference.
pub const TRADE_SLOTS: usize = 4;
/// Seconds in `Review` before a confirm is accepted. Tuning.
pub const TRADE_REVIEW_DELAY: f64 = 2.0;

/// One side's offer, slot by slot.
pub type Offer = [Option<(ItemId, u16)>; TRADE_SLOTS];

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TradeStage {
    /// `a` asked, `b` has not answered.
    Requested,
    /// Both may change their own offer and lock.
    Editing,
    /// Both locked at `since`: offers are frozen, only confirm or cancel.
    Review { since: f64 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TradeEnd {
    /// `b` said no to the request.
    Declined,
    /// Either side pressed cancel.
    Cancelled,
    /// A side left the world or disconnected.
    Left,
    /// The commit found an offer no longer held or no longer valid.
    Invalid,
    /// The commit would have put a receiver over the stack cap or out of slots.
    CarryingTooMuch,
}

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Trade {
    pub id: u32,
    /// `a` requested, `b` was asked.
    pub a: PlayerId,
    pub b: PlayerId,
    pub stage: TradeStage,
    /// Index 0 is `a`'s offer, 1 is `b`'s.
    pub offers: [Offer; 2],
    pub locked: [bool; 2],
    pub confirmed: [bool; 2],
}

impl Trade {
    /// A fresh request from `a` to `b`.
    pub fn new(id: u32, a: PlayerId, b: PlayerId) -> Self {
        Self {
            id,
            a,
            b,
            stage: TradeStage::Requested,
            offers: [[None; TRADE_SLOTS]; 2],
            locked: [false; 2],
            confirmed: [false; 2],
        }
    }

    /// 0 for `a`, 1 for `b`, `None` for anyone else.
    pub fn side(&self, player: PlayerId) -> Option<usize> {
        if player == self.a {
            Some(0)
        } else if player == self.b {
            Some(1)
        } else {
            None
        }
    }

    /// The other party of `player`'s trade.
    pub fn other(&self, player: PlayerId) -> Option<PlayerId> {
        match self.side(player)? {
            0 => Some(self.b),
            _ => Some(self.a),
        }
    }

    pub fn involves(&self, player: PlayerId) -> bool {
        self.side(player).is_some()
    }
}

/// The index in `state.trades` of the one open trade or request `player` is in.
fn open_trade(state: &WorldState, player: PlayerId) -> Option<usize> {
    state.trades.iter().position(|trade| trade.involves(player))
}

fn updated(trade: &Trade) -> Vec<Event> {
    vec![Event::TradeUpdated {
        trade: trade.clone(),
    }]
}

fn cancelled(id: u32, reason: TradeEnd) -> Event {
    Event::TradeCancelled { trade: id, reason }
}

pub(crate) fn request(
    state: &mut WorldState,
    from: PlayerId,
    to: PlayerId,
) -> Result<Vec<Event>, Reject> {
    if from == to || !state.players.contains_key(&from) || !state.players.contains_key(&to) {
        return Err(Reject::Invalid);
    }
    if open_trade(state, from).is_some() {
        return Err(Reject::Invalid);
    }
    // The other player is busy with someone else.
    if open_trade(state, to).is_some() {
        return Err(Reject::Occupied);
    }
    let id = state.next_trade_id;
    // ponytail: ids wrap after 2^32 trades in one world; a wrapped id only collides with a trade that is still open.
    state.next_trade_id = id.wrapping_add(1);
    state.trades.push(Trade::new(id, from, to));
    Ok(vec![Event::TradeRequested {
        trade: id,
        from,
        to,
    }])
}

pub(crate) fn respond(
    state: &mut WorldState,
    player: PlayerId,
    accept: bool,
) -> Result<Vec<Event>, Reject> {
    let index = open_trade(state, player).ok_or(Reject::Invalid)?;
    let trade = state.trades.get_mut(index).ok_or(Reject::Invalid)?;
    if trade.b != player || trade.stage != TradeStage::Requested {
        return Err(Reject::Invalid);
    }
    if accept {
        trade.stage = TradeStage::Editing;
        Ok(updated(trade))
    } else {
        let id = trade.id;
        state.trades.remove(index);
        Ok(vec![cancelled(id, TradeEnd::Declined)])
    }
}

/// Why `(offered, count)` cannot be put in a slot, if it cannot. Holding enough is checked separately.
fn offer_error(offered: ItemId, count: u16) -> Option<Reject> {
    let tradeable = item(offered).is_some_and(|def| !def.has(UNTRADEABLE | PERMANENT));
    (!tradeable || !(1..=MAX_STACK).contains(&count)).then_some(Reject::Invalid)
}

pub(crate) fn set_offer(
    state: &mut WorldState,
    player: PlayerId,
    slot: u8,
    offered: ItemId,
    count: u16,
) -> Result<Vec<Event>, Reject> {
    let held = state
        .players
        .get(&player)
        .ok_or(Reject::Invalid)?
        .inventory
        .count(offered);
    let index = open_trade(state, player).ok_or(Reject::Invalid)?;
    let trade = state.trades.get_mut(index).ok_or(Reject::Invalid)?;
    let side = trade.side(player).ok_or(Reject::Invalid)?;
    if trade.stage != TradeStage::Editing {
        return Err(Reject::Invalid);
    }
    let offer = trade.offers.get(side).ok_or(Reject::Invalid)?;
    let current = *offer.get(usize::from(slot)).ok_or(Reject::Invalid)?;
    let wanted = if count == 0 {
        None
    } else {
        if let Some(reject) = offer_error(offered, count) {
            return Err(reject);
        }
        let elsewhere = offer
            .iter()
            .enumerate()
            .any(|(i, entry)| i != usize::from(slot) && entry.is_some_and(|(id, _)| id == offered));
        if elsewhere {
            return Err(Reject::Invalid);
        }
        if held < count {
            return Err(Reject::NoItem);
        }
        Some((offered, count))
    };
    if wanted == current {
        return Ok(Vec::new());
    }

    // Any change to either offer takes back both locks and confirms (the anti-scam rule).
    if let Some(entry) = trade
        .offers
        .get_mut(side)
        .and_then(|offer| offer.get_mut(usize::from(slot)))
    {
        *entry = wanted;
    }
    trade.locked = [false; 2];
    trade.confirmed = [false; 2];
    Ok(updated(trade))
}

pub(crate) fn lock(
    state: &mut WorldState,
    player: PlayerId,
    locked: bool,
    now: f64,
) -> Result<Vec<Event>, Reject> {
    let index = open_trade(state, player).ok_or(Reject::Invalid)?;
    let trade = state.trades.get_mut(index).ok_or(Reject::Invalid)?;
    let side = trade.side(player).ok_or(Reject::Invalid)?;
    // In Review the offers are frozen: confirm or cancel only.
    if trade.stage != TradeStage::Editing {
        return Err(Reject::Invalid);
    }
    let entry = trade.locked.get_mut(side).ok_or(Reject::Invalid)?;
    if *entry == locked {
        return Ok(Vec::new());
    }
    *entry = locked;
    if trade.locked == [true; 2] {
        trade.stage = TradeStage::Review { since: now };
    }
    Ok(updated(trade))
}

pub(crate) fn confirm(
    state: &mut WorldState,
    player: PlayerId,
    now: f64,
) -> Result<Vec<Event>, Reject> {
    let index = open_trade(state, player).ok_or(Reject::Invalid)?;
    let trade = state.trades.get_mut(index).ok_or(Reject::Invalid)?;
    let side = trade.side(player).ok_or(Reject::Invalid)?;
    let TradeStage::Review { since } = trade.stage else {
        return Err(Reject::Invalid);
    };
    if now < since + TRADE_REVIEW_DELAY {
        return Err(Reject::Cooldown);
    }
    let entry = trade.confirmed.get_mut(side).ok_or(Reject::Invalid)?;
    if *entry {
        return Ok(Vec::new());
    }
    *entry = true;
    if trade.confirmed != [true; 2] {
        return Ok(updated(trade));
    }
    let trade = state.trades.remove(index);
    Ok(commit(state, &trade))
}

pub(crate) fn cancel(state: &mut WorldState, player: PlayerId) -> Result<Vec<Event>, Reject> {
    let index = open_trade(state, player).ok_or(Reject::Invalid)?;
    let trade = state.trades.remove(index);
    Ok(vec![cancelled(trade.id, TradeEnd::Cancelled)])
}

/// Cancels every trade or request `player` is in, with `TradeEnd::Left`. Call it when a player
/// leaves the world or disconnects.
pub fn end_trades_of(state: &mut WorldState, player: PlayerId) -> Vec<Event> {
    let mut events = Vec::new();
    state.trades.retain(|trade| {
        let keep = !trade.involves(player);
        if !keep {
            events.push(cancelled(trade.id, TradeEnd::Left));
        }
        keep
    });
    events
}

/// The swap of a trade both sides confirmed, already taken out of `state.trades`. Everything is
/// re-validated from scratch and simulated on copies of both inventories; only a fully successful
/// simulation is written. A failure moves nothing and reports why the trade ended.
fn commit(state: &mut WorldState, trade: &Trade) -> Vec<Event> {
    match simulate(state, trade) {
        Ok(after) => {
            let mut events = vec![Event::TradeCompleted { trade: trade.id }];
            for (who, inventory) in [trade.a, trade.b].into_iter().zip(after) {
                let Some(player) = state.players.get_mut(&who) else {
                    continue;
                };
                let mut items: Vec<ItemId> = Vec::new();
                for &(id, _) in trade.offers.iter().flatten().flatten() {
                    if !items.contains(&id) {
                        items.push(id);
                    }
                }
                let old = std::mem::replace(&mut player.inventory, inventory);
                for id in items {
                    let count = player.inventory.count(id);
                    if count == old.count(id) {
                        continue;
                    }
                    events.push(Event::InventoryChanged {
                        player: who,
                        item: id,
                        count,
                    });
                    // A worn item whose last copy left comes off in the same commit.
                    let worn = slot_of(id).filter(|&slot| player.worn(slot) == Some(id));
                    if let Some(slot) = worn.filter(|_| count == 0) {
                        if let Some(entry) = player.equipped.get_mut(slot as usize) {
                            *entry = None;
                        }
                        events.push(Event::Equipped {
                            player: who,
                            slot,
                            item: None,
                        });
                    }
                }
            }
            events
        }
        Err(reason) => vec![cancelled(trade.id, reason)],
    }
}

/// Both inventories as they would be after the swap, or why the trade cannot complete.
fn simulate(
    state: &WorldState,
    trade: &Trade,
) -> Result<[crate::inventory::Inventory; 2], TradeEnd> {
    let both = [trade.a, trade.b].map(|who| state.players.get(&who));
    let [Some(a), Some(b)] = both else {
        return Err(TradeEnd::Left);
    };
    for offer in &trade.offers {
        let stacks: Vec<_> = offer.iter().flatten().collect();
        let duplicate = stacks
            .iter()
            .enumerate()
            .any(|(i, (id, _))| stacks.iter().skip(i + 1).any(|(other, _)| other == id));
        if duplicate || stacks.iter().any(|&&(id, n)| offer_error(id, n).is_some()) {
            return Err(TradeEnd::Invalid);
        }
    }
    let mut after = [a.inventory.clone(), b.inventory.clone()];
    for (inventory, offer) in after.iter_mut().zip(&trade.offers) {
        for &(id, n) in offer.iter().flatten() {
            if !inventory.remove(id, n) {
                return Err(TradeEnd::Invalid);
            }
        }
    }
    let [to_a, to_b] = &trade.offers;
    for (inventory, incoming) in after.iter_mut().zip([to_b, to_a]) {
        for &(id, n) in incoming.iter().flatten() {
            if inventory.add(id, n) != n {
                return Err(TradeEnd::CarryingTooMuch);
            }
        }
    }
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{DIRT, FIST, SAND, SHIRT_RED, STONE, Slot, WRENCH};
    use crate::rules::{Action, apply};
    use crate::world::World;

    const A: PlayerId = PlayerId(1);
    const B: PlayerId = PlayerId(2);

    fn two_players() -> WorldState {
        let mut state = WorldState::new(World::empty("T".into()), 1);
        state.join(A);
        state.join(B);
        state
    }

    fn give(state: &mut WorldState, who: PlayerId, id: ItemId, n: u16) {
        let added = state.players.get_mut(&who).unwrap().inventory.add(id, n);
        assert_eq!(added, n);
    }

    fn held(state: &WorldState, who: PlayerId, id: ItemId) -> u16 {
        state.players[&who].inventory.count(id)
    }

    fn act(state: &mut WorldState, who: PlayerId, action: Action, now: f64) -> Vec<Event> {
        apply(state, who, action, now).unwrap_or_else(|r| panic!("{action:?} by {who:?}: {r:?}"))
    }

    fn offer(slot: u8, item: ItemId, count: u16) -> Action {
        Action::TradeSetOffer { slot, item, count }
    }

    /// A asked B and B accepted: both are editing.
    fn editing() -> WorldState {
        let mut state = two_players();
        act(&mut state, A, Action::TradeRequest { to: B }, 0.0);
        act(&mut state, B, Action::TradeRespond { accept: true }, 0.0);
        state
    }

    fn trade(state: &WorldState) -> &Trade {
        state.trades.first().expect("a trade is open")
    }

    /// Both lock at `now` and both confirm after the review delay; returns the confirm events.
    fn lock_and_confirm(state: &mut WorldState, now: f64) -> Vec<Event> {
        act(state, A, Action::TradeLock { locked: true }, now);
        act(state, B, Action::TradeLock { locked: true }, now);
        let later = now + TRADE_REVIEW_DELAY;
        act(state, A, Action::TradeConfirm, later);
        act(state, B, Action::TradeConfirm, later)
    }

    #[test]
    fn a_full_trade_swaps_and_conserves_every_item_count() {
        let mut state = editing();
        give(&mut state, A, DIRT, 10);
        give(&mut state, A, STONE, 3);
        give(&mut state, B, SAND, 7);
        give(&mut state, B, DIRT, 1);
        let total = |state: &WorldState, id| held(state, A, id) + held(state, B, id);
        let before = [DIRT, STONE, SAND].map(|id| total(&state, id));

        act(&mut state, A, offer(0, DIRT, 4), 1.0);
        act(&mut state, A, offer(3, STONE, 3), 1.0);
        act(&mut state, B, offer(1, SAND, 7), 1.0);
        act(&mut state, B, offer(2, DIRT, 1), 1.0);
        let events = lock_and_confirm(&mut state, 2.0);

        assert_eq!(events.first(), Some(&Event::TradeCompleted { trade: 0 }));
        assert!(state.trades.is_empty());
        assert_eq!(
            [DIRT, STONE, SAND].map(|id| (held(&state, A, id), held(&state, B, id))),
            [(7, 4), (0, 3), (7, 0)]
        );
        assert_eq!([DIRT, STONE, SAND].map(|id| total(&state, id)), before);
        // One InventoryChanged per changed item and player, carrying the new totals.
        for (who, id) in [
            (A, DIRT),
            (A, STONE),
            (A, SAND),
            (B, DIRT),
            (B, STONE),
            (B, SAND),
        ] {
            let change = Event::InventoryChanged {
                player: who,
                item: id,
                count: held(&state, who, id),
            };
            assert!(events.contains(&change), "{change:?} in {events:?}");
        }
    }

    #[test]
    fn an_edit_after_lock_clears_both_locks_and_confirms() {
        let mut state = editing();
        give(&mut state, A, DIRT, 5);
        act(&mut state, A, offer(0, DIRT, 1), 1.0);
        act(&mut state, A, Action::TradeLock { locked: true }, 1.0);
        assert_eq!(trade(&state).locked, [true, false]);
        // B's edit takes A's lock back too.
        give(&mut state, B, SAND, 1);
        let events = act(&mut state, B, offer(0, SAND, 1), 1.0);
        assert_eq!(trade(&state).locked, [false, false]);
        assert!(
            matches!(events.as_slice(), [Event::TradeUpdated { trade }] if trade.locked == [false; 2])
        );
        // Both lock again: Review, frozen.
        act(&mut state, A, Action::TradeLock { locked: true }, 2.0);
        act(&mut state, B, Action::TradeLock { locked: true }, 3.0);
        assert_eq!(trade(&state).stage, TradeStage::Review { since: 3.0 });
        let before = state.clone();
        assert_eq!(
            apply(&mut state, A, offer(0, DIRT, 2), 4.0),
            Err(Reject::Invalid)
        );
        assert_eq!(
            apply(&mut state, A, Action::TradeLock { locked: false }, 4.0),
            Err(Reject::Invalid)
        );
        assert_eq!(state, before);
        // Setting what is already there is no change and keeps the locks.
        let mut editing_state = editing();
        give(&mut editing_state, A, DIRT, 1);
        act(&mut editing_state, A, offer(0, DIRT, 1), 1.0);
        act(
            &mut editing_state,
            B,
            Action::TradeLock { locked: true },
            1.0,
        );
        assert_eq!(act(&mut editing_state, A, offer(0, DIRT, 1), 1.0), vec![]);
        assert_eq!(trade(&editing_state).locked, [false, true]);
    }

    #[test]
    fn confirm_waits_for_the_review_delay() {
        let mut state = editing();
        act(&mut state, A, Action::TradeLock { locked: true }, 10.0);
        act(&mut state, B, Action::TradeLock { locked: true }, 10.0);
        let before = state.clone();
        let early = 10.0 + TRADE_REVIEW_DELAY - 0.01;
        assert_eq!(
            apply(&mut state, A, Action::TradeConfirm, early),
            Err(Reject::Cooldown)
        );
        assert_eq!(state, before);
        let events = act(
            &mut state,
            A,
            Action::TradeConfirm,
            10.0 + TRADE_REVIEW_DELAY,
        );
        assert!(
            matches!(events.as_slice(), [Event::TradeUpdated { trade }] if trade.confirmed == [true, false])
        );
        // Confirming twice changes nothing.
        assert_eq!(act(&mut state, A, Action::TradeConfirm, 13.0), vec![]);
    }

    #[test]
    fn a_receiver_at_199_getting_2_cancels_and_moves_nothing() {
        let mut state = editing();
        give(&mut state, A, DIRT, 2);
        give(&mut state, B, DIRT, 199);
        act(&mut state, A, offer(0, DIRT, 2), 1.0);
        let before = [A, B].map(|who| state.players[&who].clone());
        let events = lock_and_confirm(&mut state, 2.0);
        assert_eq!(
            events,
            vec![Event::TradeCancelled {
                trade: 0,
                reason: TradeEnd::CarryingTooMuch
            }]
        );
        assert_eq!([A, B].map(|who| state.players[&who].clone()), before);
        assert!(state.trades.is_empty());
    }

    #[test]
    fn a_receiver_without_a_free_slot_cancels() {
        let mut state = editing();
        give(&mut state, A, SAND, 1);
        let b = state.players.get_mut(&B).unwrap();
        b.inventory.capacity = u16::try_from(b.inventory.slots().len()).unwrap();
        act(&mut state, A, offer(0, SAND, 1), 1.0);
        let events = lock_and_confirm(&mut state, 2.0);
        assert!(matches!(
            events.as_slice(),
            [Event::TradeCancelled {
                reason: TradeEnd::CarryingTooMuch,
                ..
            }]
        ));
        assert_eq!((held(&state, A, SAND), held(&state, B, SAND)), (1, 0));
    }

    #[test]
    fn a_commit_after_one_side_dropped_the_offered_items_cancels() {
        let mut state = editing();
        give(&mut state, A, DIRT, 3);
        give(&mut state, B, SAND, 5);
        act(&mut state, A, offer(0, DIRT, 3), 1.0);
        act(&mut state, B, offer(0, SAND, 5), 1.0);
        act(&mut state, A, Action::TradeLock { locked: true }, 1.0);
        act(&mut state, B, Action::TradeLock { locked: true }, 1.0);
        // An offer is a promise, not an escrow: A can still get rid of the items.
        act(
            &mut state,
            A,
            Action::Trash {
                item: DIRT,
                count: 1,
            },
            1.5,
        );
        act(&mut state, A, Action::TradeConfirm, 5.0);
        let events = act(&mut state, B, Action::TradeConfirm, 5.0);
        assert_eq!(
            events,
            vec![Event::TradeCancelled {
                trade: 0,
                reason: TradeEnd::Invalid
            }]
        );
        assert_eq!((held(&state, A, DIRT), held(&state, B, DIRT)), (2, 0));
        assert_eq!((held(&state, A, SAND), held(&state, B, SAND)), (0, 5));
    }

    #[test]
    fn the_last_worn_copy_is_unequipped_on_commit() {
        let mut state = editing();
        assert_eq!(state.players[&A].worn(Slot::Shirt), Some(SHIRT_RED));
        act(&mut state, A, offer(0, SHIRT_RED, 1), 1.0);
        let events = lock_and_confirm(&mut state, 2.0);
        assert!(events.contains(&Event::Equipped {
            player: A,
            slot: Slot::Shirt,
            item: None
        }));
        assert_eq!(state.players[&A].worn(Slot::Shirt), None);
        // B already owned a red shirt (starter outfit) and keeps wearing it.
        assert_eq!(held(&state, B, SHIRT_RED), 2);
        assert_eq!(state.players[&B].worn(Slot::Shirt), Some(SHIRT_RED));
    }

    #[test]
    fn offers_are_validated_when_edited() {
        let mut state = editing();
        give(&mut state, A, DIRT, 5);
        give(&mut state, A, STONE, 5);
        act(&mut state, A, offer(0, DIRT, 5), 1.0);
        let before = state.clone();
        for (action, reject) in [
            (offer(1, FIST, 1), Reject::Invalid),
            (offer(1, WRENCH, 1), Reject::Invalid),
            (offer(1, ItemId(u16::MAX), 1), Reject::Invalid),
            (offer(1, STONE, MAX_STACK + 1), Reject::Invalid),
            (offer(1, STONE, 6), Reject::NoItem),
            (offer(1, SAND, 1), Reject::NoItem),
            (offer(1, DIRT, 1), Reject::Invalid), // duplicate in one offer
            (offer(TRADE_SLOTS as u8, STONE, 1), Reject::Invalid),
            (offer(u8::MAX, STONE, 1), Reject::Invalid),
            (Action::TradeRespond { accept: true }, Reject::Invalid), // not pending
            (Action::TradeConfirm, Reject::Invalid),                  // not in review
            (Action::TradeRequest { to: B }, Reject::Invalid),        // already trading
        ] {
            assert_eq!(apply(&mut state, A, action, 2.0), Err(reject), "{action:?}");
            assert_eq!(state, before, "{action:?}");
        }
        // Count 0 clears the slot, and the slot can then take the same item again.
        act(&mut state, A, offer(0, DIRT, 0), 2.0);
        assert_eq!(trade(&state).offers[0][0], None);
        act(&mut state, A, offer(1, DIRT, 2), 2.0);
        assert_eq!(trade(&state).offers[0][1], Some((DIRT, 2)));
    }

    #[test]
    fn a_request_can_be_declined_and_only_the_asked_player_answers() {
        let mut state = two_players();
        let events = act(&mut state, A, Action::TradeRequest { to: B }, 0.0);
        assert_eq!(
            events,
            vec![Event::TradeRequested {
                trade: 0,
                from: A,
                to: B
            }]
        );
        let before = state.clone();
        for (who, action) in [
            (A, Action::TradeRespond { accept: true }),
            (A, offer(0, DIRT, 1)),
            (B, offer(0, DIRT, 1)),
            (B, Action::TradeLock { locked: true }),
        ] {
            assert_eq!(apply(&mut state, who, action, 0.0), Err(Reject::Invalid));
            assert_eq!(state, before);
        }
        let events = act(&mut state, B, Action::TradeRespond { accept: false }, 0.0);
        assert_eq!(
            events,
            vec![Event::TradeCancelled {
                trade: 0,
                reason: TradeEnd::Declined
            }]
        );
        assert!(state.trades.is_empty());
    }

    #[test]
    fn either_side_can_cancel_at_any_stage() {
        for (who, stage) in [(A, 0), (B, 0), (A, 1), (B, 1), (A, 2), (B, 2)] {
            let mut state = two_players();
            act(&mut state, A, Action::TradeRequest { to: B }, 0.0);
            if stage >= 1 {
                act(&mut state, B, Action::TradeRespond { accept: true }, 0.0);
            }
            if stage >= 2 {
                act(&mut state, A, Action::TradeLock { locked: true }, 0.0);
                act(&mut state, B, Action::TradeLock { locked: true }, 0.0);
            }
            let events = act(&mut state, who, Action::TradeCancel, 1.0);
            assert_eq!(
                events,
                vec![Event::TradeCancelled {
                    trade: 0,
                    reason: TradeEnd::Cancelled
                }]
            );
            assert!(state.trades.is_empty());
            assert_eq!(
                apply(&mut state, who, Action::TradeCancel, 1.0),
                Err(Reject::Invalid)
            );
        }
    }

    #[test]
    fn leaving_ends_the_trades_of_that_player_with_left() {
        let mut state = editing();
        state.join(PlayerId(3));
        state.join(PlayerId(4));
        act(
            &mut state,
            PlayerId(3),
            Action::TradeRequest { to: PlayerId(4) },
            0.0,
        );
        assert_eq!(
            end_trades_of(&mut state, B),
            vec![Event::TradeCancelled {
                trade: 0,
                reason: TradeEnd::Left
            }]
        );
        assert_eq!(state.trades.len(), 1);
        assert_eq!(state.trades[0].a, PlayerId(3));
        assert_eq!(end_trades_of(&mut state, B), vec![]);
    }

    #[test]
    fn one_open_request_or_trade_per_player() {
        let mut state = two_players();
        let c = PlayerId(3);
        state.join(c);
        assert_eq!(
            apply(&mut state, A, Action::TradeRequest { to: A }, 0.0),
            Err(Reject::Invalid)
        );
        assert_eq!(
            apply(
                &mut state,
                A,
                Action::TradeRequest { to: PlayerId(99) },
                0.0
            ),
            Err(Reject::Invalid)
        );
        act(&mut state, A, Action::TradeRequest { to: B }, 0.0);
        let before = state.clone();
        // B already has a pending request, A already asked someone, and C cannot reach either.
        assert_eq!(
            apply(&mut state, c, Action::TradeRequest { to: B }, 0.0),
            Err(Reject::Occupied)
        );
        assert_eq!(
            apply(&mut state, c, Action::TradeRequest { to: A }, 0.0),
            Err(Reject::Occupied)
        );
        assert_eq!(
            apply(&mut state, A, Action::TradeRequest { to: c }, 0.0),
            Err(Reject::Invalid)
        );
        assert_eq!(
            apply(&mut state, B, Action::TradeRequest { to: c }, 0.0),
            Err(Reject::Invalid)
        );
        assert_eq!(state, before);
        // Once it ends, a new one gets a new id.
        act(&mut state, B, Action::TradeCancel, 0.0);
        let events = act(&mut state, c, Action::TradeRequest { to: B }, 0.0);
        assert_eq!(
            events,
            vec![Event::TradeRequested {
                trade: 1,
                from: c,
                to: B
            }]
        );
    }

    #[test]
    fn trading_gives_no_xp() {
        let mut state = editing();
        give(&mut state, A, DIRT, 5);
        act(&mut state, A, offer(0, DIRT, 5), 1.0);
        let events = lock_and_confirm(&mut state, 2.0);
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, Event::XpChanged { .. } | Event::LevelUp { .. }))
        );
        assert_eq!((state.players[&A].level, state.players[&A].xp), (1, 0));
    }
}
