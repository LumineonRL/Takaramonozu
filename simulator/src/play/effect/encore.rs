//! Encore. Repeats previous skill that activated.
//!
//! I have no idea how to implement this without `.clone` this will be fun.
//! Like amp I guess I'll do a mechanic dump here.
//! Block duration skills like SRU still write the last activated slot.
//! First non-Encore skill to activate on a given moment is saved. Activations normally happen
//! from right to left. Except when Amps are involved. There's a 50/50 chance that amps go first
//! regardless of what lane they're in and ruin everything see: coinflip mechanic.
//!
//! There's a cycle mechanic. See the tests for more info but here's a tldr
//! Wait I ended up typing up way more here than in the tests I guess this wasn't much of a tldr
//! at all.
//! On a 1st cycle, say a scorer and some Encores proc on the same note. The scorer would
//! write the copy slot and the Encores would copy it.
//! On the 2nd cycle, the scorer is still in the slot. The next time an Encore procs, it copies
//! the scorer again.
//! This can't happen a third time i.e. there is no third cycle. If an Encore would fire on a tick
//! where nothing copyable was written, the slot is poised at the end of the tick and any later
//! Encore would have nothing to copy until a skill procs again.
//!
//! Here's an example:
//! Note 50: Scorer + Encores proc. Encores copy the scorer (1st cycle). Scorer stays in copy slot.
//! Note 75: Encores only proc. Encores copy the scorer again (2nd cycle). Slot is poisoned.
//! Note 100: Scorer + Encores proc. Starts over.
//!
//! If every encore were to whiff on note 75, nothing would get poisoned and the scorer would stay the
//! target for the next encore.
//!
//! If the scorers weren't synced it might look like this:
//! 58 note scorer -> Encore copies it at 75 and poisons the slot -> note 100 nothing to copy
//! 116 scorer -> 125 encore copy again. Only ever one cycle if the activations aren't simultaneous.

use crate::skill::SkillEffect;

/// skill encore can replay, including amp boosts
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::play) struct Copied {
    pub lane: u32,
    pub worn: bool,
    pub effect: SkillEffect,
    pub levels: u32,
}

#[derive(Clone, Copy, Default, Debug)]
pub(in crate::play) struct Slot {
    last: Option<Copied>,
    /// First skill to proc on a moment
    temp: Option<Copied>,
    /// If an encore copied something this moment
    copied: bool,
}

impl Slot {
    #[inline(always)]
    pub(in crate::play) fn write(&mut self, landed: Copied) {
        self.temp = self.temp.or(Some(landed));
    }

    /// What an Encore proc now repeats. Updates mid-moment.
    #[inline(always)]
    pub(in crate::play) fn copy(&mut self) -> Option<Copied> {
        let target = self.temp.or(self.last);
        self.copied |= target.is_some();
        target
    }

    #[inline(always)]
    pub(in crate::play) fn end(&mut self) {
        let kept = if self.copied { None } else { self.last };
        self.last = self.temp.take().or(kept);
        self.copied = false;
    }
}

