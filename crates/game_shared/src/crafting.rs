//! Crafting (docs/knowledge/18): a fixed recipe table. `rules::apply` (`Action::Craft`) checks the
//! station, the inputs and room for the output on a cloned inventory, then swaps inputs for the
//! output in one step.

use crate::items::{
    COBBLESTONE, CRATE, FERN, GLASS, GRAVEL, ICE, ItemId, LADDER, LOG_END, MOSSY_STONE, PLANKS,
    SAND, STONE, STONE_BRICKS, STONE_PLATFORM, VINE_SOIL, WOOD_PLATFORM, WORKBENCH,
};
use crate::mods::equip;
use crate::rules::REACH;

#[derive(Clone, Copy, Debug)]
pub struct Recipe {
    /// The station block that must be within `CRAFT_STATION_REACH`; `None` crafts anywhere.
    pub station: Option<ItemId>,
    pub inputs: &'static [(ItemId, u16)],
    pub output: (ItemId, u16),
}

/// The station must be within normal reach; the Reach mod does not extend it.
pub const CRAFT_STATION_REACH: u32 = REACH;

const W: Option<ItemId> = Some(WORKBENCH);

/// Reference: a recipe's index is its id in `Action::Craft`. Append only.
pub const RECIPES: &[Recipe] = &[
    Recipe {
        station: None,
        inputs: &[(PLANKS, 10), (LOG_END, 4)],
        output: (WORKBENCH, 1),
    },
    Recipe {
        station: W,
        inputs: &[(PLANKS, 8), (LOG_END, 2)],
        output: (CRATE, 2),
    },
    Recipe {
        station: W,
        inputs: &[(PLANKS, 6)],
        output: (LADDER, 4),
    },
    Recipe {
        station: W,
        inputs: &[(PLANKS, 8), (LOG_END, 4)],
        output: (equip("pickaxe_wood"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(equip("pickaxe_wood"), 1), (STONE, 20), (COBBLESTONE, 10)],
        output: (equip("pickaxe_stone"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(equip("pickaxe_wood"), 1), (GRAVEL, 20), (STONE_BRICKS, 10)],
        output: (equip("pickaxe_copper"), 1),
    },
    Recipe {
        station: W,
        inputs: &[
            (equip("pickaxe_stone"), 1),
            (STONE_BRICKS, 30),
            (MOSSY_STONE, 20),
        ],
        output: (equip("pickaxe_iron"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(equip("pickaxe_iron"), 1), (SAND, 50), (GLASS, 30)],
        output: (equip("pickaxe_gold"), 1),
    },
    Recipe {
        station: W,
        inputs: &[
            (equip("pickaxe_iron"), 1),
            (VINE_SOIL, 40),
            (MOSSY_STONE, 40),
        ],
        output: (equip("pickaxe_emerald"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(equip("pickaxe_gold"), 1), (ICE, 100), (GLASS, 60)],
        output: (equip("pickaxe_diamond"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(equip("pickaxe_gold"), 1), (STONE_BRICKS, 80), (CRATE, 40)],
        output: (equip("pickaxe_drill"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(STONE_BRICKS, 40), (CRATE, 10)],
        output: (equip("shoes_boots_iron"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(VINE_SOIL, 30), (FERN, 20)],
        output: (equip("shoes_sneakers_red"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(GLASS, 40), (ICE, 20)],
        output: (equip("face_goggles"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(FERN, 100), (VINE_SOIL, 50)],
        output: (equip("wings_leaf_green"), 1),
    },
    Recipe {
        station: W,
        inputs: &[(CRATE, 150), (GLASS, 100), (ICE, 50)],
        output: (equip("wings_feather_gold"), 1),
    },
    // docs/knowledge/ux/platforms.md
    Recipe {
        station: W,
        inputs: &[(PLANKS, 2)],
        output: (WOOD_PLATFORM, 4),
    },
    Recipe {
        station: W,
        inputs: &[(STONE_BRICKS, 2), (PLANKS, 1)],
        output: (STONE_PLATFORM, 4),
    },
];

pub fn recipe(id: u16) -> Option<&'static Recipe> {
    RECIPES.get(usize::from(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::MAX_STACK;
    use crate::items::item;

    #[test]
    fn every_recipe_uses_real_items_and_sane_counts() {
        assert_eq!(RECIPES.len(), 18);
        for (i, r) in RECIPES.iter().enumerate() {
            assert!(
                r.station.is_none_or(|s| item(s).is_some()),
                "recipe {i}: station"
            );
            for &(id, count) in r.inputs.iter().chain([&r.output]) {
                assert!(item(id).is_some(), "recipe {i}: {id:?}");
                assert!(
                    (1..=MAX_STACK).contains(&count),
                    "recipe {i}: count {count}"
                );
            }
        }
        assert!(
            RECIPES[0].station.is_none(),
            "the workbench itself needs no station"
        );
        assert!(recipe(RECIPES.len() as u16).is_none());
    }
}
