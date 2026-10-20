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

use crate::play::queue::Queue;
use crate::skill::SkillEffect;
use crate::song::{LANES, Time};

/// skill encore can replay, including amp boosts
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::play) struct Copied {
    pub lane: u32,
    pub worn: bool,
    pub effect: SkillEffect,
    pub levels: u32,
}

const EFFECTS: [SkillEffect; 12] = [
    SkillEffect::Scorer,
    SkillEffect::Healer,
    SkillEffect::PerfectLock,
    SkillEffect::Encore,
    SkillEffect::Amplify,
    SkillEffect::TotalTrick,
    SkillEffect::PerfectScoreUp,
    SkillEffect::SkillRateUp,
    SkillEffect::AppealBoost,
    SkillEffect::ComboFever,
    SkillEffect::Sync,
    SkillEffect::Spark,
];

impl Copied {
    /// The lane, whether it was a card/acc, the effect, and the level it proc'd at.
    /// Basically everything an Encore needs to resolve its skill packed into the smallest
    /// data structure I could get.
    #[expect(clippy::cast_possible_truncation, reason = "a lane and at most 16 levels")]
    #[expect(clippy::host_endian_bytes, reason = "gets packed and unpacked in the same trial")]
    const fn pack(self) -> u32 {
        let levels = (self.levels as u8).saturating_add(1);
        u32::from_ne_bytes([self.lane as u8, self.worn as u8, self.effect as u8, levels])
    }

    #[expect(clippy::host_endian_bytes, reason = "gets packed and unpacked in the same trial")]
    fn unpack(packed: u32) -> Option<Self> {
        let [lane, worn, effect, levels] = packed.to_ne_bytes();
        Some(Self {
            lane: u32::from(lane),
            worn: worn != 0,
            effect: *EFFECTS.get(usize::from(effect))?,
            levels: u32::from(levels.checked_sub(1)?),
        })
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub(in crate::play) struct Slot {
    /// The `pack`'d version of the last skill, see above.
    last: u32,
    /// First skill to proc on a moment
    temp: u32,
    /// If an encore copied something this moment
    copied: bool,
    /// Specifically for Sync (why do the worst skills need the most edge-case handling? Total Trick was the same.)
    /// This is specifically the bit of the lane each lane's last Sync copied (or 0).
    /// That lane then gets copied.
    picks: [u16; LANES],
}

/// This one is specifically for Amp+Encore
/// The slot on the team with Amp if Amp is present.
pub(in crate::play) trait Repeats<'q>: Default {
    const ON: bool;
    const SPARK: bool = false;

    #[inline(always)]
    fn new(_queue: &'q Queue<'_>) -> Self {
        Self::default()
    }

    fn write(&mut self, landed: Copied);
    fn copy(&mut self) -> Option<Copied>;
    fn end(&mut self);
    fn pick(&mut self, lane: u32, picked: u16);
    fn picked(&self, lane: u32) -> u16;

    fn ready(&self, _threshold: u8) -> bool {
        debug_assert!(false, "no Spark");
        true
    }

    fn spend(&mut self, _threshold: u8) -> u32 {
        debug_assert!(false, "no Spark");
        0
    }

    fn bank(&mut self) {
        debug_assert!(false, "no Spark");
    }

    fn open(&mut self, _lane: u32, _end: Time, _bonus: u32, _notes: u32) {
        debug_assert!(false, "no Spark");
    }

    fn tap(&mut self, _now: Time) -> u64 {
        debug_assert!(false, "no Spark");
        0
    }

    fn per_note(&mut self) {}

    fn paid(&self) -> u64 {
        0
    }
}

/// This one's for an Amplify team with no Encore and is entirely here for performance reasons.
/// Basically if a team has Amplify but does not have an Encore, we pass this dummy struct that looks
/// like Encore with ZSTs so nothing different needs to be done.
/// Legit a 7% performance hit to not have this struct
#[derive(Clone, Copy, Default, Debug)]
pub(in crate::play) struct NoRepeat;

impl Repeats<'_> for NoRepeat {
    const ON: bool = false;

    #[inline(always)]
    fn write(&mut self, _: Copied) {}

    #[inline(always)]
    fn copy(&mut self) -> Option<Copied> {
        None
    }

    #[inline(always)]
    fn end(&mut self) {}

    #[inline(always)]
    fn pick(&mut self, _: u32, _: u16) {}

    #[inline(always)]
    fn picked(&self, _: u32) -> u16 {
        0
    }
}