#[cfg(test)]
mod tests {
    use crate::play::Scoring;
    use crate::play::activation::{Skill, Triggers};
    use crate::play::effect::tests::{FLIP, TEAM, draws, lock, others_first};
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, fifty_one, only, play, scorer, seeded, worn};
    use crate::scalar::Rate;
    use crate::skill::{SkillEffect, TriggerType};

    fn encore(threshold: u16, chance: Rate) -> Skill {
        Skill { effect: SkillEffect::Encore, ..scorer(threshold, chance, 0) }
    }

    /// Scorer every 10 icons, Encore every 5.
    /// Each scorer gets copied on its own icon and next Encore then the slot empties.
    /// Scorer every 12 instead -> Encore copies it once.
    #[test]
    fn check_second_cycle_but_not_third() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[3] = encore(5, ALWAYS);
        scoring.skills[6] = scorer(10, ALWAYS, 1000);
        assert_eq!(Triggers::build(&queue, &scoring).encore(), 1 << 3);
        let synced = play(&queue, &scoring);
        assert_eq!((synced.activations, synced.skill), (5 + 10, (5 + 9) * 1000), "10, 15, …, 50");

        scoring.skills[6] = scorer(12, ALWAYS, 1000);
        let unsynced = play(&queue, &scoring);
        assert_eq!((unsynced.activations, unsynced.skill), (4 + 10, 8 * 1000), "15, 25, 40, 50");
    }

    /// Encore is always last regardless of coinflip result.
    #[test]
    fn check_encores_go_last() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut plain = Scoring::flat(TEAM);
        plain.skills[8] = scorer(1, FLIP, 100);
        plain.skills[5] = encore(1, FLIP);
        plain.skills[2] = scorer(1, FLIP, 1);
        let mut ghost = plain;
        ghost.skills[0] = Skill { effect: SkillEffect::Amplify, ..scorer(1, 0, 4) };

        for seed in 0..32 {
            for (team, rolls) in [(&plain, 3), (&ghost, 4)] {
                let (big, small) = if rolls == 4 && !others_first(seed) {
                    (1, 2)
                } else {
                    (0, 1)
                };
                let (mut want, mut last) = (0, None);
                for icon in draws(seed, rolls * 51).chunks(rolls) {
                    let first = match (icon[big], icon[small]) {
                        (1, _) => Some(100),
                        (_, 1) => Some(1),
                        _ => None,
                    };
                    let copy = if icon[rolls - 1] == 1 {
                        first.or(last)
                    } else {
                        None
                    };
                    want += 100 * icon[big] + icon[small] + copy.unwrap_or(0);
                    last = first.or(if copy.is_some() { None } else { last });
                }
                assert_eq!(seeded(&queue, team, seed).skill, want, "seed {seed}, {rolls} rolls");
            }
        }
    }

    #[test]
    fn check_an_accessory_on_an_encore() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        for (carrier, other, want) in [(6, 2, 5 * 1000), (2, 6, 500 + 4 * 1000)] {
            let mut scoring = Scoring::flat(TEAM);
            scoring.skills[carrier] = encore(10, 0);
            scoring.accessories[carrier] = worn(ALWAYS, 500);
            scoring.skills[other] = encore(10, ALWAYS);
            let out = play(&queue, &scoring);
            let got = (out.activations, out.accessory_activations, out.skill);
            assert_eq!(got, (5, 5, want), "carrier on {carrier}");
        }

        let mut alone = Scoring::flat(TEAM);
        alone.skills[4] = encore(10, ALWAYS);
        alone.accessories[4] = worn(ALWAYS, 500);
        let out = play(&queue, &alone);
        let got = (out.activations, out.accessory_activations, out.skill);
        assert_eq!(got, (5, 0, 0), "nothing to copy, acc doesn't roll");
    }

    /// PL on icon 30 for 300 ms, Encore on 40 copies it
    /// starts a new lock from 40 owned by the encore.
    #[test]
    fn check_copied_lock() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut copied = Scoring::flat(TEAM);
        lock(&mut copied, 8, TriggerType::RhythmIcons, 30, ALWAYS, 300);
        copied.skills[4] = encore(40, ALWAYS);
        let mut own = copied;
        lock(&mut own, 4, TriggerType::RhythmIcons, 40, ALWAYS, 300);
        let mut idle = copied;
        idle.skills[4] = scorer(40, ALWAYS, 0);
        let out = play(&queue, &copied);
        assert_eq!(out, play(&queue, &own));
        assert!(out.tap > play(&queue, &idle).tap, "the copy locked");

        copied.durations[8][0] = 2500;
        copied.skills[4].threshold = 5;
        assert_eq!(play(&queue, &copied).activations, 1 + 6 + 1, "5 to 30, then once at 6800");
    }
}
