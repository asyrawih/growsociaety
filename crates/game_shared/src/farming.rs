//! Trees: grow time, growth stage and the splice recipes (docs/knowledge/06). Planting and
//! harvesting themselves are `Place` and `Punch` in `rules`.

use crate::items::{
    CAVE_WALL, COBBLESTONE, CRATE, DIRT, FERN, GLASS, GRAVEL, ICE, ItemId, LADDER, LOG_END,
    MOSSY_STONE, PLANKS, SAND, STONE, STONE_BRICK_WALL, STONE_BRICKS, VINE_SOIL, WOOD_WALL, item,
};

/// `grow_seconds(rarity) = GROW_CUBIC * rarity^3 + GROW_LINEAR * rarity`. Reference.
pub const GROW_CUBIC: u64 = 1;
pub const GROW_LINEAR: u64 = 30;
/// Blocks a tree gives, rolled uniformly when it is planted. Tuning.
pub const TREE_YIELD_MIN: u8 = 2;
pub const TREE_YIELD_MAX: u8 = 5;
/// Chance that a harvest also drops a seed of the same item. Tuning.
pub const TREE_SEED_CHANCE: f32 = 0.20;
/// Punches that destroy a sapling that is not grown yet. Tuning.
pub const SAPLING_HITS: u8 = 3;
/// A grown tree is at the last stage; a sapling moves through the ones before it.
pub const GROWN_STAGE: u8 = 5;

#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tree {
    /// The block this tree will give.
    pub item: ItemId,
    /// The `now` of the `apply` call that planted or spliced it.
    pub planted_at: f64,
    pub fruit: u8,
    pub spliced: bool,
}

/// `(a, b, result)`. Order of `a` and `b` does not matter. Ours; placeholders the owner can redesign.
pub const RECIPES: &[(ItemId, ItemId, ItemId)] = &[
    (DIRT, STONE, GRAVEL),
    (STONE, CAVE_WALL, COBBLESTONE),
    (SAND, STONE, GLASS),
    (DIRT, CAVE_WALL, VINE_SOIL),
    (DIRT, SAND, FERN),
    (GRAVEL, STONE, STONE_BRICKS),
    (COBBLESTONE, DIRT, MOSSY_STONE),
    (GLASS, SAND, ICE),
    (FERN, DIRT, LOG_END),
    (LOG_END, STONE, PLANKS),
    (STONE_BRICKS, CAVE_WALL, STONE_BRICK_WALL),
    (PLANKS, CAVE_WALL, WOOD_WALL),
    (PLANKS, GRAVEL, CRATE),
    (PLANKS, STONE_BRICKS, LADDER),
];

pub fn grow_seconds(rarity: u16) -> u64 {
    let rarity = u64::from(rarity);
    GROW_CUBIC * rarity * rarity * rarity + GROW_LINEAR * rarity
}

/// Seconds this tree needs from `planted_at`. `None` if its item is not in the table.
fn grow_time(tree: &Tree) -> Option<f64> {
    item(tree.item).map(|def| grow_seconds(def.rarity) as f64)
}

/// A comparison, not a ticking timer: trees grow while nobody is watching.
pub fn is_grown(tree: &Tree, now: f64) -> bool {
    grow_time(tree).is_some_and(|needed| now - tree.planted_at >= needed)
}

/// Which of the six growth sprites to draw: `GROWN_STAGE` when grown, else `floor(progress * GROWN_STAGE)`.
pub fn growth_stage(tree: &Tree, now: f64) -> u8 {
    match grow_time(tree) {
        Some(_) if is_grown(tree, now) => GROWN_STAGE,
        Some(needed) => {
            let progress = ((now - tree.planted_at) / needed).clamp(0.0, 1.0);
            ((progress * f64::from(GROWN_STAGE)) as u8).min(GROWN_STAGE - 1)
        }
        None => 0,
    }
}

