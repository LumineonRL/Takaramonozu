use rand_pcg::Pcg32;
use rand_pcg::rand_core::{Rng as _, SeedableRng as _};
use rand_xoshiro::SplitMix64;

use crate::scalar::{RATE_UNIT, Rate};

fn mix(seed: u64) -> u64 {
    SplitMix64::seed_from_u64(seed).next_u64()
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
        Self(Pcg32::new(mix(seed), stream))
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
