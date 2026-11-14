//! The item table. Behaviour reads `item()`; it never matches on ids (docs/knowledge/03).

#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ItemId(pub u16);

pub const EMPTY: ItemId = ItemId(0);
pub const DIRT: ItemId = ItemId(1);
pub const STONE: ItemId = ItemId(2);
pub const COBBLESTONE: ItemId = ItemId(3);
pub const BEDROCK: ItemId = ItemId(4);
pub const SAND: ItemId = ItemId(5);
pub const ICE: ItemId = ItemId(7);
pub const GRAVEL: ItemId = ItemId(9);
pub const MOSSY_STONE: ItemId = ItemId(10);
pub const VINE_SOIL: ItemId = ItemId(11);
pub const PLANKS: ItemId = ItemId(12);
pub const LOG_END: ItemId = ItemId(13);
pub const STONE_BRICKS: ItemId = ItemId(14);
pub const GLASS: ItemId = ItemId(16);
pub const CRATE: ItemId = ItemId(17);
pub const LAVA: ItemId = ItemId(21);
pub const LADDER: ItemId = ItemId(33);
pub const FIST: ItemId = ItemId(47);
pub const FERN: ItemId = ItemId(62);
pub const CAVE_WALL: ItemId = ItemId(100);
pub const WOOD_WALL: ItemId = ItemId(102);
pub const STONE_BRICK_WALL: ItemId = ItemId(104);
pub const SMALL_CLAIM: ItemId = ItemId(200);
pub const MEDIUM_CLAIM: ItemId = ItemId(201);
pub const LARGE_CLAIM: ItemId = ItemId(202);
pub const WORLD_CLAIM: ItemId = ItemId(203);
pub const MAIN_DOOR: ItemId = ItemId(300);
pub const WRENCH: ItemId = ItemId(301);
/// The starter outfit: the first four `EQUIPMENT` entries (a test checks the names).
pub const HAIR_BROWN: ItemId = ItemId(EQUIP_ID_BASE);
pub const SHIRT_RED: ItemId = ItemId(EQUIP_ID_BASE + 1);
pub const PANTS_JEANS_BLUE: ItemId = ItemId(EQUIP_ID_BASE + 2);
pub const SHOES_BOOTS_BROWN: ItemId = ItemId(EQUIP_ID_BASE + 3);
/// Rule: a new player owns and wears these.
pub const STARTER_OUTFIT: [ItemId; 4] =
    [HAIR_BROWN, SHIRT_RED, PANTS_JEANS_BLUE, SHOES_BOOTS_BROWN];

/// `seed id = SEED_ID_OFFSET + item id`. A seed is found with arithmetic, not a lookup field.
pub const SEED_ID_OFFSET: u16 = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemKind {
    Block,
    Background,
    Seed,
    Claim,
    Tool,
    /// Worn or held by the player, in this slot (docs/knowledge/03, "Equipment").
    Equip(Slot),
    Resource,
}

/// Where a piece of equipment goes. One item per slot; `slot as usize` indexes `Player::equipped`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Slot {
    Hair,
    Hat,
    Face,
    Shirt,
    Pants,
    Shoes,
    Back,
    Hand,
}

pub const SLOT_COUNT: usize = 8;

