//! Score triggers, aka score-based scorers.

use super::chain::Chain;
use super::fire;
use crate::play::effect::Lasting;
use crate::play::rng::Rng;
use crate::play::{Outcome, Scoring};
use crate::song::{LANES, Time};

/// I wonder how SIF ran on devices that went past 60 FPS. I think it was still hard-capped at 60?
///
/// tbh this is probably way over-engineered, but I'm a sucker for modeling things the hard way
/// even if it makes things harder and contributes almost no additional accuracy.
///
/// Score-based scorers have two properties that are important. A) Their activations contribute to
/// the total needed for their next activation and B) they're only supposed to be able to activate once
/// per frame. So if, in a single tick, enough points are scored for a score-based scorer to try to proc
/// multiple times, it will roll once, wait a frame, and then roll again the next frame, up until the point
/// where it exhausted all of its rolls or it's not allowed to activate anymore because this skill was so
/// busted they nerfed it into the ground.
///
/// I thought of doing a naive implementation where I just roll it on the next ms, which technically speaking
/// is less than one frame at 60 FPS. It'd be way easier than doing FPS math. The difference is so minor.
/// But here I am doing FPS math because why not.
const FPS: u32 = 60;
const MS: u32 = 1000;

/// For score-based scorers when a card needs to roll again on a later frame.
const NO_PASS: Time = Time(u32::MAX);

#[expect(clippy::integer_division, reason = "frame index")]
const fn frame_of(t: Time) -> u32 {
    t.0.saturating_mul(FPS) / MS
}

/// The timestamp of the next frame.
const fn frame_start(k: u32) -> Time {
    Time(k.saturating_mul(MS).div_ceil(FPS))
}

/// I noted in a comment above about how I'm a sucker for over-engineering to get as close to reality.
/// This next section will make me sound like a hypocrite as I will state several assumptions I made
/// that I can not verify. I read through SIFcord's Discord logs and JP wikis to get as much information as I could.
///
/// A) Fixed multiples. If the threshold is 10k points and 10,500 are scored in one go, it is now 9500 from triggering
/// again, not 10,000.
/// B) What happens if the threshold is passed multiple times in one frame. In the 10k threshold example, say a scorer
/// puts up 20k points in one activation. I have chosen to model this as roll the skill for the 10k beind surpassed
/// immediately, then queue the roll for surpassing 20k points to be triggered on the next frame. It may not actually
/// work like this and I can't confirm.
/// C) Skill activations still go from right to left, but score-based scorers trigger as soon as their threshold is passed.
/// So if a lane to the left of a score-based scorer sets it over the score threshold to activate, it will technically break
/// order and trigger immediately, even before more-left skills activate on that same event.
///
/// And the big reason nothing matters is these cards were nerfed into irrelevancy. They have an activation
/// limit placed on them that ranges from 8 to 13 (some cards have different limits) which applies to both their
/// cards skills and accessory skills combined. Well, when they were first nerfed it only applied to the card and
/// not the accessory, but they ended up effectively being a buff in live arena mode so they have to be nerfed
/// again to include the accessory activation count in the total.
#[expect(clippy::doc_markdown, reason = "SIFCord doesn't need backticks.")]
pub(in crate::play) struct Ladder {
    /// Lowest threshold
    low: u64,
    /// The next frame
    pending: Time,
    /// aka thresholds
    rungs: Rungs,
}

struct Rungs {
    step: [u16; LANES],
    next: [u64; LANES],
    lanes: u16,
    /// Lanes that rolled in `frame`.
    spent: u16,
    frame: u32,
    /// Activations left, for the lanes in `limited`.
    /// I wasn't sure what else to call this but I'm a little worried it might conflict with
    /// Spark down the line since the official term for that is charge and here charge is just my
    /// best attempt at a noun to describe how many activations a card has before it hits its activation limit.
    charges: [u8; LANES],
    limited: u16,
}

