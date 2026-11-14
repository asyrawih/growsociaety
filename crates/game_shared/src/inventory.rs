//! A player's backpack: one slot per item type, at most `MAX_STACK` of each (docs/knowledge/08).

use crate::items::{FIST, ItemId, PERMANENT, WRENCH};

/// Reference. The most load-bearing number in the economy.
pub const MAX_STACK: u16 = 200;
/// Reference. Two of them are taken by the fist and the wrench.
pub const STARTING_SLOTS: u16 = 16;

/// Slots added by one backpack upgrade. Reference.
pub const SLOTS_PER_UPGRADE: u16 = 10;
/// Reference: 16 + 38 * 10 = 396 slots at most.
pub const MAX_UPGRADES: u16 = 38;
/// Gems for the first upgrade; each next one costs `UPGRADE_COST_NUM / UPGRADE_COST_DEN` times more. Tuning.
pub const UPGRADE_BASE_COST: u32 = 100;
pub const UPGRADE_COST_NUM: u32 = 6;
pub const UPGRADE_COST_DEN: u32 = 5;

/// Gem price of the upgrade bought when `bought` upgrades are already owned. Integer maths.
pub fn upgrade_cost(bought: u16) -> u32 {
    (0..bought.min(MAX_UPGRADES)).fold(UPGRADE_BASE_COST, |cost, _| {
        cost.saturating_mul(UPGRADE_COST_NUM) / UPGRADE_COST_DEN
    })
}

#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Inventory {
    /// Slots.
    pub capacity: u16,
    /// One entry per item type, count `1..=MAX_STACK`, in the order items were first acquired.
    slots: Vec<(ItemId, u16)>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            capacity: STARTING_SLOTS,
            slots: vec![(FIST, 1), (WRENCH, 1)],
        }
    }
}

impl Inventory {
    /// Backpack upgrades bought so far, derived from the capacity.
    pub fn upgrades(&self) -> u16 {
        self.capacity.saturating_sub(STARTING_SLOTS) / SLOTS_PER_UPGRADE
    }

    /// Read-only view in slot order, which is also the hotbar order.
    pub fn slots(&self) -> &[(ItemId, u16)] {
        &self.slots
    }

    pub fn count(&self, item: ItemId) -> u16 {
        let slot = self.slots.iter().find(|slot| slot.0 == item);
        slot.map_or(0, |slot| slot.1)
    }

    /// How many more of `item` would fit. Zero for ids that are not in the item table.
    pub fn room_for(&self, item: ItemId) -> u16 {
        let held = self.count(item);
        let has_slot = held > 0 || self.slots.len() < usize::from(self.capacity);
        if has_slot && crate::items::item(item).is_some() {
            MAX_STACK.saturating_sub(held)
        } else {
            0
        }
    }

    /// Adds up to `n` and returns the amount actually added. The caller decides what happens to the rest.
    pub fn add(&mut self, item: ItemId, n: u16) -> u16 {
        let added = n.min(self.room_for(item));
        if added > 0 {
            match self.slots.iter_mut().find(|slot| slot.0 == item) {
                Some(slot) => slot.1 += added,
                None => self.slots.push((item, added)),
            }
        }
        added
    }

    /// All or nothing. `PERMANENT` items never leave.
    pub fn remove(&mut self, item: ItemId, n: u16) -> bool {
        let permanent = crate::items::item(item).is_some_and(|def| def.has(PERMANENT));
        let Some(index) = self.slots.iter().position(|slot| slot.0 == item) else {
            return false;
        };
        match self.slots.get_mut(index) {
            Some(slot) if !permanent && n > 0 && slot.1 >= n => {
                slot.1 -= n;
                if slot.1 == 0 {
                    self.slots.remove(index);
                }
                true
            }
            _ => false,
        }
    }

