//! Claims: which tiles a claim block owns, and the one permission rule (docs/knowledge/07).

use crate::items::{ItemId, LARGE_CLAIM, MEDIUM_CLAIM, SMALL_CLAIM, WORLD_CLAIM};
use crate::rules::PlayerId;
use crate::world::World;
use crate::{WORLD_HEIGHT, WORLD_WIDTH};

/// A claim block's state, stored in `World::extras` under the block's tile key.
#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Claim {
    pub owner: PlayerId,
    pub access: Vec<PlayerId>,
    pub public: bool,
    /// Tile keys this claim owns; empty for a world claim. Fixed at placement.
    pub tiles: Vec<u32>,
}

impl Claim {
    /// Owner, anyone on the access list, or everyone if the claim is public.
    pub fn admits(&self, player: PlayerId) -> bool {
        self.public || self.owner == player || self.access.contains(&player)
    }
}

/// One row of the store's claim list.
#[derive(Clone, Copy, Debug)]
pub struct ClaimKind {
    pub item: ItemId,
    /// Tiles an area claim takes, its own included. `None`: a world claim.
    pub tiles: Option<u32>,
    /// Gems. Reference.
    pub price: u32,
}

/// Reference. The only place claim sizes and prices are written.
pub const CLAIMS: &[ClaimKind] = &[
    ClaimKind {
        item: SMALL_CLAIM,
        tiles: Some(10),
        price: 50,
    },
    ClaimKind {
        item: MEDIUM_CLAIM,
        tiles: Some(48),
        price: 200,
    },
    ClaimKind {
        item: LARGE_CLAIM,
        tiles: Some(200),
        price: 500,
    },
    ClaimKind {
        item: WORLD_CLAIM,
        tiles: None,
        price: 2000,
    },
];

pub fn claim_kind(item: ItemId) -> Option<&'static ClaimKind> {
    CLAIMS.iter().find(|kind| kind.item == item)
}

/// A claim's access list cannot grow past this. Tuning; bounds what one player can make the world store.
pub const MAX_CLAIM_ACCESS: usize = 32;

/// THE permission rule. Every code path that changes a tile asks this, from inside `rules::apply`.
///
/// 1. Inside an area claim: that claim decides.
/// 2. Otherwise, in a world with a world claim: the world claim decides.
/// 3. Otherwise the tile is free.
pub fn can_edit(world: &World, player: PlayerId, tile: u32) -> bool {
    let area = world.claim_of.get(&tile).copied();
    match area.or(world.world_claim) {
        // An index entry without its claim cannot happen; if it ever does, stay locked rather than open.
        Some(key) => world.claim(key).is_some_and(|claim| claim.admits(player)),
        None => true,
    }
}