impl Ladder {
    #[expect(clippy::indexing_slicing, reason = "`lanes` holds lane bits")]
    pub(in crate::play) fn new(scoring: &Scoring, lanes: u16) -> Self {
        let mut ladder = Self {
            low: u64::MAX,
            pending: NO_PASS,
            rungs: Rungs {
                step: [0; LANES],
                next: [u64::MAX; LANES],
                lanes,
                spent: 0,
                frame: 0,
                charges: [0; LANES],
                limited: 0,
            },
        };
        let mut bits = lanes;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let skill = scoring.skills[lane as usize];
            ladder.rungs.step[lane as usize] = skill.threshold;
            ladder.rungs.next[lane as usize] = u64::from(skill.threshold);
            ladder.low = ladder.low.min(u64::from(skill.threshold));
            if let Some(limit) = skill.limit {
                ladder.rungs.charges[lane as usize] = limit.get();
                ladder.rungs.limited |= bit;
            }
        }
        ladder
    }

    #[inline]
    pub(in crate::play) const fn low(&self) -> u64 {
        self.low
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "I benchmarked")]
    pub(in crate::play) fn settle<'q, const YEAR: bool>(
        &mut self,
        now: Time,
        scoring: &Scoring,
        rng: &mut Rng,
        out: &mut Outcome,
        chain: &mut Chain,
        expiry: &mut impl Lasting<'q>,
    ) {
        if out.score() >= self.low {
            self.climb::<YEAR>(0, now, scoring, rng, out, chain, expiry);
        }
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "I benchmarked")]
    #[expect(clippy::too_many_arguments, reason = "the trial's state")]
    pub(in crate::play) fn climb<'q, const YEAR: bool>(
        &mut self,
        first: u16,
        now: Time,
        scoring: &Scoring,
        rng: &mut Rng,
        out: &mut Outcome,
        chain: &mut Chain,
        expiry: &mut impl Lasting<'q>,
    ) {
        let climbed =
            self.rungs.climb::<YEAR>(first, now, scoring, rng.clone(), *out, chain, expiry);
        (*rng, *out, self.low, self.pending) = climbed;
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "I benchmarked")]
    pub(in crate::play) fn catch_up<'q, const YEAR: bool>(
        &mut self,
        until: Time,
        scoring: &Scoring,
        rng: &mut Rng,
        out: &mut Outcome,
        chain: &mut Chain,
        expiry: &mut impl Lasting<'q>,
    ) {
        if self.pending < until {
            let owed = self.rungs.owe::<YEAR>(
                self.pending,
                until,
                scoring,
                rng.clone(),
                *out,
                chain,
                expiry,
            );
            (*rng, *out, self.low, self.pending) = owed;
        }
    }
}

