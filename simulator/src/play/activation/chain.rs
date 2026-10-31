//! Year Group triggers, aka chain scorers.

use super::fire;
use crate::play::effect::Lasting;
use crate::play::rng::Rng;
use crate::play::{Outcome, Scoring};
use crate::song::{LANES, Time};

/// Year Group aka chain scorers (Year Group is a trigger condition, but every card with it is also a scorer).
/// Requires every other member of the same year group to activate. e.g. Riko needs both Chika and You cards to activate
/// before it can roll. Accessories count for this.
pub(in crate::play) struct Chain {
    lanes: u16,
    seen: [u16; LANES],
}

impl Chain {
    pub(in crate::play) const fn new(lanes: u16) -> Self {
        Self { lanes, seen: [0; LANES] }
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "benchmarked")]
    #[expect(clippy::indexing_slicing, reason = "`lanes` have lane bits")]
    pub(in crate::play) fn after<'q>(
        &mut self,
        landed: u16,
        now: Time,
        scoring: &Scoring,
        rng: &mut Rng,
        out: &mut Outcome,
        expiry: &mut impl Lasting<'q>,
    ) {
        let mut members = 0;
        let mut landings = landed;
        while let Some(lane) = landings.lowest_one() {
            landings ^= landings.isolate_lowest_one();
            members |= scoring.members[lane as usize];
        }

        let mut due = 0;
        let mut chains = self.lanes;
        while let Some(lane) = chains.lowest_one() {
            let bit = chains.isolate_lowest_one();
            chains ^= bit;
            let need = scoring.skills[lane as usize].threshold;
            let seen = &mut self.seen[lane as usize];
            *seen |= members;
            if *seen & need == need {
                due |= bit;
                *seen = 0;
            }
        }
        if due != 0 {
            *rng = self.round(due, now, scoring, rng.clone(), out, expiry);
        }
    }

    #[inline(never)]
    #[expect(clippy::indexing_slicing, reason = "`lanes` have lane bits")]
    fn round<'q>(
        &mut self,
        due: u16,
        now: Time,
        scoring: &Scoring,
        mut rng: Rng,
        out: &mut Outcome,
        expiry: &mut impl Lasting<'q>,
    ) -> Rng {
        expiry.begin();
        let mut bits = due;
        while let Some(lane) = bits.highest_one() {
            let bit = bits.isolate_highest_one();
            bits ^= bit;
            if fire::<true, false, _>(bit, now, scoring, &mut rng, out, expiry) != 0 {
                let member = scoring.members[lane as usize];
                let mut others = self.lanes;
                while let Some(other) = others.lowest_one() {
                    others ^= others.isolate_lowest_one();
                    self.seen[other as usize] |= member;
                }
            }
        }
        expiry.end();
        rng
    }
}

