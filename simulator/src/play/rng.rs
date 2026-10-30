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
/// Sync skill also needed its own RNG to pick its target
/// and Amplify/Encore get one for the coinflip mechanic.
/// the coin flip one is weird because it's literally just one single 50/50 at the start of a trial.
/// and really it SHOULDN'T impact anything if it was lumped into the skills stream, right? Like,
/// just treat the coin flip as a 50/50 that happens at the start of a trial where an encore or amp is present.
/// TODO to maybe change.
///
/// I guess I need to explain what "coin flip means" for those out of the loop and why it's here.
/// there's a mechanic where the order in which skills resolve when an Encore or Amplify is on the team
/// is random from trial to trial.
/// If multiple skills are set to trigger at the same moment (almost always from a "rhythm icon" condition), for
/// ALMOST EVERY skill, they will resolve from the right-most lane to the leftmost lane regardless of what the skill effect is.
/// Except for Encore and Amplify.
///
/// The possible orderings are [Amp, Other, Encore] or [Other, Amp, Encore] where other is literally any
/// other skill, And again, this only matters if multiple skills are trying to proc on the same exact instant.
impl Rng {
    pub const SKILLS: u64 = 0;
    pub const ACCURACY: u64 = 1;
    pub const SYNC: u64 = 2;
    pub const COIN: u64 = 3;

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

    /// As a performance improvement, Judgement RNG is now rolled in batches rather than sequentially.
    /// tldr rolling a batch at a time instead of one at a time leads to more vectorization
    /// Note that it's ONLY for Judgements - the other 3 RNG streams can't (or at least I can't figure out how)
    /// to have their rolls pre-computed up front like this.
    ///
    /// Much of this is a vectorized version of the existing `pcg32` from the `rand_pcg` crate.
    /// See <https://github.com/rust-random/rngs/blob/master/rand_pcg/src/pcg64.rs> for more details.
    /// Alsoalso yes, the linked file is pcg64, but pcg32 lives in there and the code here is pcg32.
    #[expect(clippy::indexing_slicing, reason = "split into batches so it vectorizes.")]
    pub fn roll_into(&mut self, rate: Rate, slots: &mut [bool]) -> usize {
        let stream = self.0.stream();
        if rate >= RATE_UNIT {
            slots.fill(true);
            self.0.advance(u64::try_from(slots.len()).unwrap_or(u64::MAX));
            return slots.len();
        }
        let increment = stream.wrapping_shl(1) | 1;
        let first = self.0.state();
        let mut states = LANES
            .map(|(mult, sum)| first.wrapping_mul(mult).wrapping_add(increment.wrapping_mul(sum)));
        let (mult, sum) = JUMP;
        let plus = increment.wrapping_mul(sum);
        let cut = cut(rate);

        let mut hits = 0_usize;
        let (chunks, rest) = slots.as_chunks_mut::<BATCH>();
        for chunk in chunks {
            *chunk = states.map(|state| output(state) < cut);
            hits = hits.wrapping_add(chunk.iter().map(|&hit| usize::from(hit)).sum::<usize>());
            states = states.map(|state| state.wrapping_mul(mult).wrapping_add(plus));
        }
        for (slot, &state) in rest.iter_mut().zip(&states) {
            *slot = output(state) < cut;
            hits = hits.wrapping_add(usize::from(*slot));
        }
        self.0 = Pcg32::from_state(states[rest.len()], stream);
        hits
    }
}

/// Checks if a roll succeeds or not.
/// Since this is only being used for judgements, basically accuracy gets passed into `roll_into`,
/// which gets passed into `cut`, which determines as a batch which rolls are a perfect and a great.
/// Left open ended though because idk maybe with simple skills like scorers only this could be done
/// too but I don't know if it's worth the effort.
fn cut(rate: Rate) -> u32 {
    let cut = u64::from(rate).wrapping_shl(32).div_ceil(u64::from(RATE_UNIT));
    u32::try_from(cut).unwrap_or(u32::MAX)
}

/// Tried a bunch of different values and 16 measured fastest on AVX-512.
/// And AVX2. And SSE4.1.
const BATCH: usize = 16;

/// Copied directly from:
/// <https://github.com/rust-random/rngs/blob/84683fcacf9a5e346a945d1f620c772f91b4b82d/rand_pcg/src/pcg64.rs#L19>
const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

