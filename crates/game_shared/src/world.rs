//! Tile storage. `tiles` is private so bounds are checked in one place.

use std::collections::HashMap;

use crate::claims::Claim;
use crate::farming::Tree;
use crate::items::ItemId;
use crate::{WORLD_HEIGHT, WORLD_WIDTH};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tile {
    pub fg: ItemId, // 0 = empty
    pub bg: ItemId, // 0 = empty
}

/// A stack of items lying on a tile. It has no physics and belongs to nobody.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Drop {
    pub id: u32,
    pub item: ItemId,
    pub count: u16,
    pub x: u32,
    pub y: u32,
}

/// Per-tile state that most tiles do not have.
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TileExtra {
    /// The tile's fg holds the tree's seed id.
    Tree(Tree),
    /// The tile's fg holds the claim block.
    Claim(Claim),
}

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct World {
    pub name: String,
    tiles: Vec<Tile>, // WORLD_WIDTH * WORLD_HEIGHT, row-major, row 0 = bottom
    /// By tile key (`World::tile_key`).
    pub extras: HashMap<u32, TileExtra>,
    /// Tile key -> key of the area claim that owns it. Derived from `extras`, never persisted;
    /// `insert_claim` and `remove_extra` keep it in step.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub claim_of: HashMap<u32, u32>,
    /// Key of the world claim block, if the world has one. Derived like `claim_of`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub world_claim: Option<u32>,
    pub drops: Vec<Drop>,
    /// Id the next new drop gets; ids are never reused within a world.
    pub next_drop_id: u32,
    pub door: (u32, u32),
}

impl World {
    /// An all-empty world. Real worlds come from `worldgen::generate`.
    pub(crate) fn empty(name: String) -> Self {
        Self {
            name,
            tiles: vec![Tile::default(); (WORLD_WIDTH * WORLD_HEIGHT) as usize],
            extras: HashMap::new(),
            claim_of: HashMap::new(),
            world_claim: None,
            drops: Vec::new(),
            next_drop_id: 0,
            door: (0, 0),
        }
    }

    /// Row-major tile index, the key of per-tile maps. `None` out of bounds.
    pub fn tile_key(x: u32, y: u32) -> Option<u32> {
        // Lazy on purpose: the multiply must not run for out-of-range coordinates.
        (x < WORLD_WIDTH && y < WORLD_HEIGHT).then(|| y * WORLD_WIDTH + x)
    }

    fn index(x: u32, y: u32) -> Option<usize> {
        Self::tile_key(x, y).map(|key| key as usize)
    }

    /// Inverse of `tile_key`.
    pub fn tile_xy(key: u32) -> Option<(u32, u32)> {
        (key < WORLD_WIDTH * WORLD_HEIGHT).then_some((key % WORLD_WIDTH, key / WORLD_WIDTH))
    }

    /// The tree planted on tile `(x, y)`, if any.
    pub fn tree(&self, x: u32, y: u32) -> Option<&Tree> {
        match self.extras.get(&Self::tile_key(x, y)?) {
            Some(TileExtra::Tree(tree)) => Some(tree),
            _ => None,
        }
    }

    /// The claim whose block stands on the tile with this key, if any.
    pub fn claim(&self, key: u32) -> Option<&Claim> {
        match self.extras.get(&key) {
            Some(TileExtra::Claim(claim)) => Some(claim),
            _ => None,
        }
    }

    /// Stores a claim under its block's tile key and indexes it. A claim without tiles is the world claim.
    pub fn insert_claim(&mut self, key: u32, claim: Claim) {
        self.remove_extra(key);
        if claim.tiles.is_empty() {
            self.world_claim = Some(key);
        }
        for &tile in &claim.tiles {
            self.claim_of.insert(tile, key);
        }
        self.extras.insert(key, TileExtra::Claim(claim));
    }

    /// Removes whatever extra the tile has. A removed claim frees its tiles.
    pub fn remove_extra(&mut self, key: u32) -> Option<TileExtra> {
        let extra = self.extras.remove(&key)?;
        if let TileExtra::Claim(_) = extra {
            self.claim_of.retain(|_, owner| *owner != key);
            if self.world_claim == Some(key) {
                self.world_claim = None;
            }
        }
        Some(extra)
    }

    /// `claim_of` and `world_claim` computed from `extras` alone: what a freshly loaded world gets.
    pub fn claim_index_from_extras(&self) -> (HashMap<u32, u32>, Option<u32>) {
        let mut claim_of = HashMap::new();
        let mut world_claim = None;
        for (&key, extra) in &self.extras {
            let TileExtra::Claim(claim) = extra else {
                continue;
            };
            if claim.tiles.is_empty() {
                world_claim = Some(key);
            }
            claim_of.extend(claim.tiles.iter().map(|&tile| (tile, key)));
        }
        (claim_of, world_claim)
    }

    pub fn get(&self, x: u32, y: u32) -> Option<Tile> {
        self.tiles.get(Self::index(x, y)?).copied()
    }

    /// Returns `false` (and changes nothing) when out of bounds.
    pub fn set(&mut self, x: u32, y: u32, tile: Tile) -> bool {
        match Self::index(x, y).and_then(|i| self.tiles.get_mut(i)) {
            Some(slot) => {
                *slot = tile;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_and_set_are_bounds_checked() {
        let mut world = World::empty("T".into());
        let tile = Tile {
            fg: ItemId(1),
            bg: ItemId(100),
        };
        assert!(world.set(WORLD_WIDTH - 1, WORLD_HEIGHT - 1, tile));
        assert_eq!(world.get(WORLD_WIDTH - 1, WORLD_HEIGHT - 1), Some(tile));
        assert_eq!(world.get(0, 0), Some(Tile::default()));

        for (x, y) in [
            (WORLD_WIDTH, 0),
            (0, WORLD_HEIGHT),
            (u32::MAX, 0),
            (0, u32::MAX),
            (u32::MAX, u32::MAX),
        ] {
            assert_eq!(world.get(x, y), None);
            assert!(!world.set(x, y, tile));
        }
    }
}