#[cfg(test)]
mod tests {
    use crate::play::Scoring;
    use crate::play::activation::tests::{CLOCKS, FLIP, clocked, draws, laddered, seed_where};
    use crate::play::activation::{Skill, Triggers};
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, fifty_one, only, play, scorer, seeded, worn};
    use crate::scalar::Rate;
    use crate::skill::TriggerType;

    /// Second-year Aqours. Any distinct bits should work. The trial just compares them.
    /// Sorry that there's a million tests for this mechanic.
    const CHIKA: u16 = 1 << 0;
    const YOU: u16 = 1 << 1;
    const RIKO: u16 = 1 << 2;

    fn chained(need: u16, chance: Rate, value: u32) -> Skill {
        clocked(TriggerType::YearGroup, need, chance, value)
    }

    /// Dummy cards whose skills proc every threshold, but their effect doesn't actually do anything.
    /// So we can isolate the year group trigger without having to account for what the others skills
    /// are doing.
    fn member(scoring: &mut Scoring, lane: usize, who: u16, clock: TriggerType, threshold: u16) {
        scoring.skills[lane] = clocked(clock, threshold, ALWAYS, 0);
        scoring.members[lane] = who;
    }

    /// I never called "year group scorers" "chain scorers" but apparently that was sifcord's name for them.
    /// I will be adopting it going forward because it's easier to type.
    #[test]
    fn check_chain_requires_other_members() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        for (often, rarely) in [(CHIKA, YOU), (YOU, CHIKA)] {
            let mut scoring = Scoring::flat(0);
            member(&mut scoring, 8, often, TriggerType::RhythmIcons, 1);
            member(&mut scoring, 5, rarely, TriggerType::RhythmIcons, 5);
            scoring.skills[0] = chained(CHIKA | YOU, ALWAYS, 1);
            scoring.members[0] = RIKO;
            let triggers = Triggers::build(&queue, &scoring);
            assert_eq!(triggers.year(), 0b1);
            assert_eq!(triggers.rolls(), 51 + 10, "nothing pre-scheduled for chain scorers");

            let out = play(&queue, &scoring);
            assert_eq!(out.skill, 10, "once per 5 icons");
            assert_eq!(out.activations, 51 + 10 + 10);
        }

        let mut alone = Scoring::flat(0);
        member(&mut alone, 8, CHIKA, TriggerType::RhythmIcons, 1);
        member(&mut alone, 5, RIKO, TriggerType::RhythmIcons, 1);
        alone.skills[0] = chained(CHIKA | YOU, ALWAYS, 1);
        alone.members[0] = RIKO;
        assert_eq!(play(&queue, &alone).skill, 0, "no You, no chain");
    }

    /// Riko in left-most slot 1, You and Chika alternating in slots 2-9. The question is, if You and Chika have the same
    /// threshold, and all 8 of them proc their skills at the same time, does Riko proc 1 or 4 times? After all,
    /// since skill activations happen right->left, Riko's condition is satisfied after lanes 8 and 9 proc, theoretically
    /// letting her roll, then roll again when lanes 6 and 7 proc, then again when 5 and 4, and finally when 2 and 3 proc.
    ///
    /// But this apparently, from what I can gather from sifcord logs, is not actually what happens. You get at most
    /// one activation at any given moment for chain scorers. And they don't bank and roll on the next frame like score scorers do.
    #[test]
    fn check_a_chain_goes_once_per_moment() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        for (clock, ticks) in CLOCKS {
            let mut scoring = Scoring::flat(0);
            for lane in 1..9 {
                member(&mut scoring, lane, [YOU, CHIKA][lane % 2], clock, 1);
            }
            scoring.skills[0] = chained(CHIKA | YOU, ALWAYS, 2500);
            scoring.members[0] = RIKO;

            let out = play(&queue, &scoring);
            assert_eq!(out.skill, u64::from(ticks) * 2500, "{clock:?}");
            assert_eq!(out.activations, u32::from(ticks) * 9, "{clock:?}");
        }
    }

    /// Another assumption I can't verify, related to the one above. Extra members proccing at the same time
    ///  a chain threshold is satisfied means none of them carry over to the next threshold.
    /// with it.
    /// Here we have Chikas in 8 and 2, a You in 5 every 4 icons, another You in 7 every 6.
    /// If the second Chika carried over, the the You at icon 6 would set the chain off again, at 6, 18, 30 and 42.
    #[test]
    fn check_a_moments_extra_members_dont_carry_over() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        member(&mut scoring, 8, CHIKA, TriggerType::RhythmIcons, 4);
        member(&mut scoring, 7, YOU, TriggerType::RhythmIcons, 6);
        member(&mut scoring, 5, YOU, TriggerType::RhythmIcons, 4);
        member(&mut scoring, 2, CHIKA, TriggerType::RhythmIcons, 4);
        scoring.skills[0] = chained(CHIKA | YOU, ALWAYS, 1);
        scoring.members[0] = RIKO;
        assert_eq!(play(&queue, &scoring).skill, 12, "every 4 icons but didn't go 16 times.");
    }

    /// Chain scorers break the right->left skill activation paradigm. Or at least all evidence points to
    /// this being in the case. Every other skill rolls, then chain scorer is checked, and then if rolls if
    /// its trigger was satisfied.
    #[test]
    fn check_chain_rolls_last() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        scoring.skills[8] = chained(CHIKA | YOU, FLIP, 1000);
        scoring.members[8] = RIKO;
        member(&mut scoring, 6, CHIKA, TriggerType::RhythmIcons, 51);
        member(&mut scoring, 4, YOU, TriggerType::RhythmIcons, 51);
        scoring.skills[2] = scorer(51, FLIP, 1);

        let seed = seed_where(4, |d| d[2] != d[3]);
        let d = draws(seed, 4);
        let want = d[2] + d[3] * 1000;
        assert_ne!(
            want,
            d[2] * 1000 + d[3],
            "chain would draw before lane 3 if it were right->left"
        );
        assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}: {d:?}");
    }

    #[test]
    fn check_an_accessory_counts_as_its_card() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        scoring.skills[8] = clocked(TriggerType::RhythmIcons, 1, 0, 0);
        scoring.members[8] = CHIKA;
        member(&mut scoring, 5, YOU, TriggerType::RhythmIcons, 1);
        scoring.skills[0] = chained(CHIKA | YOU, ALWAYS, 1);
        scoring.members[0] = RIKO;
        assert_eq!(play(&queue, &scoring).skill, 0, "Chika never activates");

        scoring.accessories[8] = worn(ALWAYS, 0);
        let hers = play(&queue, &scoring);
        assert_eq!(hers.skill, 51, "her accessory does");
        assert_eq!(hers.accessory_activations, 51);

        scoring.skills[0].chance = 0;
        scoring.accessories[0] = worn(ALWAYS, 9000);
        let rikos = play(&queue, &scoring);
        assert_eq!(
            (rikos.accessory_activations, rikos.skill),
            (2 * 51, 51 * 9000),
            "and so does Riko's"
        );
    }

    #[test]
    fn check_chain_resets_if_skill_fails() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        member(&mut scoring, 8, CHIKA, TriggerType::RhythmIcons, 1);
        member(&mut scoring, 5, YOU, TriggerType::RhythmIcons, 10);
        scoring.skills[0] = chained(CHIKA | YOU, FLIP, 1);
        scoring.members[0] = RIKO;

        let mut missed = false;
        for seed in 0..32 {
            let d = draws(seed, 51 + 5 + 5);
            let mut i = 0;
            let mut want = 0;
            for icon in 1..=51 {
                i += 1;
                if icon % 10 == 0 {
                    want += d[i + 1];
                    i += 2;
                }
            }
            assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}: {d:?}");
            missed |= want < 5;
        }
        assert!(missed, "chain skill never missed");
    }

    /// Riko chains in 8 and 2, a You chain between them, a Chika in 9 and a plain Riko in 1, all landing every icon.
    /// The You chain starts it off of the plain Riko at icon 1, then all three go every icon. Since a chain resets when
    /// all other events have been processed at a given time, the You chain proccing in the counts for both Rikos' next cycle.
    #[test]
    fn check_chains_proc_each_other() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        member(&mut scoring, 8, CHIKA, TriggerType::RhythmIcons, 1);
        member(&mut scoring, 0, RIKO, TriggerType::RhythmIcons, 1);
        scoring.skills[7] = chained(CHIKA | YOU, ALWAYS, 1);
        scoring.skills[4] = chained(CHIKA | RIKO, ALWAYS, 1000);
        scoring.skills[1] = chained(CHIKA | YOU, ALWAYS, 1_000_000);
        (scoring.members[7], scoring.members[4], scoring.members[1]) = (RIKO, YOU, RIKO);
        assert_eq!(Triggers::build(&queue, &scoring).year(), 0b1001_0010);

        let out = play(&queue, &scoring);
        assert_eq!(out.skill, 50 + 51 * 1000 + 50 * 1_000_000);
    }

    #[test]
    fn check_score_scorer_counts_for_chain_scorer() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[8] = laddered(1000, ALWAYS, 0);
        scoring.members[8] = CHIKA;
        member(&mut scoring, 5, YOU, TriggerType::RhythmIcons, 1);
        scoring.skills[0] = chained(CHIKA | YOU, ALWAYS, 1);
        scoring.members[0] = RIKO;

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 51 + 51 + 46, "a threshold per tap");
        assert_eq!(out.skill, 46);
    }

    #[test]
    fn check_chain_score_increments_score_scorer_threshold() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        member(&mut scoring, 8, CHIKA, TriggerType::RhythmIcons, 1);
        member(&mut scoring, 5, YOU, TriggerType::RhythmIcons, 1);
        scoring.skills[3] = laddered(10_000, ALWAYS, 0);
        scoring.skills[0] = chained(CHIKA | YOU, ALWAYS, 10_000);
        scoring.members[0] = RIKO;

        let out = play(&queue, &scoring);
        assert_eq!(out.skill, 51 * 10_000);
        assert_eq!(out.activations, 51 * 4);
    }
}