/// The tiles a new area claim at `(x, y)` takes: every tile sorted by (Chebyshev distance to the
/// claim, row, column), taking those no area claim owns yet until `count` are found.
pub fn claim_area(world: &World, x: u32, y: u32, count: u32) -> Vec<u32> {
    // ponytail: sorts the whole world (5,000 tiles) per placement; walk outward ring by ring if worlds grow.
    let mut candidates: Vec<(u32, u32, u32)> = (0..WORLD_HEIGHT)
        .flat_map(|row| (0..WORLD_WIDTH).map(move |column| (row, column)))
        .map(|(row, column)| (column.abs_diff(x).max(row.abs_diff(y)), row, column))
        .collect();
    candidates.sort_unstable();
    candidates
        .into_iter()
        .filter_map(|(_, row, column)| World::tile_key(column, row))
        .filter(|key| !world.claim_of.contains_key(key))
        .take(count as usize)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const OWNER: PlayerId = PlayerId(1);
    const FRIEND: PlayerId = PlayerId(2);
    const STRANGER: PlayerId = PlayerId(3);

    fn key(x: u32, y: u32) -> u32 {
        World::tile_key(x, y).unwrap()
    }

    fn place(world: &mut World, x: u32, y: u32, count: u32, owner: PlayerId) -> Vec<u32> {
        let tiles = claim_area(world, x, y, count);
        let claim = Claim {
            owner,
            access: Vec::new(),
            public: false,
            tiles: tiles.clone(),
        };
        world.insert_claim(key(x, y), claim);
        tiles
    }

    #[test]
    fn claim_table_matches_the_reference() {
        let rows: Vec<_> = CLAIMS
            .iter()
            .map(|kind| (kind.item.0, kind.tiles, kind.price))
            .collect();
        assert_eq!(
            rows,
            vec![
                (200, Some(10), 50),
                (201, Some(48), 200),
                (202, Some(200), 500),
                (203, None, 2000)
            ]
        );
        assert!(claim_kind(ItemId(1)).is_none());
    }

    #[test]
    fn area_claims_own_exactly_their_count_starting_with_their_own_tile() {
        let world = World::empty("T".into());
        for count in [10, 48, 200] {
            let tiles = claim_area(&world, 50, 25, count);
            assert_eq!(tiles.len(), count as usize);
            assert_eq!(tiles[0], key(50, 25));
            assert_eq!(tiles.iter().collect::<HashSet<_>>().len(), tiles.len());
        }
        // Small: the claim tile, the 8 around it would make 9, so one more from the next ring:
        // lowest row first, then lowest column.
        let small: HashSet<u32> = claim_area(&world, 50, 25, 10).into_iter().collect();
        let mut expected: HashSet<u32> = (24..=26)
            .flat_map(|y| (49..=51).map(move |x| key(x, y)))
            .collect();
        expected.insert(key(48, 23));
        assert_eq!(small, expected);
        // Large: roughly a 14x14 square, so nothing further than 7 tiles away.
        for tile in claim_area(&world, 50, 25, 200) {
            let (x, y) = World::tile_xy(tile).unwrap();
            assert!(x.abs_diff(50).max(y.abs_diff(25)) <= 7);
        }
    }

    #[test]
    fn adjacent_claims_are_disjoint_and_keep_their_count() {
        let mut world = World::empty("T".into());
        let first: HashSet<u32> = place(&mut world, 50, 25, 10, OWNER).into_iter().collect();
        let second: HashSet<u32> = place(&mut world, 51, 25, 10, FRIEND).into_iter().collect();
        assert_eq!((first.len(), second.len()), (10, 10));
        assert!(first.is_disjoint(&second));
        // Its own tile was already taken by the first claim, so the second bends around it.
        assert!(first.contains(&key(51, 25)) && !second.contains(&key(51, 25)));
        assert_eq!(world.claim_of.len(), 20);
    }

    #[test]
    fn a_claim_in_a_corner_keeps_its_full_count() {
        let world = World::empty("T".into());
        for (x, y) in [
            (0, 0),
            (WORLD_WIDTH - 1, 0),
            (0, WORLD_HEIGHT - 1),
            (WORLD_WIDTH - 1, WORLD_HEIGHT - 1),
        ] {
            for count in [10, 48, 200] {
                let tiles = claim_area(&world, x, y, count);
                assert_eq!(tiles.len(), count as usize, "({x},{y})");
                assert_eq!(tiles[0], key(x, y));
            }
        }
        // A full world gives what is left, never more.
        assert_eq!(
            claim_area(&world, 0, 0, u32::MAX).len(),
            (WORLD_WIDTH * WORLD_HEIGHT) as usize
        );
    }

    #[test]
    fn permission_follows_the_three_step_rule() {
        let mut world = World::empty("T".into());
        let free = key(90, 40);
        assert!(can_edit(&world, STRANGER, free));

        // 1. Area claim: owner, access list, public.
        let tiles = place(&mut world, 50, 25, 10, OWNER);
        let inside = tiles[5];
        assert!(can_edit(&world, OWNER, inside));
        assert!(!can_edit(&world, FRIEND, inside) && !can_edit(&world, STRANGER, inside));
        assert!(can_edit(&world, STRANGER, free));

        // 2. World claim covers every tile no area claim owns.
        world.insert_claim(
            key(10, 10),
            Claim {
                owner: FRIEND,
                access: vec![STRANGER],
                public: false,
                tiles: Vec::new(),
            },
        );
        assert_eq!(world.world_claim, Some(key(10, 10)));
        assert!(can_edit(&world, FRIEND, free) && can_edit(&world, STRANGER, free));
        assert!(!can_edit(&world, OWNER, free));
        // The area claim still decides for its own tiles, even against the world owner.
        assert!(can_edit(&world, OWNER, inside) && !can_edit(&world, FRIEND, inside));

        // 3. Removing the claims frees everything again, and the index follows.
        world.remove_extra(key(50, 25));
        assert!(world.claim_of.is_empty());
        assert!(can_edit(&world, FRIEND, inside) && !can_edit(&world, OWNER, inside));
        world.remove_extra(key(10, 10));
        assert_eq!(world.world_claim, None);
        assert!(can_edit(&world, OWNER, inside));
    }

    #[test]
    fn a_dangling_index_entry_locks_the_tile() {
        let mut world = World::empty("T".into());
        world.claim_of.insert(key(5, 5), key(6, 6));
        assert!(!can_edit(&world, OWNER, key(5, 5)));
    }

    #[test]
    fn the_index_rebuilt_from_extras_equals_the_incremental_one() {
        let mut world = World::empty("T".into());
        place(&mut world, 50, 25, 200, OWNER);
        place(&mut world, 52, 25, 48, FRIEND);
        place(&mut world, 0, 0, 10, OWNER);
        world.insert_claim(
            key(99, 49),
            Claim {
                owner: OWNER,
                access: Vec::new(),
                public: true,
                tiles: Vec::new(),
            },
        );
        assert_eq!(
            world.claim_index_from_extras(),
            (world.claim_of.clone(), world.world_claim)
        );
        world.remove_extra(key(52, 25));
        assert_eq!(
            world.claim_index_from_extras(),
            (world.claim_of.clone(), world.world_claim)
        );
        assert_eq!(world.claim_of.len(), 210);
    }
}
