use rand_pcg::Pcg32;
use rand_pcg::rand_core::{Rng as _, SeedableRng as _};
use rand_xoshiro::SplitMix64;

use crate::scalar::{RATE_UNIT, Rate};

fn mix(seed: u64) -> u64 {
    SplitMix64::seed_from_u64(seed).next_u64()
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rng(Pcg32);

impl Rng {
    pub const SKILLS: u64 = 0;
    /// TODO - not actually drawing from this yet
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