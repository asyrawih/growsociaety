//! Item mods (docs/knowledge/18): what worn equipment does besides looking good. A mod only counts
//! in the slot it belongs to; `worn_mods` folds everything worn into one `Mods` that `physics::step_with`
//! and `rules` read, so the client's prediction and the server agree.

use crate::TILE_SIZE;
use crate::items::{EQUIPMENT, ItemId, SLOT_COUNT, Slot, item, pickaxe_tier, slot_of};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Mod {
    ExtraJump,
    Glide,
    /// Percent faster walking.
    RunBoost(u8),
    /// Damage points per punch (see `DAMAGE_PER_HIT`).
    DigBoost(u8),
    /// Extra tiles of reach.
    Reach(u8),
    /// Added to the upper bound of every gem roll.
    GemFinder(u8),
}

impl Mod {
    /// The one slot this mod works in.
    pub fn slot(self) -> Slot {
        match self {
            Mod::ExtraJump | Mod::Glide => Slot::Back,
            Mod::RunBoost(_) => Slot::Shoes,
            Mod::DigBoost(_) | Mod::Reach(_) => Slot::Hand,
            Mod::GemFinder(_) => Slot::Face,
        }
    }
}

/// Damage points one hit of a block's `hits` is worth; block health = `hits * DAMAGE_PER_HIT`.
pub const DAMAGE_PER_HIT: u16 = 4;
/// Points a bare fist does per punch.
pub const FIST_DAMAGE: u16 = 4;
pub const EXTRA_JUMP_VELOCITY: f32 = 15.0 * TILE_SIZE;
pub const GLIDE_FALL_SPEED: f32 = 3.0 * TILE_SIZE;
/// Damage points per punch for pickaxe tiers 1..=4 (`items::PICKAXE_TIERS`). Tuning.
pub const DIG_POINTS: [u16; 4] = [4, 5, 6, 8];

/// Equipment folder name -> mods. Pickaxes get DigBoost from their tier on top. Tuning.
pub const ITEM_MODS: &[(&str, &[Mod])] = &[
    ("wings_angel_white", &[Mod::ExtraJump]),
    ("wings_bat_black", &[Mod::ExtraJump]),
    ("wings_cosmic", &[Mod::ExtraJump]),
    ("wings_demon_red", &[Mod::ExtraJump]),
    ("wings_dragon_blue", &[Mod::ExtraJump]),
    ("wings_eagle_brown", &[Mod::ExtraJump]),
    ("wings_feather_gold", &[Mod::ExtraJump]),
    ("wings_flame_red", &[Mod::ExtraJump]),
    ("wings_raven_black", &[Mod::ExtraJump]),
    ("wings_steel", &[Mod::ExtraJump]),
    ("wings_bone", &[Mod::Glide]),
    ("wings_butterfly_orange", &[Mod::Glide]),
    ("wings_cloud_white", &[Mod::Glide]),
    ("wings_crystal_purple", &[Mod::Glide]),
    ("wings_dragonfly_cyan", &[Mod::Glide]),
    ("wings_fairy_pink", &[Mod::Glide]),
    ("wings_frost_blue", &[Mod::Glide]),
    ("wings_leaf_green", &[Mod::Glide]),
    ("wings_rainbow", &[Mod::Glide]),
    ("shoes_sneakers_red", &[Mod::RunBoost(20)]),
    ("shoes_boots_iron", &[Mod::RunBoost(10)]),
    ("pickaxe_drill", &[Mod::Reach(1)]),
    ("face_goggles", &[Mod::GemFinder(1)]),
];

/// Everything worn, folded together. Defaults: fist damage, no extras.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Mods {
    pub extra_jump: bool,
    pub glide: bool,
    pub run_pct: u8,
    pub dig: u16,
    pub reach: u8,
    pub gem: u8,
}