/// The item two different items splice into, in either order.
pub fn splice(a: ItemId, b: ItemId) -> Option<ItemId> {
    // ponytail: linear scan, switch to a HashMap if recipes pass a few thousand
    let recipe = RECIPES
        .iter()
        .find(|&&(x, y, _)| (x, y) == (a, b) || (x, y) == (b, a));
    recipe.map(|&(_, _, result)| result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{BEDROCK, EMPTY, seed_of};
    use crate::rules::{DROP_BLOCK, DROP_SEED};
    use std::collections::HashSet;

    fn tree(of: ItemId) -> Tree {
        Tree {
            item: of,
            planted_at: 100.0,
            fruit: 3,
            spliced: false,
        }
    }

    #[test]
    fn grow_seconds_matches_the_reference_points() {
        assert_eq!(grow_seconds(1), 31);
        assert_eq!(grow_seconds(2), 68);
        assert_eq!(grow_seconds(100), 1_003_000);
        assert_eq!(grow_seconds(999), 997_032_969);
        assert_eq!(grow_seconds(u16::MAX), 65535u64.pow(3) + 30 * 65535); // no overflow
    }

    #[test]
    fn grown_exactly_at_the_grow_time() {
        let sapling = tree(GRAVEL); // rarity 2: 68 s
        assert!(!is_grown(&sapling, 100.0));
        assert!(!is_grown(&sapling, 100.0 + 67.0));
        assert!(is_grown(&sapling, 100.0 + 68.0));
        assert!(is_grown(&sapling, 1e12));
        // A clock that runs backwards never grows a tree, and an unknown item never grows.
        assert!(!is_grown(&sapling, 0.0));
        assert!(!is_grown(&tree(ItemId(u16::MAX)), 1e12));
    }

    #[test]
    fn growth_stages_run_from_0_to_5() {
        let sapling = tree(DIRT); // 31 s
        let stage = |elapsed: f64| growth_stage(&sapling, 100.0 + elapsed);
        assert_eq!(stage(-5.0), 0);
        assert_eq!(stage(0.0), 0);
        assert_eq!(stage(6.1), 0);
        assert_eq!(stage(6.3), 1);
        assert_eq!(stage(15.5), 2);
        assert_eq!(stage(30.9), 4);
        assert_eq!(stage(31.0), GROWN_STAGE);
        assert_eq!(stage(1e9), GROWN_STAGE);
    }

    #[test]
    fn every_recipe_is_consistent_with_the_item_table() {
        let mut pairs = HashSet::new();
        let mut results = HashSet::new();
        for &(a, b, result) in RECIPES {
            let [a_def, b_def, result_def] = [a, b, result].map(|id| item(id).expect("row exists"));
            assert_ne!(a, b);
            assert_eq!(
                result_def.rarity,
                a_def.rarity + b_def.rarity,
                "{}",
                result_def.name
            );
            assert!(
                pairs.insert((a.0.min(b.0), a.0.max(b.0))),
                "pair twice: {}",
                result_def.name
            );
            assert!(
                results.insert(result),
                "two recipes for {}",
                result_def.name
            );
            // Parents must be plantable, and the result must be too or it could never be grown.
            for id in [a, b, result] {
                assert!(seed_of(id).is_some(), "{id:?} has no seed");
            }
        }
        assert_eq!(RECIPES.len(), 14);
    }

    #[test]
    fn splice_works_in_both_orders_and_only_for_recipes() {
        for &(a, b, result) in RECIPES {
            assert_eq!(splice(a, b), Some(result));
            assert_eq!(splice(b, a), Some(result));
        }
        assert_eq!(splice(DIRT, DIRT), None);
        assert_eq!(splice(DIRT, BEDROCK), None);
        assert_eq!(splice(EMPTY, EMPTY), None);
        assert_eq!(splice(CRATE, LADDER), None);
    }

    #[test]
    fn a_farm_returns_more_than_one_seed_per_seed_planted() {
        let avg_fruit = f32::from(TREE_YIELD_MIN + TREE_YIELD_MAX) / 2.0;
        // A block placed and broken until it is gone yields DROP_SEED / (1 - DROP_BLOCK) seeds.
        let seeds_back = TREE_SEED_CHANCE + avg_fruit * DROP_SEED / (1.0 - DROP_BLOCK);
        assert!(seeds_back > 1.0, "{seeds_back}");
        assert!((seeds_back - 1.37).abs() < 0.01, "{seeds_back}");
    }
}