const fn step((mult, sum): (u64, u64)) -> (u64, u64) {
    (mult.wrapping_mul(MULTIPLIER), sum.wrapping_mul(MULTIPLIER).wrapping_add(1))
}

/// The different lanes. A given lane starts `i` draws ahead.
/// Also because this can be confusing, LANES is a SIMD term and a game mechanic.
/// The lanes in this module are completely separate from the lanes in every other module.
/// Keeping it because they're both the correct term.
#[expect(clippy::indexing_slicing, reason = "bounded")]
const LANES: [(u64, u64); BATCH] = {
    let mut lanes = [(1, 0); BATCH];
    let mut lane = 1;
    while lane < BATCH {
        lanes[lane] = step(lanes[lane - 1]);
        lane += 1;
    }
    lanes
};

const JUMP: (u64, u64) = step(LANES[BATCH - 1]);

/// See <https://github.com/rust-random/rngs/blob/84683fcacf9a5e346a945d1f620c772f91b4b82d/rand_pcg/src/pcg64.rs#L149>
#[inline(always)]
#[expect(clippy::cast_possible_truncation, reason = "XSH RR keeps the low 32 bits")]
const fn output(state: u64) -> u32 {
    let rot = (state >> 59) as u32;
    let xsh = (((state >> 18) ^ state) >> 27) as u32;
    xsh.rotate_right(rot)
}

#[cfg(test)]
mod tests {
    use super::{Rng, cut};
    use crate::scalar::RATE_UNIT;

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

    /// Checks that the vectorized lane RNG comes out the same as the regular RNG.
    /// Tests it on all RNG streams. Just in case.
    /// `u64::MAX` is to test a stream with its top bit set.
    #[test]
    fn check_roll_into() {
        for stream in [Rng::SKILLS, Rng::ACCURACY, Rng::SYNC, Rng::COIN, u64::MAX] {
            for seed in [0, 1, 2, 0x5C00_1D01, u64::MAX] {
                for len in [0, 1, 2, 15, 16, 17, 31, 32, 33, 100, 1131] {
                    for rate in [0, 1, 3, 4999, 9250, RATE_UNIT - 1, RATE_UNIT, 20_000] {
                        let mut one = Rng::new(seed, stream);
                        let want: Vec<bool> =
                            core::iter::repeat_with(|| one.roll(rate)).take(len).collect();
                        let mut batch = Rng::new(seed, stream);
                        let mut got = vec![false; len];
                        let hits = batch.roll_into(rate, &mut got);
                        let case = format!("stream {stream} seed {seed} len {len} rate {rate}");
                        assert_eq!(got, want, "{case}");
                        assert_eq!(hits, want.iter().filter(|&&hit| hit).count(), "{case}");
                        assert_eq!(batch.next_u32(), one.next_u32(), "{case}: left elsewhere");
                    }
                }
            }
        }
    }

    /// `cut` is the vectorized version of `below`. Ensures that they produce the exact same numbers.
    #[test]
    fn check_cut_matches_below() {
        let hits = |draw: u32, rate| (u64::from(draw) * u64::from(RATE_UNIT)) >> 32 < rate;
        assert_eq!(cut(0), 0);
        for rate in 1..RATE_UNIT {
            let cut = cut(rate);
            assert!(hits(cut - 1, u64::from(rate)), "rate {rate}: {} misses", cut - 1);
            assert!(!hits(cut, u64::from(rate)), "rate {rate}: {cut} hits");
        }
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

        for (a, b) in [
            (Rng::SKILLS, Rng::ACCURACY),
            (Rng::SKILLS, Rng::SYNC),
            (Rng::ACCURACY, Rng::SYNC),
            (Rng::SKILLS, Rng::COIN),
            (Rng::ACCURACY, Rng::COIN),
            (Rng::SYNC, Rng::COIN),
        ] {
            let mut sums = vec![0_i128; INDICES];
            for seed in 0..SEEDS {
                let (mut x, mut y) = (Rng::new(seed, a), Rng::new(seed, b));
                for sum in &mut sums {
                    *sum += centred(x.next_u32()) * centred(y.next_u32());
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
                "streams {a} and {b}: index {index} is {}.{} SE off independent",
                tenths / 10,
                tenths % 10
            );
        }
    }
}
