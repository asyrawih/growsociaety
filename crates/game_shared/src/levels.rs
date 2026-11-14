//! Levels and XP (docs/knowledge/14). XP is credited through `rules::apply`; nothing is gated by
//! level yet.

/// Reference: the genre's cap. At the cap XP stops accumulating.
pub const MAX_LEVEL: u16 = 125;
/// `xp_to_next(level) = XP_CURVE_BASE × level`. Tuning.
pub const XP_CURVE_BASE: u32 = 100;
/// Harvesting a grown tree gives this much XP per point of the fruit's rarity. Tuning.
pub const XP_HARVEST_PER_RARITY: u32 = 2;
/// Planting a seed (not splicing). Tuning.
pub const XP_PLANT: u32 = 1;
/// Splicing two seeds. Tuning.
pub const XP_SPLICE: u32 = 2;
/// Breaking a block or background gives `max(XP_BREAK_MIN, rarity)`. Tuning.
pub const XP_BREAK_MIN: u32 = 1;

/// XP needed to go from `level` to the next one; 0 at `MAX_LEVEL`.
pub fn xp_to_next(level: u16) -> u32 {
    if level >= MAX_LEVEL {
        0
    } else {
        XP_CURVE_BASE.saturating_mul(u32::from(level))
    }
}

/// Adds `gained` XP, carrying over across as many levels as it covers, and returns the levels
/// gained. Saturating: at `MAX_LEVEL` the progress stays 0.
pub fn add_xp(level: &mut u16, xp: &mut u32, gained: u32) -> u16 {
    let start = *level;
    let mut left = gained;
    while *level < MAX_LEVEL {
        let need = xp_to_next(*level).saturating_sub(*xp);
        if left < need {
            *xp += left;
            break;
        }
        left -= need;
        *level += 1;
        *xp = 0;
    }
    if *level >= MAX_LEVEL {
        *xp = 0;
    }
    level.saturating_sub(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Total XP from level 1 to the start of `level`.
    fn total_to_reach(level: u16) -> u32 {
        (1..level).map(xp_to_next).sum()
    }

    #[test]
    fn xp_to_next_matches_the_table() {
        for (level, next, total) in [
            (1, 100, 0),
            (2, 200, 100),
            (5, 500, 1_000),
            (10, 1_000, 4_500),
            (25, 2_500, 30_000),
            (50, 5_000, 122_500),
            (125, 0, 775_000),
        ] {
            assert_eq!(xp_to_next(level), next, "level {level}");
            assert_eq!(total_to_reach(level), total, "level {level}");
            let l = u32::from(level);
            assert_eq!(total, 50 * l * (l - 1), "closed form at {level}");
        }
        assert_eq!(xp_to_next(u16::MAX), 0);
    }

    #[test]
    fn add_xp_carries_over_stops_at_the_cap_and_never_overflows() {
        let (mut level, mut xp) = (1, 0);
        assert_eq!(add_xp(&mut level, &mut xp, 99), 0);
        assert_eq!((level, xp), (1, 99));
        // 1 finishes level 1, 200 finishes level 2, 50 is carried into level 3.
        assert_eq!(add_xp(&mut level, &mut xp, 1 + 200 + 50), 2);
        assert_eq!((level, xp), (3, 50));
        assert_eq!(add_xp(&mut level, &mut xp, 0), 0);
        assert_eq!((level, xp), (3, 50));

        let (mut level, mut xp) = (1, 0);
        assert_eq!(add_xp(&mut level, &mut xp, total_to_reach(MAX_LEVEL)), 124);
        assert_eq!((level, xp), (MAX_LEVEL, 0));
        assert_eq!(add_xp(&mut level, &mut xp, u32::MAX), 0);
        assert_eq!((level, xp), (MAX_LEVEL, 0));

        let (mut level, mut xp) = (1, 0);
        assert_eq!(add_xp(&mut level, &mut xp, u32::MAX), 124);
        assert_eq!((level, xp), (MAX_LEVEL, 0));
    }
}
