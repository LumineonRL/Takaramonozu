//! Skill activation trigger conditions e.g. rhythm icons, combo, etc.

use super::{Outcome, Queue, Scoring};
use crate::rng::Rng;
use crate::scalar::{Centi, Rate};
use crate::skill::{SkillEffect, TriggerType};
use crate::song::{LANES, Time};

/// Won't work for all skills, but there are some that we can pre-compute at compile-time
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Skill {
    pub value: Centi,
    pub chance: Rate,
    pub threshold: u16,
    pub effect: SkillEffect,
    pub trigger: TriggerType,
}

impl Skill {
    pub const NONE: Self = Self {
        value: 0,
        chance: 0,
        threshold: 0,
        effect: SkillEffect::Scorer,
        trigger: TriggerType::RhythmIcons,
    };
}

impl Default for Skill {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessorySkill {
    pub value: Centi,
    pub chance: Rate,
    pub effect: SkillEffect,
}

impl AccessorySkill {
    pub const NONE: Self = Self { value: 0, chance: 0, effect: SkillEffect::Scorer };

    #[must_use]
    pub const fn is_inert(self) -> bool {
        self.chance == 0
    }
}

impl Default for AccessorySkill {
    fn default() -> Self {
        Self::NONE
    }
}

// Can pre-add to the queue certain activations like rhythm icons, combo and time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Schedule {
    at: Vec<u32>,
    lanes: Vec<u16>,
}

impl Schedule {
    /// Pre-schedules each lane's skill activations if their trigger type allows for it.
    /// We can pre-schedule every condition besides perfects, scorer, star note, and year group.
    /// I say "all of them besides", but there's only 7 `TriggerType`s and we can only pre-schedule 3 of them.
    /// For the record, the 3 we can pre-schedule are rhythm icons, combo, and time.
    ///
    /// # Panics
    /// Neverrrrr
    #[expect(clippy::arithmetic_side_effects, reason = "`bounded by threshold")]
    #[expect(clippy::expect_used, reason = "9 lanes fit u16 no problemo so we gucci.")]
    fn build(count: u32, unit: u32, skills: &[Skill; LANES], clock: TriggerType) -> Self {
        const { assert!(LANES <= u16::BITS as usize, "u16") };

        let mut dense = vec![0_u16; count as usize + 1];
        for (lane, skill) in skills.iter().enumerate() {
            if skill.trigger != clock || skill.threshold == 0 {
                continue;
            }
            let bit = 1_u16.checked_shl(u32::try_from(lane).expect("9 lanes")).expect("u16");
            let step = u32::from(skill.threshold);
            let mut at = step;
            while at <= count {
                if let Some(mask) = dense.get_mut(at as usize) {
                    *mask |= bit;
                }
                at += step;
            }
        }

        let mut out = Self { at: Vec::new(), lanes: Vec::new() };
        for (at, &mask) in dense.iter().enumerate() {
            if mask != 0 {
                out.at.push(u32::try_from(at).unwrap_or(u32::MAX).saturating_mul(unit));
                out.lanes.push(mask);
            }
        }
        out.at.push(u32::MAX);
        out.lanes.push(0);
        out
    }

    pub(super) fn columns(&self) -> (&[u32], &[u16]) {
        (&self.at, &self.lanes)
    }