impl Slot {
    /// Index order, which is also the order of the worn-slots row in the HUD.
    pub const ALL: [Slot; SLOT_COUNT] = [
        Slot::Hair,
        Slot::Hat,
        Slot::Face,
        Slot::Shirt,
        Slot::Pants,
        Slot::Shoes,
        Slot::Back,
        Slot::Hand,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Collision {
    None,
    Solid,
    Platform,
    Ladder,
    Hazard,
    Bounce,
    Liquid,
}

pub const UNBREAKABLE: u8 = 1 << 0;
pub const UNTRADEABLE: u8 = 1 << 1;
pub const NO_SEED: u8 = 1 << 2;
pub const PERMANENT: u8 = 1 << 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sprite {
    /// Terrain index in `assets/tiles/terrain.png`. The icon is the mask-0 cell.
    Terrain(u8),
    /// File name under `assets/blocks/`, 32x32.
    Block(&'static str),
    /// File name under `assets/blocks/`, multiplied by an RGB colour.
    Tinted(&'static str, [u8; 3]),
    /// Folder name under `assets/items/`; the client draws its `icon.png` (every equipment folder has one).
    Equip(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct ItemDef {
    pub id: ItemId,
    pub name: &'static str,
    pub kind: ItemKind,
    pub collision: Collision,
    pub rarity: u16,
    /// Punches to break with a bare fist; 0 for non-placeables. All values are Tuning.
    pub hits: u8,
    pub flags: u8,
    pub sprite: Sprite,
}

impl ItemDef {
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

// One positional column per ItemDef field keeps the table readable as a table.
#[allow(clippy::too_many_arguments)]
const fn def(
    id: ItemId,
    name: &'static str,
    kind: ItemKind,
    collision: Collision,
    rarity: u16,
    hits: u8,
    flags: u8,
    sprite: Sprite,
) -> ItemDef {
    ItemDef {
        id,
        name,
        kind,
        collision,
        rarity,
        hits,
        flags,
        sprite,
    }
}

/// The one greyscale seed sprite; each seed tints it with its block's colour.
const SEED_SPRITE: &str = "66_seed_gray.png";
const CLAIM_SPRITE: &str = "48_padlock.png";
/// Punches to break a claim block. Tuning.
const CLAIM_HITS: u8 = 8;

const fn seed(of: ItemId, name: &'static str, rarity: u16, rgb: [u8; 3]) -> ItemDef {
    def(
        ItemId(SEED_ID_OFFSET + of.0),
        name,
        ItemKind::Seed,
        Collision::None,
        rarity,
        0,
        NO_SEED,
        Sprite::Tinted(SEED_SPRITE, rgb),
    )
}

/// Claims come only from the store (`claims::CLAIMS` has their sizes and prices).
const fn claim(id: ItemId, name: &'static str, rgb: [u8; 3]) -> ItemDef {
    def(
        id,
        name,
        ItemKind::Claim,
        Collision::Solid,
        0,
        CLAIM_HITS,
        NO_SEED,
        Sprite::Tinted(CLAIM_SPRITE, rgb),
    )
}

const TOOL_FLAGS: u8 = UNTRADEABLE | PERMANENT | NO_SEED;
const FIXTURE_FLAGS: u8 = UNBREAKABLE | NO_SEED;

use Collision::{Hazard, Solid};
use ItemKind::{Background, Block, Tool};

#[rustfmt::skip]
const BASE_ITEMS: &[ItemDef] = &[
    def(DIRT,      "dirt",      Block,      Solid,           1, 3, 0,             Sprite::Terrain(1)),
    def(STONE,     "stone",     Block,      Solid,           1, 6, 0,             Sprite::Terrain(2)),
    def(BEDROCK,   "bedrock",   Block,      Solid,           0, 0, FIXTURE_FLAGS, Sprite::Block("04_bedrock.png")),
    def(SAND,      "sand",      Block,      Solid,           1, 2, 0,             Sprite::Terrain(3)),
    def(LAVA,      "lava",      Block,      Hazard,          2, 4, 0,             Sprite::Block("21_lava.png")),
    def(FIST,      "fist",      Tool,       Collision::None, 0, 0, TOOL_FLAGS,    Sprite::Block("47_fist.png")),
    def(CAVE_WALL, "cave wall", Background, Collision::None, 1, 2, 0,             Sprite::Block("bg_dirt.png")),
    def(MAIN_DOOR, "main door", Block,      Collision::None, 0, 0, FIXTURE_FLAGS, Sprite::Block("65_door.png")),
    def(WRENCH,    "wrench",    Tool,       Collision::None, 0, 0, TOOL_FLAGS,    Sprite::Block("64_wrench.png")),
    // Splice results (docs/knowledge/06). Rarity = sum of the parents' rarities.
    def(GRAVEL,           "gravel",           Block,      Solid,             2, 3, 0, Sprite::Block("09_gravel.png")),
    def(COBBLESTONE,      "cobblestone",      Block,      Solid,             2, 5, 0, Sprite::Block("03_cobblestone.png")),
    def(GLASS,            "glass",            Block,      Solid,             2, 2, 0, Sprite::Block("16_glass.png")),
    def(VINE_SOIL,        "vine soil",        Block,      Solid,             2, 3, 0, Sprite::Block("11_vine_soil.png")),
    def(FERN,             "fern",             Block,      Collision::None,   2, 2, 0, Sprite::Block("62_fern.png")),
    def(STONE_BRICKS,     "stone bricks",     Block,      Solid,             3, 7, 0, Sprite::Block("14_stone_bricks.png")),
    def(MOSSY_STONE,      "mossy stone",      Block,      Solid,             3, 6, 0, Sprite::Block("10_mossy_stone.png")),
    def(ICE,              "ice",              Block,      Solid,             3, 3, 0, Sprite::Block("07_ice.png")),
    def(LOG_END,          "log end",          Block,      Solid,             3, 4, 0, Sprite::Block("13_log_end.png")),
    def(PLANKS,           "planks",           Block,      Solid,             4, 4, 0, Sprite::Block("12_planks.png")),
    def(STONE_BRICK_WALL, "stone brick wall", Background, Collision::None,   4, 4, 0, Sprite::Block("bg_stone_bricks.png")),
    def(WOOD_WALL,        "wood wall",        Background, Collision::None,   5, 3, 0, Sprite::Block("bg_wood.png")),
    def(CRATE,            "crate",            Block,      Solid,             6, 5, 0, Sprite::Block("17_crate.png")),
    def(LADDER,           "ladder",           Block,      Collision::Ladder, 7, 3, 0, Sprite::Block("33_ladder.png")),
    seed(DIRT, "dirt seed", 1, [166, 94, 40]),
    seed(STONE, "stone seed", 1, [128, 136, 150]),
    seed(SAND, "sand seed", 1, [232, 190, 104]),
    seed(LAVA, "lava seed", 2, [240, 96, 48]),
    seed(CAVE_WALL, "cave wall seed", 1, [120, 60, 36]),
    seed(GRAVEL, "gravel seed", 2, [176, 158, 140]),
    seed(COBBLESTONE, "cobblestone seed", 2, [96, 92, 104]),
    seed(GLASS, "glass seed", 2, [176, 226, 255]),
    seed(VINE_SOIL, "vine soil seed", 2, [112, 120, 56]),
    seed(FERN, "fern seed", 2, [84, 170, 70]),
    seed(STONE_BRICKS, "stone bricks seed", 3, [190, 186, 204]),
    seed(MOSSY_STONE, "mossy stone seed", 3, [104, 150, 110]),
    seed(ICE, "ice seed", 3, [96, 176, 240]),
    seed(LOG_END, "log end seed", 3, [214, 150, 96]),
    seed(PLANKS, "planks seed", 4, [170, 110, 70]),
    seed(STONE_BRICK_WALL, "stone brick wall seed", 4, [98, 92, 128]),
    seed(WOOD_WALL, "wood wall seed", 5, [92, 72, 60]),
    seed(CRATE, "crate seed", 6, [226, 170, 60]),
    seed(LADDER, "ladder seed", 7, [196, 120, 60]),
    claim(SMALL_CLAIM, "small claim", [205, 127, 50]),
    claim(MEDIUM_CLAIM, "medium claim", [208, 208, 220]),
    claim(LARGE_CLAIM, "large claim", [255, 215, 0]),
    claim(WORLD_CLAIM, "world claim", [90, 220, 255]),
];

/// Every item: the hand-written rows, then one row per `EQUIPMENT` entry built at compile time.
pub const ITEMS: &[ItemDef] = &ALL_ITEMS;

const ALL_ITEMS: [ItemDef; BASE_ITEMS.len() + EQUIPMENT.len()] = all_items();

/// Compile-time only: the indices are constants, so an out-of-range one fails the build.
const fn all_items() -> [ItemDef; BASE_ITEMS.len() + EQUIPMENT.len()] {
    let mut all = [BASE_ITEMS[0]; BASE_ITEMS.len() + EQUIPMENT.len()];
    let mut i = 0;
    while i < BASE_ITEMS.len() {
        all[i] = BASE_ITEMS[i];
        i += 1;
    }
    let mut j = 0;
    while j < EQUIPMENT.len() {
        all[BASE_ITEMS.len() + j] = equip_def(j);
        j += 1;
    }
    all
}

/// Equipment id = `EQUIP_ID_BASE + index` into `EQUIPMENT`. Range `400..=999`.
pub const EQUIP_ID_BASE: u16 = 400;
const EQUIP_ID_LAST: u16 = 999;
const _: () = assert!(EQUIPMENT.len() <= (EQUIP_ID_LAST - EQUIP_ID_BASE + 1) as usize);

/// APPEND-ONLY: ids are persisted and sent over the network. Never reorder or remove an entry;
/// new art goes at the end. Every name is a folder in `assets/items/` (a test checks both ways).
/// The slot comes from the name's prefix (`SLOT_PREFIXES`), the price and power from the tables below.
pub const EQUIPMENT: &[&str] = &[
    "hair_brown",
    "shirt_red",
    "pants_jeans_blue",
    "shoes_boots_brown",
    "face_goggles",
    "hair_afro_black",
    "hair_anime_red",
    "hair_bob_blonde",
    "hair_bob_blue",
    "hair_braids_brown",
    "hair_buns_pink",
    "hair_buzz_brown",
    "hair_curly_ginger",
    "hair_curly_orange",
    "hair_emo_black",
    "hair_fluffy_white",
    "hair_fringe_purple",
    "hair_long_navy",
    "hair_messy_green",
    "hair_mohawk_blue",
    "hair_mohawk_green",
    "hair_pigtails_blonde",
    "hair_ponytail_black",
    "hair_ponytail_brown",
    "hair_slick_grey",
    "hair_spiky_blonde",
    "hair_spiky_orange",
    "hair_topknot_black",
    "hair_undercut_brown",
    "hair_wavy_red",
    "hat_bandana_red",
    "hat_beanie_green",
    "hat_cap_red",
    "hat_knight_helmet",
    "hat_straw",
    "pants_black",
    "pants_camo_green",
    "pants_cargo_khaki",
    "pants_greaves_iron",
    "pants_hiking_olive",
    "pants_jeans_blue2",
    "pants_kilt_red",
    "pants_leather_brown",
    "pants_leather_brown2",
    "pants_ninja_black",
    "pants_overalls_brown",
    "pants_prison_orange",
    "pants_robe_purple",
    "pants_royal_gold",
    "pants_sailor_white",
    "pants_shorts_denim",
    "pants_shorts_yellow",
    "pants_skirt_pink",
    "pants_suit_navy",
    "pants_sweat_grey",
    "pants_track_red",
    "pants_trousers_black2",
    "pickaxe_bone",
    "pickaxe_candy",
    "pickaxe_copper",
    "pickaxe_diamond",
    "pickaxe_drill",
    "pickaxe_emerald",
    "pickaxe_fire",
    "pickaxe_gold",
    "pickaxe_ice",
    "pickaxe_iron",
    "pickaxe_lightning",
    "pickaxe_nature",
    "pickaxe_obsidian",
    "pickaxe_rainbow",
    "pickaxe_royal",
    "pickaxe_ruby",
    "pickaxe_rusty",
    "pickaxe_shadow",
    "pickaxe_stone",
    "pickaxe_wood",
    "shirt_camo_green",
    "shirt_chainmail_grey",
    "shirt_chef_white",
    "shirt_chestplate_iron",
    "shirt_chestplate_iron2",
    "shirt_flannel_red",
    "shirt_hawaiian_orange",
    "shirt_hoodie_blue",
    "shirt_hoodie_blue2",
    "shirt_jacket_leather2",
    "shirt_jersey_cyan",
    "shirt_labcoat_white",
    "shirt_leather_jacket",
    "shirt_ninja_black",
    "shirt_raincoat_yellow",
    "shirt_robe_purple",
    "shirt_sailor_striped",
    "shirt_sailor_stripes",
    "shirt_suit_black",
    "shirt_sweater_pink",
    "shirt_tee_red",
    "shirt_tunic_gold",
    "shirt_tunic_green",
    "shirt_tunic_green2",
    "shirt_vest_farmer",
    "shoes_boots_iron",
    "shoes_sneakers_red",
    "wings_angel_white",
    "wings_bat_black",
    "wings_bone",
    "wings_butterfly_orange",
    "wings_cloud_white",
    "wings_cosmic",
    "wings_crystal_purple",
    "wings_demon_red",
    "wings_dragon_blue",
    "wings_dragonfly_cyan",
    "wings_eagle_brown",
    "wings_fairy_pink",
    "wings_feather_gold",
    "wings_flame_red",
    "wings_frost_blue",
    "wings_leaf_green",
    "wings_rainbow",
    "wings_raven_black",
    "wings_steel",
];

/// Folder-name prefix of each slot.
pub const SLOT_PREFIXES: [(&str, Slot); SLOT_COUNT] = [
    ("hair_", Slot::Hair),
    ("hat_", Slot::Hat),
    ("face_", Slot::Face),
    ("shirt_", Slot::Shirt),
    ("pants_", Slot::Pants),
    ("shoes_", Slot::Shoes),
    ("wings_", Slot::Back),
    ("pickaxe_", Slot::Hand),
];

/// Store prices in gems. Tuning.
pub const HAIR_PRICE: u32 = 150;
/// Shirt, pants and shoes.
pub const CLOTHES_PRICE: u32 = 200;
/// Hat and face.
pub const HEADWEAR_PRICE: u32 = 300;
pub const WINGS_PRICE: u32 = 10_000;

/// Damage one punch adds with no pickaxe in the hand. Tuning.
pub const FIST_POWER: u8 = 1;

/// One pickaxe tier: the damage a punch adds and the store price. Tuning.
#[derive(Clone, Copy, Debug)]
pub struct PickaxeTier {
    pub power: u8,
    pub price: u32,
    pub pickaxes: &'static [&'static str],
}

pub const PICKAXE_TIERS: &[PickaxeTier] = &[
    PickaxeTier {
        power: 1,
        price: 200,
        pickaxes: &[
            "pickaxe_wood",
            "pickaxe_rusty",
            "pickaxe_bone",
            "pickaxe_candy",
        ],
    },
    PickaxeTier {
        power: 2,
        price: 1_000,
        pickaxes: &[
            "pickaxe_stone",
            "pickaxe_copper",
            "pickaxe_iron",
            "pickaxe_ice",
            "pickaxe_nature",
        ],
    },
    PickaxeTier {
        power: 3,
        price: 4_000,
        pickaxes: &[
            "pickaxe_gold",
            "pickaxe_emerald",
            "pickaxe_ruby",
            "pickaxe_fire",
            "pickaxe_lightning",
            "pickaxe_shadow",
        ],
    },
    PickaxeTier {
        power: 4,
        price: 15_000,
        pickaxes: &[
            "pickaxe_diamond",
            "pickaxe_obsidian",
            "pickaxe_drill",
            "pickaxe_rainbow",
            "pickaxe_royal",
        ],
    },
];

const fn has_prefix(name: &str, prefix: &str) -> bool {
    let (name, prefix) = (name.as_bytes(), prefix.as_bytes());
    if prefix.len() > name.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if name[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The slot a folder name belongs to, from its prefix.
const fn slot_of_name(name: &str) -> Option<Slot> {
    let mut i = 0;
    while i < SLOT_PREFIXES.len() {
        let (prefix, slot) = SLOT_PREFIXES[i];
        if has_prefix(name, prefix) {
            return Some(slot);
        }
        i += 1;
    }
    None
}

/// Compile-time only (see `all_items`). A name without a slot prefix fails the build.
const fn equip_def(index: usize) -> ItemDef {
    let name = EQUIPMENT[index];
    let Some(slot) = slot_of_name(name) else {
        panic!("an EQUIPMENT name has no slot prefix");
    };
    def(
        ItemId(EQUIP_ID_BASE + index as u16),
        name,
        ItemKind::Equip(slot),
        Collision::None,
        0,
        0,
        NO_SEED,
        Sprite::Equip(name),
    )
}

/// The slot `id` is worn in, if it is equipment.
pub fn slot_of(id: ItemId) -> Option<Slot> {
    match item(id)?.kind {
        ItemKind::Equip(slot) => Some(slot),
        _ => None,
    }
}

/// The tier of a pickaxe, if `id` is one.
pub fn pickaxe_tier(id: ItemId) -> Option<&'static PickaxeTier> {
    let def = item(id).filter(|def| def.kind == ItemKind::Equip(Slot::Hand))?;
    PICKAXE_TIERS
        .iter()
        .find(|tier| tier.pickaxes.contains(&def.name))
}

/// Damage one punch adds with `hand` held (the `Hand` slot). The fist when empty.
pub fn punch_power(hand: Option<ItemId>) -> u8 {
    hand.and_then(pickaxe_tier)
        .map_or(FIST_POWER, |tier| tier.power)
}

/// Store price of a piece of equipment, by its slot (pickaxes by tier). `None` for anything else.
pub fn equip_price(id: ItemId) -> Option<u32> {
    match slot_of(id)? {
        Slot::Hair => Some(HAIR_PRICE),
        Slot::Shirt | Slot::Pants | Slot::Shoes => Some(CLOTHES_PRICE),
        Slot::Hat | Slot::Face => Some(HEADWEAR_PRICE),
        Slot::Back => Some(WINGS_PRICE),
        Slot::Hand => pickaxe_tier(id).map(|tier| tier.price),
    }
}

/// The seed of `id`, if it has one. Arithmetic on the id, checked against the table.
pub fn seed_of(id: ItemId) -> Option<ItemId> {
    let def = item(id).filter(|def| !def.has(NO_SEED))?;
    let seed = ItemId(def.id.0.checked_add(SEED_ID_OFFSET)?);
    item(seed).map(|seed| seed.id)
}

/// The item a seed grows into, if `seed` is a seed.
pub fn seed_parent(seed: ItemId) -> Option<ItemId> {
    let def = item(seed).filter(|def| def.kind == ItemKind::Seed)?;
    let parent = ItemId(def.id.0.checked_sub(SEED_ID_OFFSET)?);
    item(parent).map(|parent| parent.id)
}

/// The only way systems learn about an item. `None` for `EMPTY` and unknown ids.
pub fn item(id: ItemId) -> Option<&'static ItemDef> {
    // ponytail: linear scan over a tiny table; build an id-indexed lookup if a profile ever shows this.
    ITEMS.iter().find(|def| def.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_item_returns_the_matching_row() {
        let mut seen = HashSet::new();
        for def in ITEMS {
            assert!(seen.insert(def.id), "duplicate id {:?}", def.id);
            assert_eq!(item(def.id).map(|d| d.name), Some(def.name));
        }
        assert!(item(EMPTY).is_none());
        assert!(item(ItemId(u16::MAX)).is_none());
    }

    #[test]
    fn seed_of_follows_the_no_seed_flag() {
        assert_eq!(seed_of(DIRT), Some(ItemId(1001)));
        assert_eq!(seed_of(CAVE_WALL), Some(ItemId(1100)));
        for id in [
            EMPTY,
            BEDROCK,
            MAIN_DOOR,
            FIST,
            ItemId(1001),
            ItemId(u16::MAX),
        ] {
            assert_eq!(seed_of(id), None, "{id:?}");
        }
    }

    #[test]
    fn seed_parent_is_the_inverse_of_seed_of() {
        for def in ITEMS {
            match seed_of(def.id) {
                Some(seed) => assert_eq!(seed_parent(seed), Some(def.id), "{}", def.name),
                None => assert!(def.has(NO_SEED), "{}", def.name),
            }
        }
        for id in [EMPTY, DIRT, FIST, ItemId(999), ItemId(u16::MAX)] {
            assert_eq!(seed_parent(id), None, "{id:?}");
        }
    }

    #[test]
    fn named_constants_have_rows() {
        for id in [BEDROCK, MAIN_DOOR, FIST, WRENCH] {
            assert!(item(id).is_some(), "{id:?}");
        }
    }

    #[test]
    fn placeables_are_breakable_or_flagged_unbreakable() {
        for def in ITEMS {
            if matches!(def.kind, ItemKind::Block | ItemKind::Background) {
                assert!(def.hits > 0 || def.has(UNBREAKABLE), "{}", def.name);
            }
        }
    }

    #[test]
    fn seed_rows_match_no_seed_flag_both_ways() {
        for def in ITEMS {
            if !def.has(NO_SEED) {
                let seed = item(ItemId(SEED_ID_OFFSET + def.id.0));
                assert_eq!(seed.map(|s| s.kind), Some(ItemKind::Seed), "{}", def.name);
            }
            if def.kind == ItemKind::Seed {
                let parent = def
                    .id
                    .0
                    .checked_sub(SEED_ID_OFFSET)
                    .map(ItemId)
                    .and_then(item);
                let parent = parent.unwrap_or_else(|| panic!("{} has no parent", def.name));
                assert!(
                    matches!(parent.kind, ItemKind::Block | ItemKind::Background),
                    "{}",
                    def.name
                );
                assert!(!parent.has(NO_SEED), "{}", def.name);
                assert_eq!(def.rarity, parent.rarity, "{}", def.name);
            }
        }
    }

    #[test]
    fn every_sprite_file_exists() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/blocks");
        let files: HashSet<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        // Every row has a sprite: the enum has no "none" variant.
        for def in ITEMS {
            match def.sprite {
                Sprite::Terrain(_) => {}
                Sprite::Block(file) | Sprite::Tinted(file, _) => {
                    assert!(files.contains(file), "{}: {file}", def.name);
                }
                Sprite::Equip(folder) => {
                    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/items");
                    let path = std::path::Path::new(dir).join(folder);
                    assert!(path.is_dir(), "{}: {folder}", def.name);
                }
            }
        }
    }

    #[test]
    fn every_equipment_entry_has_an_icon() {
        // The client draws `Sprite::Equip` as this file and composites nothing at runtime.
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/items");
        for name in EQUIPMENT {
            let icon = std::path::Path::new(dir).join(name).join("icon.png");
            assert!(icon.is_file(), "{}", icon.display());
        }
    }

    #[test]
    fn every_item_folder_has_one_equipment_entry_and_back() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/items");
        let folders: HashSet<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().unwrap().is_dir())
            .map(|entry| entry.file_name().into_string().unwrap())
            .collect();
        let entries: HashSet<String> = EQUIPMENT.iter().map(|name| name.to_string()).collect();
        assert_eq!(entries.len(), EQUIPMENT.len(), "duplicate EQUIPMENT entry");
        assert_eq!(
            folders.difference(&entries).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "folders without an entry"
        );
        assert_eq!(
            entries.difference(&folders).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "entries without a folder"
        );
    }

    #[test]
    fn equipment_rows_follow_the_table_rules() {
        for (index, &name) in EQUIPMENT.iter().enumerate() {
            let id = ItemId(EQUIP_ID_BASE + index as u16);
            let def = item(id).unwrap_or_else(|| panic!("{name} has no row"));
            let slot = slot_of_name(name).unwrap_or_else(|| panic!("{name} has no slot"));
            assert_eq!(def.name, name);
            assert_eq!(def.kind, ItemKind::Equip(slot), "{name}");
            assert_eq!(slot_of(id), Some(slot), "{name}");
            assert_eq!(def.sprite, Sprite::Equip(name), "{name}");
            assert_eq!(
                (def.rarity, def.hits, def.collision),
                (0, 0, Collision::None)
            );
            assert!(
                def.has(NO_SEED) && !def.has(UNTRADEABLE | PERMANENT),
                "{name}"
            );
            assert_eq!(seed_of(id), None, "{name}");
            assert!(equip_price(id).is_some_and(|price| price > 0), "{name}");
        }
        // Equipment is the only kind with a slot, and nothing else has an equipment price.
        for def in ITEMS {
            let is_equip = matches!(def.kind, ItemKind::Equip(_));
            assert_eq!(slot_of(def.id).is_some(), is_equip, "{}", def.name);
            assert_eq!(equip_price(def.id).is_some(), is_equip, "{}", def.name);
        }
        assert_eq!(
            slot_of(ItemId(EQUIP_ID_BASE + EQUIPMENT.len() as u16)),
            None
        );
        assert_eq!(slot_of_name("cape_red"), None);
        assert_eq!(
            Slot::ALL.map(|slot| slot as usize),
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
    }

    #[test]
    fn the_starter_outfit_is_the_first_four_entries() {
        let names = STARTER_OUTFIT.map(|id| item(id).map(|def| def.name));
        assert_eq!(
            names,
            [
                Some("hair_brown"),
                Some("shirt_red"),
                Some("pants_jeans_blue"),
                Some("shoes_boots_brown")
            ]
        );
        let slots = STARTER_OUTFIT.map(slot_of);
        assert_eq!(
            slots,
            [Slot::Hair, Slot::Shirt, Slot::Pants, Slot::Shoes].map(Some)
        );
    }

    #[test]
    fn prices_and_pickaxe_power_follow_the_tables() {
        let named = |name: &str| {
            let index = EQUIPMENT.iter().position(|&n| n == name).unwrap();
            ItemId(EQUIP_ID_BASE + index as u16)
        };
        for (name, price) in [
            ("hair_brown", 150),
            ("shirt_red", 200),
            ("pants_black", 200),
            ("shoes_sneakers_red", 200),
            ("hat_straw", 300),
            ("face_goggles", 300),
            ("wings_bone", 10_000),
            ("pickaxe_wood", 200),
            ("pickaxe_iron", 1_000),
            ("pickaxe_gold", 4_000),
            ("pickaxe_diamond", 15_000),
        ] {
            assert_eq!(equip_price(named(name)), Some(price), "{name}");
        }
        assert_eq!(punch_power(None), FIST_POWER);
        assert_eq!(punch_power(Some(named("pickaxe_wood"))), 1);
        assert_eq!(punch_power(Some(named("pickaxe_royal"))), 4);
        // Only a pickaxe has power; anything else in the hand is the fist.
        assert_eq!(punch_power(Some(named("hair_brown"))), FIST_POWER);
        assert_eq!(punch_power(Some(ItemId(u16::MAX))), FIST_POWER);
        assert_eq!(equip_price(DIRT), None);

        // Every pickaxe is in exactly one tier and every tier entry is a pickaxe.
        let pickaxes: Vec<&str> = EQUIPMENT
            .iter()
            .copied()
            .filter(|name| slot_of_name(name) == Some(Slot::Hand))
            .collect();
        assert_eq!(pickaxes.len(), 20);
        for name in &pickaxes {
            let tiers = PICKAXE_TIERS
                .iter()
                .filter(|tier| tier.pickaxes.contains(name))
                .count();
            assert_eq!(tiers, 1, "{name}");
        }
        let listed: usize = PICKAXE_TIERS.iter().map(|tier| tier.pickaxes.len()).sum();
        assert_eq!(listed, pickaxes.len());
    }

    #[test]
    fn seeds_and_claims_are_tinted_with_distinct_colours() {
        for (kind, file) in [
            (ItemKind::Seed, SEED_SPRITE),
            (ItemKind::Claim, CLAIM_SPRITE),
        ] {
            let mut colours = HashSet::new();
            for def in ITEMS.iter().filter(|def| def.kind == kind) {
                let Sprite::Tinted(sprite, rgb) = def.sprite else {
                    panic!("{} is not tinted", def.name);
                };
                assert_eq!(sprite, file, "{}", def.name);
                assert!(colours.insert(rgb), "{} shares its colour", def.name);
            }
            assert!(!colours.is_empty());
        }
    }

    #[test]
    fn claims_are_solid_unseeded_and_breakable() {
        for id in [SMALL_CLAIM, MEDIUM_CLAIM, LARGE_CLAIM, WORLD_CLAIM] {
            let def = item(id).unwrap();
            assert_eq!(
                (def.kind, def.collision, def.rarity),
                (ItemKind::Claim, Collision::Solid, 0)
            );
            assert!(def.has(NO_SEED) && !def.has(UNBREAKABLE) && def.hits > 0);
            assert_eq!(seed_of(id), None);
        }
    }
}