impl Rungs {
    #[inline]
    #[expect(clippy::indexing_slicing, reason = "`lanes` holds lane bits")]
    fn take(&mut self, now: Time, score: u64) -> (u16, u64, Time) {
        let frame = frame_of(now);
        if frame != self.frame {
            self.frame = frame;
            self.spent = 0;
        }
        let mut due = 0;
        let mut low = u64::MAX;
        let mut bits = self.lanes;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let lane = lane as usize;
            if self.next[lane] <= score && self.spent & bit == 0 {
                due |= bit;
                self.next[lane] = self.next[lane].saturating_add(u64::from(self.step[lane]));
            }
            low = low.min(self.next[lane]);
        }
        self.spent |= due;
        let pending = if low <= score {
            frame_start(frame.saturating_add(1))
        } else {
            NO_PASS
        };
        (due, low, pending)
    }

    /// For when a score-based scorer is "owed" an activation on a later frame.
    #[inline(never)]
    #[expect(clippy::too_many_arguments, reason = "the trial's state")]
    fn owe<'q, const YEAR: bool>(
        &mut self,
        mut pending: Time,
        until: Time,
        scoring: &Scoring,
        mut rng: Rng,
        mut out: Outcome,
        chain: &mut Chain,
        expiry: &mut impl Lasting<'q>,
    ) -> (Rng, Outcome, u64, Time) {
        let mut low = 0;
        while pending < until {
            (rng, out, low, pending) =
                self.climb::<YEAR>(0, pending, scoring, rng, out, chain, expiry);
        }
        (rng, out, low, pending)
    }

    /// increments the score counter for score-based scorers.
    #[inline(never)]
    #[expect(clippy::too_many_arguments, reason = "the trial's state")]
    fn climb<'q, const YEAR: bool>(
        &mut self,
        first: u16,
        now: Time,
        scoring: &Scoring,
        mut rng: Rng,
        mut out: Outcome,
        chain: &mut Chain,
        expiry: &mut impl Lasting<'q>,
    ) -> (Rng, Outcome, u64, Time) {
        let (mut due, mut low, mut pending) = (first, 0, NO_PASS);
        loop {
            if out.score() >= low {
                let (taken, next, owed) = self.take(now, out.score());
                (due, low, pending) = (due | taken, next, owed);
            }
            if due == 0 {
                return (rng, out, low, pending);
            }
            let landed = fire_landed(due, now, scoring, &mut rng, &mut out, expiry);
            if self.spend(landed) {
                low = 0;
            }
            if YEAR {
                chain.after(landed, now, scoring, &mut rng, &mut out, expiry);
            }
            due = 0;
        }
    }

    /// Spends a usage of a lane with an activation limit and retires lanes that hit the limit.
    #[inline]
    #[expect(clippy::indexing_slicing, reason = "`We control the lane bits")]
    fn spend(&mut self, landed: u16) -> bool {
        let mut retired = false;
        let mut bits = landed & self.limited;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let charges = &mut self.charges[lane as usize];
            *charges = charges.saturating_sub(1);
            if *charges == 0 {
                self.lanes ^= bit;
                self.limited ^= bit;
                retired = true;
            }
        }
        retired
    }
}