    #[must_use]
    pub fn rolls(&self) -> u32 {
        self.lanes.iter().map(|m| m.count_ones()).sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lanes.len().saturating_sub(1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// TODO: Star Notes belongs here too.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Triggers {
    icons: Schedule,
    combo: Schedule,
    time: Schedule,
}

impl Triggers {
    #[must_use]
    pub fn build(queue: &Queue, scoring: &Scoring) -> Self {
        let skills = &scoring.skills;
        let seconds = queue.last_judgement().seconds();
        Self {
            icons: Schedule::build(queue.icons(), 1, skills, TriggerType::RhythmIcons),
            combo: Schedule::build(queue.notes(), 1, skills, TriggerType::Combo),
            time: Schedule::build(seconds, Time::SECOND, skills, TriggerType::Time),
        }
    }

    #[must_use]
    pub const fn icons(&self) -> &Schedule {
        &self.icons
    }

    #[must_use]
    pub const fn combo(&self) -> &Schedule {
        &self.combo
    }

    #[must_use]
    pub const fn time(&self) -> &Schedule {
        &self.time
    }

    #[must_use]
    pub fn rolls(&self) -> u32 {
        [&self.icons, &self.combo, &self.time].iter().map(|s| s.rolls()).sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        [&self.icons, &self.combo, &self.time].iter().map(|s| s.len()).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Ties happen right to left. Actually there's a coinflip mechanic involving Amplify, but we'll worry about that once I get around to Amplify.
#[inline]
pub(super) fn fire(mask: u16, scoring: &Scoring, rng: &mut Rng, out: &mut Outcome) {
    let mut bits = mask;
    while let Some(lane) = bits.highest_one() {
        bits ^= bits.isolate_highest_one();
        let Some(skill) = scoring.skills.get(lane as usize) else {
            continue;
        };
        if rng.roll(skill.chance) {
            out.activations = out.activations.saturating_add(1);
            pay(skill.effect, skill.value, out);
            continue;
        }

        let Some(accessory) = scoring.accessories.get(lane as usize) else {
            continue;
        };
        if accessory.is_inert() || !rng.roll(accessory.chance) {
            continue;
        }
        out.accessory_activations = out.accessory_activations.saturating_add(1);
        pay(accessory.effect, accessory.value, out);
    }
}

/// TODO: every other effect.
#[inline]
fn pay(effect: SkillEffect, value: Centi, out: &mut Outcome) {
    if effect == SkillEffect::Scorer {
        out.skill = out.skill.saturating_add(u64::from(value));
    }
}

#[cfg(test)]
mod tests {
    use super::{Skill, Triggers};
    use crate::play::tests::{
        ALWAYS, arena, at, config, fifty_one, only, play, scorer, seeded, tap, worn,
    };
    use crate::play::{Queue, Scoring};
    use crate::play_config::{NoteSpeed, PlayConfig};
    use crate::rng::Rng;
    use crate::scalar::{RATE_UNIT, Rate};
    use crate::skill::TriggerType;
    use crate::song::{Event, EventKind, Time};

    #[expect(clippy::integer_division, reason = "a coin flip")]
    const FLIP: Rate = RATE_UNIT / 2;

    const CLOCKS: [(TriggerType, u16); 3] = [
        (TriggerType::RhythmIcons, 51),
        (TriggerType::Combo, 51),
        (TriggerType::Time, 7),
    ];

    fn clocked(trigger: TriggerType, threshold: u16, chance: Rate, value: u32) -> Skill {
        Skill { trigger, ..scorer(threshold, chance, value) }
    }

    /// First `n` skill draws of a seed.
    fn draws(seed: u64, n: usize) -> Vec<u64> {
        let mut rng = Rng::new(seed, Rng::SKILLS);
        core::iter::repeat_with(|| u64::from(rng.roll(FLIP))).take(n).collect()
    }

    /// A manually and meticulously selected seed where the first two rolls differ.
    fn split_seed() -> u64 {
        (0..64_u64)
            .find(|&s| {
                let mut rng = Rng::new(s, Rng::SKILLS);
                rng.roll(FLIP) != rng.roll(FLIP)
            })
            .expect("a seed where the first two draws differ")
    }

    #[test]
    fn check_scorer_and_charm() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // This card is 26 icons at a 26% chance for 1120 points.
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[2] = scorer(26, ALWAYS, 1120);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 1);
        assert_eq!(out.skill, 1120);
        assert_eq!(out.tap, 1000 * 50 + 1100, "tap unchanged");
        assert_eq!(out.score(), out.tap + out.skill);

        scoring.skills[2] = scorer(26, ALWAYS, 2800);
        assert_eq!(play(&queue, &scoring).skill, 2800);

        scoring.skills[2] = scorer(26, 0, 1120);
        let cold = play(&queue, &scoring);
        assert_eq!((cold.activations, cold.skill), (0, 0));
        assert_eq!(Triggers::build(&queue, &scoring).rolls(), 1);
    }

    #[test]
    fn check_tap_score_before_skills() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        let bare = play(&queue, &scoring).tap;

        scoring.skills[5] = scorer(1, ALWAYS, 1_000_000);
        scoring.skills[6] = clocked(TriggerType::Combo, 1, ALWAYS, 1_000_000);
        scoring.skills[7] = clocked(TriggerType::Time, 1, ALWAYS, 1_000_000);
        let loud = play(&queue, &scoring);
        assert_eq!(loud.activations, 51 + 51 + 7, "every icon, combo and second");
        assert_eq!(loud.tap, bare);
    }

    #[test]
    fn check_accessory_trigger() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        for (clock, ticks) in CLOCKS {
            let n = u32::from(ticks);
            let mut scoring = Scoring::flat(0);
            scoring.skills[2] = clocked(clock, 1, ALWAYS, 1120);
            scoring.accessories[2] = worn(ALWAYS, 9000);
            let hits = play(&queue, &scoring);
            assert_eq!((hits.activations, hits.accessory_activations), (n, 0), "{clock:?}");
            assert_eq!(
                hits.skill,
                u64::from(n) * 1120,
                "{clock:?}: acc never got a chance to activate"
            );

            scoring.skills[2] = clocked(clock, 1, 0, 1120);
            let misses = play(&queue, &scoring);
            assert_eq!((misses.activations, misses.accessory_activations), (0, n), "{clock:?}");
            assert_eq!(misses.skill, u64::from(n) * 9000, "{clock:?}: acc can now activate");
        }
    }

    #[test]
    fn check_no_card_skill_means_no_acc_skill() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[3] = Skill::NONE;
        scoring.accessories[3] = worn(ALWAYS, 9000);
        assert_eq!(Triggers::build(&queue, &scoring).rolls(), 0, "nothing to fail");

