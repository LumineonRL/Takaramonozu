//! Accuracy. All judgements are pre-rolled before a trial starts.

use super::config::PlayConfig;
use super::queue::Queue;
use super::rng::Rng;
use crate::scalar::{RATE_UNIT, Rate};

/// Precompute acc rolls before a trial starts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Judgements {
    perfect: Vec<bool>,
    seed: u64,
    accuracy: Rate,
    perfects: u32,
}

impl Judgements {
    #[must_use]
    pub fn build(queue: &Queue, config: &PlayConfig) -> Self {
        let records = queue.events().len();
        let rolls = records.saturating_sub(1);

        let (perfect, perfects) = match config.accuracy {
            0 => (vec![false; records], 0),
            rate if rate >= RATE_UNIT => (vec![true; records], rolls),
            rate => {
                let mut rng = Rng::new(config.seed, Rng::ACCURACY);
                let mut perfect = vec![false; records];
                let mut hits = 0_usize;
                for slot in perfect.iter_mut().take(rolls) {
                    *slot = rng.roll(rate);
                    hits = hits.saturating_add(usize::from(*slot));
                }
                (perfect, hits)
            }
        };

        Self {
            perfect,
            seed: config.seed,
            accuracy: config.accuracy,
            perfects: u32::try_from(perfects).unwrap_or(u32::MAX),
        }
    }

    /// Judgements as they were played.
    /// For the sync skill. Well had been considered for a while and encore and amp
    /// will end up using it too, but sync was the reason.
    #[must_use]
    pub fn recorded(mut perfect: Vec<bool>, seed: u64) -> Self {
        let perfects = perfect.iter().filter(|&&p| p).count();
        perfect.push(false);
        let mut out = Self {
            perfect,
            seed,
            accuracy: 0,
            perfects: u32::try_from(perfects).unwrap_or(u32::MAX),
        };
        out.accuracy = out.observed();
        out
    }

    /// Whether a given judgement rolled as Perfect
    #[inline]
    #[must_use]
    pub fn at(&self, judgement: usize) -> bool {
        self.perfect.get(judgement).copied().unwrap_or(true)
    }

    #[must_use]
    pub fn records(&self) -> usize {
        self.perfect.len()
    }

    /// Judgements actually rolled
    #[must_use]
    pub fn rolls(&self) -> u32 {
        u32::try_from(self.records()).unwrap_or(u32::MAX).saturating_sub(1)
    }

    #[must_use]
    pub const fn perfects(&self) -> u32 {
        self.perfects
    }

    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub const fn accuracy(&self) -> Rate {
        self.accuracy
    }

    #[must_use]
    pub fn observed(&self) -> Rate {
        let scaled = u64::from(self.perfects)
            .saturating_mul(u64::from(RATE_UNIT))
            .checked_div(u64::from(self.rolls()))
            .unwrap_or(0);
        Rate::try_from(scaled).unwrap_or(RATE_UNIT)
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::integer_division, reason = "fractions of `RATE_UNIT`")]

    use super::Judgements;
    use crate::play::activation::Triggers;
    use crate::play::config::PlayConfig;
    use crate::play::queue::Queue;
    use crate::play::tests::{arena, at, config, fifty_one, only, scorer, sloppy, tap, worn};
    use crate::play::{Scoring, run};
    use crate::scalar::{RATE_UNIT, Rate};
    use crate::song::{Event, EventKind, Time};

    #[test]
    fn check_all_great_12_perc_lower_than_ap() {
        let stream = [tap(2000, 0), tap(2500, 1), tap(3000, 2)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let scoring = Scoring::flat(800_000); // 10_000 a note flat

        assert_eq!(at(&queue, &scoring, &config()).tap, 3 * 10_000);
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).tap, 3 * 8_800, "0.88 a note");

        for seed in 0..16 {
            let all_perfect = at(&queue, &scoring, &sloppy(PlayConfig::ALL_PERFECT, seed));
            let all_great = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(all_perfect.perfects, 3, "seed {seed}");
            assert_eq!(all_great.perfects, 0, "seed {seed}");
            assert_eq!(all_perfect.tap, 3 * 10_000, "seed {seed}");
            assert_eq!(all_great.tap, 3 * 8_800, "seed {seed}");
        }

        assert_eq!(at(&queue, &scoring, &sloppy(RATE_UNIT * 2, 7)).tap, 3 * 10_000);
    }

    #[test]
    fn check_a_hold_is_perfect_only_if_both_taps_are() {
        const FLIP: Rate = RATE_UNIT / 2;

        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            (Time(3000), Event::new(0, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let scoring = Scoring::flat(800_000);
        let triggers = Triggers::build(&queue, &scoring);

        let mut seen = [false; 4];
        for seed in 0..64 {
            let config = sloppy(FLIP, seed);
            let judged = Judgements::build(&queue, &config);
            assert_eq!(judged.rolls(), 2, "hold is 2 judgements");
            assert_eq!(judged.records(), queue.events().len());

            let (head, release) = (judged.at(0), judged.at(1));
            let out = run(&queue, &config, &scoring, &triggers, &judged);

            let both = head && release;
            assert_eq!(out.notes, 1, "holds score once at the end.");
            assert_eq!(out.perfects, u32::from(both), "seed {seed}: {head} then {release}");
            assert_eq!(
                out.tap,
                match (head, release) {
                    (true, true) => 12_500,
                    (false, false) => 9_680,
                    _ => 11_000,
                },
                "seed {seed}: {head} then {release}",
            );
            assert_eq!(judged.perfects(), u32::from(head) + u32::from(release));
            seen[usize::from(head) * 2 + usize::from(release)] = true;
        }
        assert!(seen.iter().all(|&s| s), "Seeds missed a (head, release) pair: {seen:?}");
    }

    #[test]
    fn check_diff_teams_same_judgements() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut one = Scoring::flat(80_000);
        one.skills[2] = scorer(5, RATE_UNIT / 2, 900);
        let mut two = Scoring::flat(123_456);
        two.lanes = [100, 110, 121, 100, 110, 121, 100, 110, 121];
        two.skills[7] = scorer(2, RATE_UNIT / 4, 40);
        two.accessories[7] = worn(RATE_UNIT / 2, 7);

        let mut differed = false;
        for seed in 0..32 {
            let config = sloppy(8000, seed);
            let judged = Judgements::build(&queue, &config);
            let a = run(&queue, &config, &one, &Triggers::build(&queue, &one), &judged);
            let b = run(&queue, &config, &two, &Triggers::build(&queue, &two), &judged);
            assert_eq!(a.perfects, b.perfects, "seed {seed}");
            assert_eq!(a.perfects, judged.perfects(), "no holds");
            differed |= a.skill != b.skill;
        }
        assert!(differed, "the two teams never rolled differently");
    }
}