impl Default for Mods {
    fn default() -> Self {
        Mods {
            extra_jump: false,
            glide: false,
            run_pct: 0,
            dig: FIST_DAMAGE,
            reach: 0,
            gem: 0,
        }
    }
}

/// An item's mods: its `ITEM_MODS` row, plus DigBoost from a pickaxe tier.
pub fn mods_of(id: ItemId) -> impl Iterator<Item = Mod> {
    let listed = item(id).and_then(|def| ITEM_MODS.iter().find(|(name, _)| *name == def.name));
    let dig = pickaxe_tier(id)
        .and_then(|tier| DIG_POINTS.get(usize::from(tier.power).saturating_sub(1)))
        .map(|&points| Mod::DigBoost(u8::try_from(points).unwrap_or(u8::MAX)));
    listed
        .into_iter()
        .flat_map(|(_, mods)| mods.iter().copied())
        .chain(dig)
}

/// The mods of what is worn; a mod on the wrong slot does nothing.
pub fn worn_mods(equipped: &[Option<ItemId>; SLOT_COUNT]) -> Mods {
    let mut mods = Mods::default();
    for (slot, worn) in Slot::ALL.into_iter().zip(equipped) {
        let Some(id) = *worn else { continue };
        if slot_of(id) != Some(slot) {
            continue;
        }
        for m in mods_of(id).filter(|m| m.slot() == slot) {
            match m {
                Mod::ExtraJump => mods.extra_jump = true,
                Mod::Glide => mods.glide = true,
                Mod::RunBoost(pct) => mods.run_pct = mods.run_pct.max(pct),
                Mod::DigBoost(points) => mods.dig = mods.dig.max(u16::from(points)),
                Mod::Reach(tiles) => mods.reach = mods.reach.max(tiles),
                Mod::GemFinder(bonus) => mods.gem = mods.gem.max(bonus),
            }
        }
    }
    mods
}

/// The `ItemId` of an equipment folder name (400 + its index). Panics at compile time if missing.
pub const fn equip(name: &str) -> ItemId {
    let mut i = 0;
    while i < EQUIPMENT.len() {
        let candidate = EQUIPMENT[i].as_bytes();
        let wanted = name.as_bytes();
        if candidate.len() == wanted.len() {
            let mut j = 0;
            while j < wanted.len() && candidate[j] == wanted[j] {
                j += 1;
            }
            if j == wanted.len() {
                return ItemId(crate::items::EQUIP_ID_BASE + i as u16);
            }
        }
        i += 1;
    }
    panic!("no such equipment name");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mod_row_is_real_equipment_on_an_allowed_slot() {
        for (name, mods) in ITEM_MODS {
            let id = equip(name);
            let slot = slot_of(id).expect("equipment");
            assert!(mods.len() <= 2, "{name}");
            for m in *mods {
                assert_eq!(m.slot(), slot, "{name}: {m:?}");
            }
        }
    }

    #[test]
    fn worn_mods_default_to_the_fist_and_ignore_the_wrong_slot() {
        let mut equipped = [None; SLOT_COUNT];
        assert_eq!(worn_mods(&equipped), Mods::default());
        assert_eq!(worn_mods(&equipped).dig, FIST_DAMAGE);
        // Wings in the shoe slot do nothing.
        equipped[Slot::Shoes as usize] = Some(equip("wings_dragon_blue"));
        assert_eq!(worn_mods(&equipped), Mods::default());
        equipped[Slot::Back as usize] = Some(equip("wings_dragon_blue"));
        equipped[Slot::Shoes as usize] = Some(equip("shoes_sneakers_red"));
        equipped[Slot::Hand as usize] = Some(equip("pickaxe_drill"));
        let mods = worn_mods(&equipped);
        assert!(mods.extra_jump && !mods.glide);
        assert_eq!((mods.run_pct, mods.reach), (20, 1));
        assert!(mods.dig > FIST_DAMAGE, "the drill's tier digs harder");
    }
}
