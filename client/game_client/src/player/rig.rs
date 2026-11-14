//! Data and pure math for the code-animated player rig (assets/README.md, "Player rig").
//! Mirrors assets/player_rig/rig.json; a test checks the two agree.

use std::f32::consts::{PI, TAU};

/// The layers are 48x48 canvases facing east; every part and item lines up with no offset.
pub const CANVAS: f32 = 48.0;
/// First empty row under the feet, canvas y (down).
pub const FEET_Y: f32 = 47.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    ArmBack,
    LegBack,
    LegFront,
    Torso,
    Head,
    ArmFront,
}

impl Part {
    /// Back to front, rig.json "order".
    pub const ALL: [Part; 6] = [
        Part::ArmBack,
        Part::LegBack,
        Part::LegFront,
        Part::Torso,
        Part::Head,
        Part::ArmFront,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Part::ArmBack => "arm_back",
            Part::LegBack => "leg_back",
            Part::LegFront => "leg_front",
            Part::Torso => "torso",
            Part::Head => "head",
            Part::ArmFront => "arm_front",
        }
    }

    /// Canvas pixel the part rotates around (y down), rig.json "pivots".
    pub fn pivot(self) -> [u32; 2] {
        match self {
            Part::ArmBack => [23, 25],
            Part::LegBack => [28, 41],
            Part::LegFront => [24, 41],
            Part::Torso => [27, 38],
            Part::Head => [24, 20],
            Part::ArmFront => [21, 25],
        }
    }

    /// The pivot as a sprite anchor (-0.5..0.5, y up).
    pub fn anchor(self) -> [f32; 2] {
        let [x, y] = self.pivot().map(|v| v as f32 / CANVAS);
        [x - 0.5, 0.5 - y]
    }
}

/// rig.json "skin_ramps", light to dark. The body PNGs use exactly `SKIN_RAMPS[0]`.
pub const SKIN_RAMPS: [(&str, [u32; 5]); 5] = [
    ("light", [0xf8dcb3, 0xf5c596, 0xe1936f, 0xc37a60, 0x8e5143]),
    ("tan", [0xf0c08e, 0xd99a68, 0xb8704c, 0x965740, 0x6a3a2c]),
    ("brown", [0xc98e62, 0xa96d47, 0x8a5236, 0x6c3e2a, 0x4a291d]),
    ("dark", [0x9a6a4b, 0x7c5038, 0x5f3a28, 0x47291e, 0x2f1b14]),
    ("pale", [0xfdeee0, 0xf7dcc8, 0xe8b9a0, 0xcf9a84, 0x9c6b5a]),
];