impl Repeats<'_> for Slot {
    const ON: bool = true;

    #[inline(always)]
    fn write(&mut self, landed: Copied) {
        self.temp = if self.temp == 0 {
            landed.pack()
        } else {
            self.temp
        };
    }

    /// What an Encore proc now repeats. Updates mid-moment.
    #[inline(always)]
    fn copy(&mut self) -> Option<Copied> {
        let target = if self.temp == 0 { self.last } else { self.temp };
        self.copied |= target != 0;
        Copied::unpack(target)
    }

    #[inline(always)]
    fn end(&mut self) {
        let kept = if self.copied { 0 } else { self.last };
        self.last = if self.temp == 0 { kept } else { self.temp };
        (self.temp, self.copied) = (0, false);
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    fn pick(&mut self, lane: u32, picked: u16) {
        self.picks[lane as usize] = picked;
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    fn picked(&self, lane: u32) -> u16 {
        self.picks[lane as usize]
    }
}

#[cfg(test)]
mod tests {
    use crate::play::Scoring;
    use crate::play::activation::{AMP_LEVELS, Payload, Skill, Triggers};
    use crate::play::effect::tests::{
        FLIP, STATS, TEAM, clocked, draws, encore, lock, others_first, param, sync,
    };
    use crate::play::queue::Queue;
    use crate::play::rng::Rng;
    use crate::play::tests::{ALWAYS, fifty_one, only, play, scorer, seeded, worn};
    use crate::skill::{SkillEffect, TriggerType};

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

    /// Param boost every 2s for 1s,
    /// copied on icon 26 at 3900. Checking that it gets boosted like one of the Encore's
    /// own would. So it reads the original's targets and a Sync at combo 22 copies lane 1 boosted
    /// and the duration begins in the Encore's lane.
    /// Then lane 8 procs at 4000 rather BUT that lane is blocked. phew. Did you get all that?
    #[test]
    fn check_copied_param() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut copied = Scoring { stats: STATS, ..Scoring::flat(TEAM) };
        param(&mut copied, 8, TriggerType::Time, 2, ALWAYS, 12_000, 1000);
        (copied.targets[8][0], copied.boosts[8][0]) = (1 << 1, 50);
        sync(&mut copied, 0, TriggerType::Combo, 22, ALWAYS, 1000, 1 << 1);
        copied.skills[4] = encore(26, ALWAYS);
        let mut own = copied;
        param(&mut own, 4, TriggerType::RhythmIcons, 26, ALWAYS, 12_000, 1000);
        (own.targets[4][0], own.boosts[4][0]) = (1 << 1, 50);
        let mut idle = copied;
        idle.skills[4] = scorer(26, ALWAYS, 0);
        let out = play(&queue, &copied);
        assert_eq!(out, play(&queue, &own));
        assert_ne!(out.tap, play(&queue, &idle).tap, "lane 8 boosted at 4000");

        let mut blocked = Scoring { stats: STATS, ..Scoring::flat(TEAM) };
        param(&mut blocked, 8, TriggerType::RhythmIcons, 30, ALWAYS, 12_000, 2500);
        blocked.skills[4] = encore(5, ALWAYS);
        assert_eq!(play(&queue, &blocked).activations, 1 + 10, "lane was not blocked by Encore");
    }

    /// Amplify at 4000 boosts the skill at 5000 to 100%.
    /// Encore at 6000 copies it at that percent.
    /// Then a Sync at combo 41 at the exact same time only gets the copied boost.
    #[test]
    fn check_copied_param_at_its_amped_percent() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut amped = Scoring { stats: STATS, ..Scoring::flat(TEAM) };
        param(&mut amped, 8, TriggerType::Time, 5, ALWAYS, 12_000, 300);
        (amped.targets[8][0], amped.boosts[8][0]) = (1 << 1, 50);
        amped.amplified[8][0] = [Payload { value: 24_000, duration: 300, extra: 100 }; AMP_LEVELS];
        amped.skills[7] = Skill {
            effect: SkillEffect::Amplify,
            ..clocked(TriggerType::Time, 4, ALWAYS, 4)
        };
        amped.skills[4] = Skill { trigger: TriggerType::Time, ..encore(6, ALWAYS) };
        sync(&mut amped, 0, TriggerType::Combo, 41, ALWAYS, 500, 1 << 1);
        let mut plain = amped;
        plain.skills[7] = clocked(TriggerType::Time, 4, ALWAYS, 0);
        (plain.skills[8].value, plain.boosts[8][0]) = (24_000, 100);
        let mut idle = amped;
        idle.skills[4] = clocked(TriggerType::Time, 6, ALWAYS, 0);
        let out = play(&queue, &amped);
        assert_eq!(out, play(&queue, &plain));
        assert!(out.tap > play(&queue, &idle).tap, "the copy boosted");
    }

    /// Sync on icon 30 picks has two options, either lanes 0 or 1.
    /// Encore on 40 copying it syncs to the same lane, whichever rolled before.
    #[test]
    fn check_copied_sync() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut copied = Scoring { stats: STATS, ..Scoring::flat(TEAM) };
        sync(&mut copied, 8, TriggerType::RhythmIcons, 30, ALWAYS, 300, 0b11);
        copied.skills[4] = encore(40, ALWAYS);
        let mut idle = copied;
        idle.skills[4] = scorer(40, ALWAYS, 0);
        let mut seen = [false; 2];
        for seed in 0..16 {
            let lane = Rng::new(seed, Rng::SYNC).below(2);
            let mut own = copied;
            sync(&mut own, 4, TriggerType::RhythmIcons, 40, ALWAYS, 300, 1 << lane);
            let out = seeded(&queue, &copied, seed);
            assert_eq!(out, seeded(&queue, &own, seed), "seed {seed}");
            assert_ne!(out.tap, seeded(&queue, &idle, seed).tap, "seed {seed}: the copy synced");
            seen[lane as usize] = true;
        }
        assert_eq!(seen, [true; 2]);
    }

    #[test]
    fn check_encore_counts_for_year_group() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[4] = encore(10, ALWAYS);
        scoring.skills[0] = clocked(TriggerType::YearGroup, 1 << 4, ALWAYS, 1000);
        (scoring.members[4], scoring.members[0]) = (1 << 4, 1);
        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill), (5 + 5, (5 + 4) * 1000));
    }

    #[test]
    fn check_packing() {
        for (index, &effect) in super::EFFECTS.iter().enumerate() {
            assert_eq!(effect as usize, index, "{effect:?}");
            for (lane, worn, levels) in [(0, false, 0), (8, true, 16), (4, false, 7)] {
                let copy = super::Copied { lane, worn, effect, levels };
                assert_eq!(super::Copied::unpack(copy.pack()), Some(copy));
            }
        }
        assert_eq!(super::EFFECTS.len(), SkillEffect::Spark as usize + 1);
        assert_eq!(super::Copied::unpack(0), None);
    }
}
