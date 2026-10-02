use rand_pcg::Pcg32;
use rand_pcg::rand_core::{Rng as _, SeedableRng as _};
use rand_xoshiro::SplitMix64;

use crate::scalar::{RATE_UNIT, Rate};

/// Turns out that using the same seed to make two different streams was leading to
/// correlated results between accuracy and skill activations. Runs with good acc also
/// had a lot of skill activations and vice versa. This should fix that.
fn mix(seed: u64, stream: u64) -> u64 {
    let mut outputs = SplitMix64::seed_from_u64(seed);
    for _ in 0..stream {
        outputs.next_u64();
    }
    outputs.next_u64()
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rng(Pcg32);

/// Gave skills and acc different RNG streams. Shouldn't affect scoring or variance or
/// anything like that, but it matters when one stream or the other just doesn't need
/// to be constructed. For example, if accuracy is set to 100%, there's no need to
/// build the acc RNG in the first place.
/// Acc is also nice because a deterministic amount is needed per trial.
/// Skills are only sometimes deterministic Throw in a score-based scorer or a
/// star note scorer and the numbers of times they'll need to roll will change from
/// trial to trial.
impl Rng {
    pub const SKILLS: u64 = 0;
    pub const ACCURACY: u64 = 1;

    #[must_use]
    pub fn new(seed: u64, stream: u64) -> Self {
        Self(Pcg32::new(mix(seed, stream), stream))
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.0.next_u32()
    }

    #[inline]
    #[must_use]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "I think casting here should be fine but I'll put a TODO."
    )]
    pub fn below(&mut self, n: u32) -> u32 {
        ((u64::from(self.next_u32()) * u64::from(n)) >> 32) as u32
    }

    #[inline]
    #[must_use]
    pub fn roll(&mut self, rate: Rate) -> bool {
        self.below(u32::from(RATE_UNIT)) < u32::from(rate)
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn check_the_draws_are_pinned() {
        let draws = |seed: u64, stream: u64| {
            let mut rng = Rng::new(seed, stream);
            [rng.next_u32(), rng.next_u32(), rng.next_u32(), rng.next_u32()]
        };
        assert_eq!(
            draws(0, Rng::SKILLS),
            [3_234_325_189, 1_963_755_818, 1_465_678_534, 3_792_411_884]
        );
        assert_eq!(
            draws(1, Rng::SKILLS),
            [2_157_191_000, 1_538_268_690, 1_916_251_149, 3_194_205_086]
        );
        assert_eq!(
            draws(0xDEAD_BEEF, Rng::SKILLS),
            [529_590_346, 3_193_142_877, 119_973_751, 2_244_065_526],
        );
        assert_eq!(
            draws(0, Rng::ACCURACY),
            [1_623_444_312, 4_262_029_841, 579_704_389, 3_865_678_028]
        );
        assert_eq!(
            draws(1, Rng::ACCURACY),
            [2_785_056_654, 111_877_511, 3_147_239_216, 894_744_346]
        );
        assert_eq!(
            draws(0xDEAD_BEEF, Rng::ACCURACY),
            [1_667_202_528, 285_341_127, 309_380_848, 3_035_236_912],
        );

        // The first skill draw at seed 0 is the first row of the draw table in
        // `docs/skill_fixtures.md`.
        assert_eq!(Rng::new(0, Rng::SKILLS).below(10_000), 7530);
    }

    #[expect(clippy::arithmetic_side_effects, reason = "u32 to i128 can't overflow")]
    fn centred(x: u32) -> i128 {
        i128::from(x) - (1_i128 << 31)
    }

    #[test]
    #[expect(clippy::integer_division, reason = "need to report the number")]
    fn check_the_streams_are_uncorrelated() {
        const SEEDS: u64 = 4096;
        const INDICES: usize = 1200;

        let mut sums = vec![0_i128; INDICES];
        for seed in 0..SEEDS {
            let mut skills = Rng::new(seed, Rng::SKILLS);
            let mut accuracy = Rng::new(seed, Rng::ACCURACY);
            for sum in &mut sums {
                *sum += centred(skills.next_u32()) * centred(accuracy.next_u32());
            }
        }

        let se = (64_u128 << 64) / 12;
        let (index, worst) = sums
            .iter()
            .map(|s| s.unsigned_abs())
            .enumerate()
            .max_by_key(|&(_, s)| s)
            .expect("indices");
        let tenths = worst * 10 / se;
        assert!(
            worst < 6 * se,
            "index {index} is {}.{} SE off independent",
            tenths / 10,
            tenths % 10
        );
    }
}