        let out = play(&queue, &scoring);
        assert_eq!((out.accessory_activations, out.skill), (0, 0));
    }

    #[test]
    fn check_skills_resolve_right_to_left() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let seed = split_seed();

        let mut rng = Rng::new(seed, Rng::SKILLS);
        let high = rng.roll(FLIP); // lane 8 first
        let low = rng.roll(FLIP);
        let want = u64::from(high) * 1000 + u64::from(low);

        for (clock, once) in CLOCKS {
            let mut scoring = Scoring::flat(0);
            scoring.skills[7] = clocked(clock, once, FLIP, 1000);
            scoring.skills[1] = clocked(clock, once, FLIP, 1);
            assert_eq!(
                Triggers::build(&queue, &scoring).len(),
                1,
                "{clock:?}: one count, two lanes"
            );
            assert_eq!(seeded(&queue, &scoring, seed).skill, want, "{clock:?}, seed {seed}");
        }
    }

    #[test]
    fn check_combo_counts_taps_and_releases() {
        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            tap(2500, 1),
            (Time(3000), Event::new(0, EventKind::HoldRelease, false, false)),
            tap(3500, 2),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.notes(), 3, "hold start doesn't increment combo");

        let mut scoring = Scoring::flat(0);
        scoring.skills[4] = clocked(TriggerType::Combo, 1, ALWAYS, 7);
        let every = play(&queue, &scoring);
        assert_eq!((every.activations, every.skill), (3, 21));

        for (threshold, rolls) in [(2, 1), (3, 1), (4, 0)] {
            scoring.skills[4] = clocked(TriggerType::Combo, threshold, ALWAYS, 7);
            let triggers = Triggers::build(&queue, &scoring);
            assert_eq!(triggers.combo().rolls(), rolls, "every {threshold}");
            assert!(triggers.icons().is_empty(), "every {threshold}");
            assert_eq!(play(&queue, &scoring).activations, rolls, "every {threshold}");
        }
    }

    #[test]
    fn check_timet() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.last_judgement(), Time(7000));
        assert_eq!(queue.times()[0], Time(2000), "the first note");

        let every: Vec<u32> = (1..=7).map(|s| s * 1000).collect();
        let cases: [(u16, &[u32]); 4] =
            [(1, &every), (2, &[2000, 4000, 6000]), (7, &[7000]), (8, &[])];
        for (threshold, want) in cases {
            let mut scoring = Scoring::flat(0);
            scoring.skills[0] = clocked(TriggerType::Time, threshold, ALWAYS, 1);
            let triggers = Triggers::build(&queue, &scoring);
            let (fires, _) = triggers.time().columns();
            assert_eq!(fires, [want, &[u32::MAX]].concat(), "every {threshold}s in ms");
            assert!(triggers.icons().is_empty() && triggers.combo().is_empty());

            let n = u32::try_from(want.len()).expect("a hearty handful");
            assert_eq!(play(&queue, &scoring).activations, n, "every {threshold}s");
            let slow = PlayConfig { speed: NoteSpeed::SLOWEST, ..config() };
            assert_eq!(at(&queue, &scoring, &slow).activations, n, "a time event");
        }
    }

    /// Time > tap > note spawn then the spawn, but reversed for the lanes
    #[test]
    fn check_time_tiebreakers() {
        // Spawns at 1400, 2400 and 3000 at the fastest speed.
        let stream = [tap(2000, 0), tap(3000, 1), tap(3600, 2)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[0] = clocked(TriggerType::Time, 3, FLIP, 100);
        scoring.skills[4] = clocked(TriggerType::Combo, 2, FLIP, 10);
        scoring.skills[8] = clocked(TriggerType::RhythmIcons, 3, FLIP, 1);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!(triggers.time().columns().0, [3000, u32::MAX]);

        let mut discriminated = false;
        for seed in 0..32 {
            let d = draws(seed, 3);
            let want = d[0] * 100 + d[1] * 10 + d[2];
            assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}: {d:?}");
            discriminated |= want != d[2] * 100 + d[1] * 10 + d[0];
        }
        assert!(discriminated, "couldn't identify order");
    }

    /// Really just making sure the event queue is populated
    #[test]
    fn check_time_scheudling() {
        let slow = |seed| PlayConfig { speed: NoteSpeed::SLOWEST, seed, ..config() };
        let offset = NoteSpeed::SLOWEST.offset();

        for (second, icon_first) in [(4000, true), (5000, false)] {
            let stream = [tap(2000, 0), tap(second, 1)];
            let arena = arena(&stream);
            let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
            assert_eq!(Time(second).spawn(offset) < Time(3000), icon_first);

            let mut scoring = Scoring::flat(0);
            scoring.skills[0] = clocked(TriggerType::RhythmIcons, 2, FLIP, 10);
            scoring.skills[8] = clocked(TriggerType::Time, 3, FLIP, 1);
            assert_eq!(Triggers::build(&queue, &scoring).time().columns().0, [3000, u32::MAX]);

            for seed in 0..32 {
                let d = draws(seed, 2);
                let want = if icon_first {
                    d[0] * 10 + d[1]
                } else {
                    d[1] * 10 + d[0]
                };
                assert_eq!(at(&queue, &scoring, &slow(seed)).skill, want, "seed {seed}");
            }
        }
    }
}