    /// Moves raw slot `from` to raw slot `to`: swaps with an occupied
    /// `to`, or goes last when `to` is past the end. `false` (and nothing changes) for an
    /// out-of-range `from`, `from == to`, or a move that would change nothing.
    pub fn move_slot(&mut self, from: u16, to: u16) -> bool {
        let (from, to, len) = (usize::from(from), usize::from(to), self.slots.len());
        if from >= len || from == to || (to >= len && from == len - 1) {
            return false;
        }
        if to < len {
            self.slots.swap(from, to);
        } else {
            let slot = self.slots.remove(from);
            self.slots.push(slot);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{DIRT, EMPTY, ITEMS, SAND, STONE};

    #[test]
    fn move_slot_swaps_or_moves_to_the_end() {
        let mut inventory = Inventory::default(); // fist, wrench
        inventory.add(DIRT, 5);
        inventory.add(SAND, 3);
        let order = |inv: &Inventory| inv.slots().iter().map(|s| s.0).collect::<Vec<_>>();
        assert!(inventory.move_slot(3, 0), "swap onto an occupied slot");
        assert_eq!(
            order(&inventory),
            [SAND, crate::items::WRENCH, DIRT, crate::items::FIST]
        );
        assert!(inventory.move_slot(0, 99), "past the end: goes last");
        assert_eq!(
            order(&inventory),
            [crate::items::WRENCH, DIRT, crate::items::FIST, SAND]
        );
        let before = inventory.clone();
        assert!(!inventory.move_slot(9, 0), "out of range");
        assert!(!inventory.move_slot(1, 1), "same slot");
        assert!(!inventory.move_slot(3, 50), "already last");
        assert_eq!(inventory, before, "refusals change nothing");
        assert_eq!(inventory.count(DIRT), 5, "counts are untouched");
    }

    #[test]
    fn upgrade_cost_rises_by_a_fifth_in_integer_maths() {
        assert_eq!(upgrade_cost(0), 100);
        assert_eq!(upgrade_cost(1), 120);
        assert_eq!(upgrade_cost(2), 144);
        assert_eq!(upgrade_cost(3), 172); // 172.8 rounded down
        for n in 0..MAX_UPGRADES - 1 {
            assert!(upgrade_cost(n + 1) > upgrade_cost(n), "{n}");
        }
        // The 38th upgrade costs about 85,000 gems.
        let last = upgrade_cost(MAX_UPGRADES - 1);
        assert!((80_000..90_000).contains(&last), "{last}");
        assert_eq!(STARTING_SLOTS + MAX_UPGRADES * SLOTS_PER_UPGRADE, 396);
        assert_eq!(Inventory::default().upgrades(), 0);
    }

    #[test]
    fn starts_with_16_slots_holding_fist_and_wrench() {
        let inventory = Inventory::default();
        assert_eq!(inventory.capacity, 16);
        assert_eq!(inventory.slots(), &[(FIST, 1), (WRENCH, 1)]);
    }

    #[test]
    fn add_past_the_cap_adds_only_up_to_200() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(DIRT, 150), 150);
        assert_eq!(inventory.room_for(DIRT), 50);
        assert_eq!(inventory.add(DIRT, 80), 50);
        assert_eq!(inventory.count(DIRT), MAX_STACK);
        assert_eq!(inventory.add(DIRT, 1), 0);
        assert_eq!(inventory.add(STONE, u16::MAX), MAX_STACK);
        // One slot per type.
        assert_eq!(inventory.slots().len(), 4);
    }

    #[test]
    fn new_type_with_no_free_slot_adds_nothing() {
        let mut inventory = Inventory {
            capacity: 3,
            ..Inventory::default()
        };
        assert_eq!(inventory.add(DIRT, 5), 5);
        assert_eq!(inventory.add(STONE, 5), 0);
        assert_eq!(inventory.room_for(STONE), 0);
        // A type that already has a slot still stacks.
        assert_eq!(inventory.add(DIRT, 5), 5);
        // Freeing the slot makes room again.
        assert!(inventory.remove(DIRT, 10));
        assert_eq!(inventory.add(STONE, 5), 5);
    }

    #[test]
    fn unknown_items_and_zero_counts_are_never_stored() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(EMPTY, 5), 0);
        assert_eq!(inventory.add(ItemId(u16::MAX), 5), 0);
        assert_eq!(inventory.add(DIRT, 0), 0);
        assert_eq!(inventory, Inventory::default());
    }

    #[test]
    fn remove_is_all_or_nothing() {
        let mut inventory = Inventory::default();
        inventory.add(DIRT, 10);
        let before = inventory.clone();
        assert!(!inventory.remove(DIRT, 11));
        assert!(!inventory.remove(DIRT, 0));
        assert!(!inventory.remove(SAND, 1));
        assert_eq!(inventory, before);
        assert!(inventory.remove(DIRT, 4));
        assert_eq!(inventory.count(DIRT), 6);
        assert!(inventory.remove(DIRT, 6));
        assert_eq!(inventory, Inventory::default());
    }

    #[test]
    fn permanent_items_cannot_be_removed() {
        let mut inventory = Inventory::default();
        for def in ITEMS.iter().filter(|def| def.has(PERMANENT)) {
            assert!(!inventory.remove(def.id, 1), "{}", def.name);
        }
        assert_eq!(inventory, Inventory::default());
    }
}