#[inline]
fn fire_landed<'q>(
    due: u16,
    now: Time,
    scoring: &Scoring,
    rng: &mut Rng,
    out: &mut Outcome,
    expiry: &mut impl Lasting<'q>,
) -> u16 {
    expiry.begin();
    let mut landed = 0;
    let mut bits = due;
    while bits != 0 {
        let bit = bits.isolate_highest_one();
        bits ^= bit;
        let before = (out.activations, out.accessory_activations);
        fire::<false>(bit, now, scoring, rng, out, expiry);
        if (out.activations, out.accessory_activations) != before {
            landed |= bit;
        }
    }
    landed
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU8;

    use crate::play::Scoring;
    use crate::play::activation::tests::{FLIP, clocked, draws, laddered, seed_where};
    use crate::play::activation::{Skill, Triggers};
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, arena, fifty_one, only, play, seeded, tap, worn};
    use crate::scalar::Rate;
    use crate::skill::TriggerType;

    /// 2000 vs 1500 means 500 towards the goal of 3000.
    #[test]
    fn check_score_thresholds_are_fixed() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[4] = laddered(1500, ALWAYS, 0);

        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!(triggers.score(), 0b1_0000);
        assert_eq!(triggers.rolls(), 0, "nothing to queue");
        assert!(!triggers.is_empty(), "can roll skill");

        let out = play(&queue, &scoring);
        assert_eq!(out.score(), 51_100);
        assert_eq!(out.activations, 34, "51,100 / 1500");
    }

    #[test]
    fn check_a_scorer_once_per_frame_per_lane() {
        let stream = [tap(2000, 0), tap(2100, 1), tap(2200, 2), tap(2300, 3)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[6] = laddered(1000, ALWAYS, 2500);

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 19);
        assert_eq!(out.skill, 19 * 2500);
    }

    #[test]
    fn check_frame_calculation() {
        let stream = [tap(2000, 0), tap(2010, 1), tap(2100, 2)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[0] = clocked(TriggerType::Time, 2, ALWAYS, 6000);
        scoring.skills[1] = clocked(TriggerType::Combo, 2, FLIP, 100);
        scoring.skills[4] = laddered(3000, FLIP, 1);

        let seed = seed_where(4, |d| d[2] != d[3]);
        let d = draws(seed, 4);
        let want = 6000 + d[1] + d[2] * 100 + d[3];
        assert_ne!(want, 6000 + d[1] + d[2] + d[3] * 100, "both thresholds before the tap");
        assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}: {d:?}");
    }

    #[test]
    fn check_scorer_threshold_accessory_works() {
        let stream = [tap(2000, 0), tap(2100, 1), tap(2200, 2), tap(2300, 3)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[3] = laddered(3000, 0, 1590);
        scoring.accessories[3] = worn(ALWAYS, 9000);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 7));
        assert_eq!(out.skill, 7 * 9000);
    }

    fn limited(threshold: u16, chance: Rate, value: u32, limit: u8) -> Skill {
        Skill {
            limit: NonZeroU8::new(limit),
            ..laddered(threshold, chance, value)
        }
    }

    /// 34 thresholds of 1500 on `fifty_one`, makes sure that it stops activating.
    #[test]
    fn check_a_score_trigger_stops_at_its_limit() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        for (limit, want) in [(0, 34), (1, 1), (5, 5), (33, 33), (34, 34), (200, 34)] {
            let mut scoring = Scoring::flat(80_000);
            scoring.skills[4] = limited(1500, ALWAYS, 0, limit);
            assert_eq!(play(&queue, &scoring).activations, want, "limit {limit}");
        }
    }

    /// I wanted to write an entire paragraph for the skill function but I'll just put it in a comment instead.
    /// This checks that a score-based scorer only has its activation limit consumed when a skill succeeds, either
    /// the card's skill or its accessory.
    #[test]
    fn check_score_activation_only_use_on_activation() {
        const LIMIT: u64 = 5;
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[0] = clocked(TriggerType::Time, 7, FLIP, 1);
        scoring.skills[4] = limited(1500, FLIP, 0, 5);

        let (mut drawing, mut charged) = (false, false);
        for seed in 0..64 {
            let d = draws(seed, 40);
            let (mut i, mut hits) = (0, 0);
            while i < 33 && hits < LIMIT {
                hits += d[i];
                i += 1;
            }
            let timer = d[i];
            if hits < LIMIT {
                hits += d[i + 1];
            }

            let out = seeded(&queue, &scoring, seed);
            assert_eq!(out.skill, timer, "seed {seed}: {d:?}");
            assert_eq!(u64::from(out.activations), hits + timer, "seed {seed}: {d:?}");
            drawing |= timer != d[33];
            charged |= timer != d[5];
        }
        assert!(drawing, "couldn't differentiate");
        assert!(charged, "still activating");
    }

    #[test]
    fn check_an_accessory_spends_the_cards_activatiion_limit() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[4] = limited(1500, 0, 0, 5);
        scoring.accessories[4] = worn(ALWAYS, 0);
        let missed = play(&queue, &scoring);
        assert_eq!((missed.activations, missed.accessory_activations), (0, 5));

        scoring.skills[4].chance = ALWAYS;
        let hit = play(&queue, &scoring);
        assert_eq!((hit.activations, hit.accessory_activations), (5, 0));

        scoring.skills[4].chance = FLIP;
        scoring.accessories[4] = worn(FLIP, 0);
        let mut mixed = false;
        for seed in 0..32 {
            let d = draws(seed, 80);
            let (mut i, mut card, mut acc) = (0, 0, 0);
            for _ in 0..34 {
                if card + acc == 5 {
                    break;
                }
                if d[i] == 1 {
                    card += 1;
                } else {
                    i += 1;
                    acc += d[i];
                }
                i += 1;
            }
            let out = seeded(&queue, &scoring, seed);
            let got = (u64::from(out.activations), u64::from(out.accessory_activations));
            assert_eq!(got, (card, acc), "seed {seed}");
            mixed |= card + acc == 5 && acc > 0;
        }
        assert!(mixed, "accessory never activated before limit reached");
    }

    #[test]
    fn check_activation_limit_is_per_lane() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[2] = limited(1500, ALWAYS, 0, 3);
        scoring.skills[6] = limited(1500, ALWAYS, 0, 7);
        scoring.skills[8] = laddered(1500, ALWAYS, 0);
        assert_eq!(play(&queue, &scoring).activations, 3 + 7 + 34);
    }
}