fn rgb(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

/// Swaps the light ramp for ramp `skin` in RGBA8 pixel bytes. Every other pixel is left alone.
pub fn recolor_skin(rgba: &mut [u8], skin: usize) {
    let (from, to) = (SKIN_RAMPS[0].1.map(rgb), SKIN_RAMPS[skin].1.map(rgb));
    for pixel in rgba.chunks_exact_mut(4) {
        if let Some(i) = from.iter().position(|c| pixel[..3] == c[..]) {
            pixel[..3].copy_from_slice(&to[i]);
        }
    }
}

use game_shared::items::SLOT_PREFIXES;
pub use game_shared::items::Slot;

/// The slot an item folder belongs to, from its name prefix (docs/knowledge/03, "Equipment").
pub fn slot_of(item: &str) -> Option<Slot> {
    SLOT_PREFIXES
        .iter()
        .find(|(prefix, _)| item.starts_with(prefix))
        .map(|&(_, slot)| slot)
}

/// Clothing layers in the order they stack inside one body part (body is under all of them).
pub const STACK: [Slot; 6] = [
    Slot::Pants,
    Slot::Shoes,
    Slot::Shirt,
    Slot::Hair,
    Slot::Face,
    Slot::Hat,
];

/// Stacking rank of a clothing item inside a body part. `None` for wings, pickaxes and non-items.
pub fn layer(item: &str) -> Option<usize> {
    let slot = slot_of(item)?;
    STACK.iter().position(|&s| s == slot)
}

/// Wings: one canvas-sized layer behind the whole body, rotating around the torso pivot.
pub const WINGS_FILE: &str = "back.png";
/// Held item: its icon, with this pixel (handle end, y down, in the 32x32 icon) in the hand.
pub const HELD_FILE: &str = "icon.png";
pub const HELD_SIZE: f32 = 32.0;
pub const HELD_GRIP: [f32; 2] = [3.0, 29.0];
/// Turn of the held item around its grip, radians (CCW). The icon points its head up-right; turned
/// down this far it rests head-forward and rises to point up-forward at the top of a punch.
pub const HELD_ANGLE: f32 = -60.0 * PI / 180.0;
/// Where the hand is in the rig canvas (y down). The held item is a child of `arm_front`.
pub const HAND: [f32; 2] = [22.0, 36.0];

/// The grip as a sprite anchor (-0.5..0.5, y up).
pub fn held_anchor() -> [f32; 2] {
    let [x, y] = HELD_GRIP.map(|v| v / HELD_SIZE);
    [x - 0.5, 0.5 - y]
}

/// The hand relative to the `arm_front` pivot, world axes (y up).
pub fn hand_offset() -> [f32; 2] {
    let [px, py] = ArmFront.pivot().map(|v| v as f32);
    [HAND[0] - px, py - HAND[1]]
}

/// Wing flap: amplitude (radians) and full flaps per second.
const FLAP: f32 = 4.0 * PI / 180.0;
const FLAP_HZ: f32 = 1.2;

/// Wing rotation at `seconds`; faster and wider in the air.
pub fn flap(motion: Motion, seconds: f32) -> f32 {
    let (amp, hz) = match motion {
        Motion::Air => (FLAP * 2.0, FLAP_HZ * 3.0),
        Motion::Idle | Motion::Walk => (FLAP, FLAP_HZ),
    };
    amp * (seconds * hz * TAU).sin()
}

/// The body parts `item` has a layer for: the `<part>.png` files in `assets/items/<item>/`.
/// Read from disk, so a new item needs no code. `assets` is the asset root.
pub fn item_parts(assets: &std::path::Path, item: &str) -> Vec<Part> {
    let dir = assets.join("items").join(item);
    Part::ALL
        .into_iter()
        .filter(|part| dir.join(format!("{}.png", part.name())).is_file())
        .collect()
}

use Part::*;
/// Leg swing amplitude, radians.
const STRIDE: f32 = 14.0 * PI / 180.0;
/// Peak of the punch arc, radians (counter-clockwise = forward when facing east).
const PUNCH_REACH: f32 = 95.0 * PI / 180.0;
/// Full walk cycles per second.
const WALK_HZ: f32 = 1.7;

/// How the body moves this frame. `swing` is punch progress in `0..1`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Motion {
    Idle,
    Walk,
    Air,
}

