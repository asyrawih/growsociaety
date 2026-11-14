//! Deterministic PRNG (SplitMix64). World generation depends on this exact stream: never change it.

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`. Always consumes exactly one value.
    pub fn next_f32(&mut self) -> f32 {
        // Top 24 bits: every value is exactly representable in an f32, and the division is exact.
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// True with probability `p`. Always consumes exactly one value.
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }

    /// Uniform in `[lo, hi_inclusive]`. Returns `lo` if the range is empty or inverted.
    pub fn range(&mut self, lo: u32, hi_inclusive: u32) -> u32 {
        let span = u64::from(hi_inclusive.saturating_sub(lo)) + 1;
        // ponytail: modulo bias (< 2^-32 for u32 spans); use rejection sampling if exact uniformity ever matters.
        lo + (self.next_u64() % span) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_splitmix64_reference_vector() {
        let mut rng = Rng::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn chance_and_range_stay_in_bounds() {
        let mut rng = Rng::new(7);
        for _ in 0..1000 {
            assert!(!rng.chance(0.0));
            assert!(rng.chance(1.0));
            assert!((3..=9).contains(&rng.range(3, 9)));
        }
        assert_eq!(rng.range(5, 5), 5);
        assert_eq!(rng.range(5, 2), 5);
        rng.range(0, u32::MAX); // full span must not overflow
    }
}
