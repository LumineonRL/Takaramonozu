//! Spark. 
//!
//! Accessory-only skill.
//! Every time an Encore skill goes off it banks a charge. Each spark has their own threshold which indicates how many charges need to be built up
//! before its skill can activate. Successfully activating spends as many charges as it can. So if the threshold is 5 and 12 charges are banked,
//! it spends 10 charges. Activation value is multiplied by the number of charges being spent.
//!
//! It wouldn't be an Encore-related skill if it didn't have some weird edge cases associated with it that I will once again do my best to document.
//! - An encore card that copies a skill that takes an effect gives +1 charge
//! - An encore card copies an SRU that does nothing because one is already active  + 1 each
//! - An Encore card copies a Param Boost +1
//! - An Encore card copies a Spark even for no benefit +1 each
//! - Several Encore cards copy one Amp at the exact same time +1 total
//! - An Encore card copies an Amp that fired at the exact same moment +0
//! - An Encore activates with nothing to copy +0
//! - Any accessory on an Encore card activates +1. Yes, even if the accessory is not an Encore.
//! - An Encore accessory activates on a card that is not an Encore +0
//!
//! So yeah this skill is quite unique in how it cares about the *card's* skill rather than the skill being activated. Usually it's the opposite.


use super::encore::{Copied, Repeats, Slot};

#[derive(Clone, Copy, Default, Debug)]
pub(in crate::play) struct Charged {
    slot: Slot,
    pool: u32,
}

impl Repeats for Charged {
    const ON: bool = true;
    const SPARK: bool = true;

    #[inline(always)]
    fn write(&mut self, landed: Copied) {
        self.slot.write(landed);
    }

    #[inline(always)]
    fn copy(&mut self) -> Option<Copied> {
        self.slot.copy()
    }

    #[inline(always)]
    fn end(&mut self) {
        self.slot.end();
    }

    #[inline(always)]
    fn pick(&mut self, lane: u32, picked: u16) {
        self.slot.pick(lane, picked);
    }

    #[inline(always)]
    fn picked(&self, lane: u32) -> u16 {
        self.slot.picked(lane)
    }

    #[inline(always)]
    fn ready(&self, threshold: u8) -> bool {
        self.pool >= u32::from(threshold)
    }

    #[inline(always)]
    fn spend(&mut self, threshold: u8) -> u32 {
        let threshold = u32::from(threshold);
        let multiples = self.pool.checked_div(threshold).unwrap_or(0);
        self.pool = self.pool.checked_rem(threshold).unwrap_or(self.pool);
        multiples
    }

    #[inline(always)]
    fn bank(&mut self) {
        self.pool = self.pool.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::Charged;
    use crate::play::Scoring;
    use crate::play::activation::{AccessorySkill, Skill, Triggers};
    use crate::play::effect::encore::Repeats;
    use crate::play::effect::tests::{FLIP, TEAM, encore, others_first};
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, fifty_one, only, play, scorer, seeded, worn};
    use crate::skill::SkillEffect;

    const SPARK: AccessorySkill =
        AccessorySkill { value: 0, chance: ALWAYS, effect: SkillEffect::Spark };

    /// Spark skill on a card with an every RI threshold and never activates.
    fn spark(scoring: &mut Scoring, lane: usize, every: u16, threshold: u8) {
        scoring.skills[lane] = scorer(every, 0, 0);
        (scoring.accessories[lane], scoring.sparks[lane]) = (SPARK, threshold);
    }

    /// 21 charges with a threshold of 5
    /// Should spend 20 keep 1.
    #[test]
    fn check_spend() {
        let mut charged = Charged::default();
        (0..21).for_each(|_| charged.bank());
        assert!(charged.ready(21) && !charged.ready(22));
        assert_eq!(charged.spend(5), 4);
        assert!(charged.ready(1) && !charged.ready(2), "kept the remainder");
        assert_eq!(charged.spend(5), 0, "short");
        assert!(charged.ready(1), "keep the charges for itself that selfish skill");
        assert!(Charged::default().ready(0));
    }

    /// No Encore so no charges so no chances to proc
    #[test]
    fn check_no_charges_no_draws() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let copier = AccessorySkill { effect: SkillEffect::Encore, ..worn(FLIP, 0) };
        for worn_on_2 in [AccessorySkill::NONE, copier] {
            let mut bare = Scoring::flat(TEAM);
            (bare.skills[2], bare.accessories[2]) = (scorer(1, FLIP, 1), worn_on_2);
            bare.skills[6] = scorer(1, FLIP, 100);
            let mut sparked = bare;
            (sparked.accessories[6], sparked.sparks[6]) = (SPARK, 1);
            assert_eq!(Triggers::build(&queue, &sparked).spark(), 1 << 6);
            for seed in 0..32 {
                let (got, want) = (seeded(&queue, &sparked, seed), seeded(&queue, &bare, seed));
                assert_eq!(got, want, "seed {seed}, {worn_on_2:?}");
            }
        }
    }

    /// Banks every 5 RI
    /// Spark threshold 3 every 7 icons.
    /// Procs on 21 with 4 charges and keeps 1, then procs again on 35 before that icon's bank, then finally on 49.
    /// If all 4 were spent on 21 then nothing would happen on 49.
    #[test]
    fn check_banker() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut base = Scoring::flat(TEAM);
        spark(&mut base, 0, 7, 3);
        let mut copies = base;
        copies.skills[8] = scorer(5, ALWAYS, 1000);
        copies.skills[4] = encore(5, ALWAYS);
        let mut worn_on_one = base;
        worn_on_one.skills[4] = encore(5, 0);
        worn_on_one.accessories[4] = worn(ALWAYS, 1);
        let mut nothing = base;
        nothing.skills[4] = encore(5, ALWAYS);

        let landed = |team: &Scoring| {
            let out = play(&queue, team);
            (out.activations, out.accessory_activations)
        };
        assert_eq!(landed(&copies), (20, 3), "card's copies");
        assert_eq!(landed(&worn_on_one), (0, 10 + 3), "acc on card");
        assert_eq!(landed(&nothing), (10, 0), "nothing to copy");
    }

    #[test]
    fn check_amplify_copies_bank_once_a_moment() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[7] = Skill { effect: SkillEffect::Amplify, ..scorer(10, ALWAYS, 4) };
        (scoring.skills[4], scoring.skills[5]) = (encore(5, ALWAYS), encore(5, ALWAYS));
        spark(&mut scoring, 0, 1, 1);
        let mut coins = [false; 2];
        for seed in 0..8 {
            assert_eq!(seeded(&queue, &scoring, seed).accessory_activations, 4, "seed {seed}");
            coins[usize::from(others_first(seed))] = true;
        }
        assert_eq!(coins, [true; 2]);
    }

    /// Scorer on 25 and 50
    /// Encore every 5 and Spark w/ threshold 1 on every icon.
    /// Spark lands on 26, the Encore copies it on 30 and doesn't have a charge to spend. 
    /// It banks a charge after, then the Spark procs again on 31, and so on to 46. 
    /// Copying the scorer on 50 banks one more for 51.
    /// Did you get all that?
    #[test]
    fn check_copy_banks_after_it_spends() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[8] = scorer(25, ALWAYS, 1000);
        scoring.skills[4] = encore(5, ALWAYS);
        spark(&mut scoring, 0, 1, 1);
        let out = play(&queue, &scoring);
        assert_eq!((out.skill, out.activations, out.accessory_activations), (4000, 12, 6));
    }
}