/// Rotation (radians, CCW) and vertical shift (px, world up) of each part, indexed like `Part::ALL`.
pub fn pose(motion: Motion, seconds: f32, swing: Option<f32>) -> [(f32, f32); 6] {
    let (leg, bob) = match motion {
        Motion::Idle => (0.0, 0.0),
        Motion::Walk => {
            let phase = (seconds * WALK_HZ * TAU).sin();
            // Lowest when the legs are spread.
            (STRIDE * phase, if phase.abs() > 0.5 { -1.0 } else { 0.0 })
        }
        // A frozen mid-stride while airborne.
        Motion::Air => (STRIDE, 0.0),
    };
    let punch = swing.map(|progress| PUNCH_REACH * (progress.clamp(0.0, 1.0) * PI).sin());
    Part::ALL.map(|part| match part {
        LegFront => (leg, 0.0),
        LegBack => (-leg, 0.0),
        ArmFront => (punch.unwrap_or(-leg), 0.0),
        ArmBack => (leg, 0.0),
        Torso | Head => (0.0, bob),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    #[test]
    fn matches_rig_json() {
        let json: String = std::fs::read_to_string(assets().join("player_rig/rig.json"))
            .unwrap()
            .split_whitespace()
            .collect();
        let order: Vec<String> = Part::ALL
            .iter()
            .map(|p| format!("\"{}\"", p.name()))
            .collect();
        assert!(json.contains(&format!("\"order\":[{}]", order.join(","))));
        for part in Part::ALL {
            let [x, y] = part.pivot();
            assert!(
                json.contains(&format!("\"{}\":[{x},{y}]", part.name())),
                "{part:?}"
            );
        }
        for (name, ramp) in SKIN_RAMPS {
            let hex: Vec<String> = ramp.iter().map(|c| format!("\"#{c:06x}\"")).collect();
            assert!(
                json.contains(&format!("\"{name}\":[{}]", hex.join(","))),
                "{name}"
            );
        }
    }

    #[test]
    fn every_item_with_body_layers_has_a_slot() {
        let mut checked = 0;
        for entry in std::fs::read_dir(assets().join("items")).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let parts = item_parts(&assets(), &name);
            if !parts.is_empty() {
                assert!(layer(&name).is_some(), "{name} has no slot");
                checked += 1;
            }
        }
        assert!(checked >= 24, "only {checked} items found");
        assert_eq!(
            item_parts(&assets(), "shirt_red"),
            [ArmBack, Torso, ArmFront]
        );
        assert!(item_parts(&assets(), "no_such_item").is_empty());
    }

    #[test]
    fn slots_come_from_the_folder_prefix() {
        assert_eq!(slot_of("wings_dragon_blue"), Some(Slot::Back));
        assert_eq!(slot_of("pickaxe_fire"), Some(Slot::Hand));
        assert_eq!(slot_of("hair_afro_black"), Some(Slot::Hair));
        assert_eq!(slot_of("pixellab_ids.txt"), None);
        assert_eq!(layer("wings_dragon_blue"), None);
        assert_eq!(layer("pickaxe_fire"), None);
        for (i, slot) in Slot::ALL.into_iter().enumerate() {
            assert_eq!(slot as usize, i);
        }
        // Every item folder is in some slot and has the file its slot draws.
        for entry in std::fs::read_dir(assets().join("items")).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            if !entry.path().is_dir() {
                continue;
            }
            let slot = slot_of(&name).unwrap_or_else(|| panic!("{name} has no slot"));
            let file = match slot {
                Slot::Back => entry.path().join(WINGS_FILE),
                Slot::Hand => entry.path().join(HELD_FILE),
                _ => continue,
            };
            assert!(file.is_file(), "{}", file.display());
        }
    }

    #[test]
    fn the_grip_lands_on_the_hand() {
        // (3, 29) of 32: left and near the bottom of the icon.
        let [ax, ay] = held_anchor();
        assert!((ax + 0.40625).abs() < 1e-6 && (ay + 0.40625).abs() < 1e-6);
        // The hand is 1 px right of and 11 px below the arm pivot (21, 25).
        assert_eq!(hand_offset(), [1.0, -11.0]);
    }

    #[test]
    fn hat_stacks_over_hair_over_shirt() {
        assert!(layer("hat_straw") > layer("hair_brown"));
        assert!(layer("hair_brown") > layer("shirt_red"));
        assert!(layer("shirt_red") > layer("pants_black"));
        assert_eq!(layer("sword"), None);
    }

    #[test]
    fn recolor_swaps_only_skin() {
        let mut px = [0xf8, 0xdc, 0xb3, 255, 1, 2, 3, 255, 0x8e, 0x51, 0x43, 128];
        recolor_skin(&mut px, 3);
        assert_eq!(
            px,
            [0x9a, 0x6a, 0x4b, 255, 1, 2, 3, 255, 0x2f, 0x1b, 0x14, 128]
        );
    }

    #[test]
    fn walk_and_punch_poses() {
        let idx = |p: Part| Part::ALL.iter().position(|&q| q == p).unwrap();
        assert!(
            pose(Motion::Idle, 0.3, None)
                .iter()
                .all(|&(a, y)| a == 0.0 && y == 0.0)
        );

        let quarter = 0.25 / WALK_HZ; // sin peak
        let walk = pose(Motion::Walk, quarter, None);
        assert!((walk[idx(LegFront)].0 - STRIDE).abs() < 1e-5);
        assert_eq!(walk[idx(LegBack)].0, -walk[idx(LegFront)].0);
        assert_eq!(walk[idx(ArmFront)].0, -walk[idx(LegFront)].0);
        assert_eq!(walk[idx(Torso)].1, -1.0);
        assert_eq!(walk[idx(Head)].1, -1.0);

        let punch = |p| pose(Motion::Walk, quarter, Some(p))[idx(ArmFront)].0;
        assert!(punch(0.0).abs() < 1e-5);
        assert!((punch(0.5) - PUNCH_REACH).abs() < 1e-5);
        assert!(punch(1.0).abs() < 1e-5);
    }
}
