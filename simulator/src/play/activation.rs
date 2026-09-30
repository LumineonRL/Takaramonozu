//! Skill activation trigger conditions e.g. rhythm icons, combo, etc.

use super::{Outcome, Queue, Scoring};
use crate::rng::Rng;
use crate::scalar::{Centi, Rate};
use crate::skill::{SkillEffect, TriggerType};
use crate::song::{LANES, Time};

/// Won't work for all skills, but there are some that we can pre-computed before a trial starts and
/// reused if the same team is being evaluated many times.
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

// Can pre-add to the queue certain activations like rhythm icons, combo, and time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Schedule {
    at: Vec<u32>,
    lanes: Vec<u16>,
}

impl Schedule {
    /// Pre-schedules each lane's skill activations if their trigger type allows for it.
    /// We can trivially pre-schedule rhythm icons, combo, and time.
    /// We can mostly pre-schedule perfects, and star notes.
    /// We can not pre-schedule year-group or score-based scorers.
    ///
    /// And what I mean by "mostly pre-schedule"  is we can pre-roll and have everything ready to go,
    /// but we need to wait for the perfects to actually occur before triggering, so there's still some
    /// branchy logic.
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Triggers {
    icons: Schedule,
    combo: Schedule,
    perfects: Schedule,
    time: Schedule,
    stars: Schedule,
}

impl Triggers {
    #[must_use]
    pub fn build(queue: &Queue, scoring: &Scoring) -> Self {
        let skills = &scoring.skills;
        let seconds = queue.last_judgement().seconds();
        Self {
            icons: Schedule::build(queue.icons(), 1, skills, TriggerType::RhythmIcons),
            combo: Schedule::build(queue.notes(), 1, skills, TriggerType::Combo),
            perfects: Schedule::build(queue.notes(), 1, skills, TriggerType::Perfects),
            time: Schedule::build(seconds, Time::SECOND, skills, TriggerType::Time),
            stars: Schedule::build(queue.stars(), 1, skills, TriggerType::StarNotes),
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
    pub const fn perfects(&self) -> &Schedule {
        &self.perfects
    }

    #[must_use]
    pub const fn time(&self) -> &Schedule {
        &self.time
    }

    #[must_use]
    pub const fn stars(&self) -> &Schedule {
        &self.stars
    }

    #[must_use]
    pub fn rolls(&self) -> u32 {
        self.all().iter().map(|s| s.rolls()).sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.all().iter().map(|s| s.len()).sum()
    }

    const fn all(&self) -> [&Schedule; 5] {
        [&self.icons, &self.combo, &self.perfects, &self.time, &self.stars]
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
    use crate::play::{Judgements, Queue, Scoring, run};
    use crate::play_config::{NoteSpeed, PlayConfig};
    use crate::rng::Rng;
    use crate::scalar::{RATE_UNIT, Rate};
    use crate::skill::TriggerType;
    use crate::song::{Event, EventKind, Time};

    #[expect(clippy::integer_division, reason = "a coin flip")]
    const FLIP: Rate = RATE_UNIT / 2;

    /// Note that for an all-perfect, Perfect is essentially equivalent to Combo.
    const CLOCKS: [(TriggerType, u16); 4] = [
        (TriggerType::RhythmIcons, 51),
        (TriggerType::Combo, 51),
        (TriggerType::Perfects, 51),
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

        scoring.skills[4] = clocked(TriggerType::Perfects, 1, ALWAYS, 1_000_000);
        scoring.skills[5] = scorer(1, ALWAYS, 1_000_000);
        scoring.skills[6] = clocked(TriggerType::Combo, 1, ALWAYS, 1_000_000);
        scoring.skills[7] = clocked(TriggerType::Time, 1, ALWAYS, 1_000_000);
        let loud = play(&queue, &scoring);
        assert_eq!(loud.activations, 51 + 51 + 51 + 7, "every icon, combo, perfect and second");
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

    fn sloppy(accuracy: Rate, seed: u64) -> PlayConfig {
        PlayConfig { accuracy, seed, ..config() }
    }

    #[expect(clippy::arithmetic_side_effects, reason = "test helper")]
    fn perfect_notes(queue: &Queue, judged: &Judgements) -> u32 {
        let mut heads = [true; 9];
        let mut count = 0;
        for (i, event) in queue.events().iter().enumerate() {
            let lane = event.lane() as usize;
            match event.kind() {
                EventKind::HoldStart => heads[lane] = judged.at(i),
                EventKind::HoldRelease => count += u32::from(heads[lane] && judged.at(i)),
                EventKind::Tap => count += u32::from(judged.at(i)),
                EventKind::End => {}
            }
        }
        count
    }

    #[test]
    fn check_perfects_is_combo_at_all_perfect() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut combo = Scoring::flat(80_000);
        combo.skills[3] = clocked(TriggerType::Combo, 7, FLIP, 300);
        combo.skills[6] = clocked(TriggerType::Combo, 5, FLIP, 20);
        let mut perfects = combo;
        perfects.skills[6].trigger = TriggerType::Perfects;

        let (a, b) = (Triggers::build(&queue, &combo), Triggers::build(&queue, &perfects));
        assert_eq!(b.perfects().columns().0, [5, 10, 15, 20, 25, 30, 35, 40, 45, 50, u32::MAX]);
        assert_eq!(a.rolls(), b.rolls());
        assert_eq!(a.len() + 1, b.len());

        for seed in 0..32 {
            assert_eq!(
                seeded(&queue, &combo, seed),
                seeded(&queue, &perfects, seed),
                "seed {seed}"
            );
        }
    }

    /// Holds are perfect if and only if the start and end were both perfect.
    #[test]
    #[expect(clippy::integer_division, reason = "firings of a threshold")]
    fn check_perfects_counts_perfect_notes() {
        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            tap(2300, 1),
            (Time(2600), Event::new(0, EventKind::HoldRelease, false, false)),
            tap(2900, 2),
            (Time(3200), Event::new(3, EventKind::HoldStart, false, false)),
            tap(3500, 4),
            (Time(3800), Event::new(3, EventKind::HoldRelease, false, false)),
            tap(4100, 5),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        for threshold in [1, 2, 3] {
            let mut scoring = Scoring::flat(0);
            scoring.skills[8] = clocked(TriggerType::Perfects, threshold, ALWAYS, 1);
            let triggers = Triggers::build(&queue, &scoring);
            assert_eq!(triggers.perfects().rolls(), 6 / u32::from(threshold), "ceiling");

            let mut seen = [false; 7];
            for seed in 0..64 {
                let config = sloppy(FLIP, seed);
                let judged = Judgements::build(&queue, &config);
                let out = run(&queue, &config, &scoring, &triggers, &judged);
                let want = perfect_notes(&queue, &judged);
                assert_eq!(out.perfects, want, "seed {seed}");
                assert_eq!(out.activations, want / u32::from(threshold), "seed {seed}");
                seen[want as usize] = true;
            }
            assert!(seen.iter().filter(|&&s| s).count() > 4, "{seen:?}");
        }
    }

    /// It would be bad if greats incremented the perfect counter.
    #[test]
    fn check_perfect_skills_roll_only_on_perfects() {
        // Spawns at 1400, 1700, 2000 and 2300 at AR10.
        let stream = [tap(2000, 0), tap(2300, 1), tap(2600, 2), tap(2900, 3)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[0] = clocked(TriggerType::RhythmIcons, 4, FLIP, 1);
        scoring.skills[8] = clocked(TriggerType::Perfects, 2, FLIP, 10);
        let triggers = Triggers::build(&queue, &scoring);

        let mut orders = [false; 2];
        for seed in 0..64 {
            let config = sloppy(FLIP, seed);
            let judged = Judgements::build(&queue, &config);

            // (ms, tap before spawn, payout)
            let mut firings = vec![(2300, 1, 1)];
            let mut count = 0;
            for (i, ms) in [2000, 2300, 2600, 2900].into_iter().enumerate() {
                count += u32::from(judged.at(i));
                if judged.at(i) && count % 2 == 0 {
                    firings.push((ms, 0, 10));
                }
            }
            firings.sort_unstable();
            let d = draws(seed, firings.len());
            let want: u64 = firings.iter().zip(&d).map(|(f, hit)| f.2 * hit).sum();

            let out = run(&queue, &config, &scoring, &triggers, &judged);
            assert_eq!(out.skill, want, "seed {seed}: {firings:?}");
            orders[usize::from(firings[0].2 == 1)] = true;
        }
        assert_eq!(orders, [true; 2], "the icon never beat, or never trailed, the perfects");
    }

    #[test]
    fn check_a_combo_and_perfect_skills_trigger_together_best_buds() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let seed = split_seed();
        let d = draws(seed, 2);
        let want = d[0] * 1000 + d[1];

        for (high, low) in [
            (TriggerType::Perfects, TriggerType::Combo),
            (TriggerType::Combo, TriggerType::Perfects),
        ] {
            let mut scoring = Scoring::flat(0);
            scoring.skills[7] = clocked(high, 51, FLIP, 1000);
            scoring.skills[1] = clocked(low, 51, FLIP, 1);
            assert_eq!(seeded(&queue, &scoring, seed).skill, want, "{high:?} over {low:?}");
        }
    }

    #[test]
    fn check_no_perfects_no_draws() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(80_000);
        scoring.skills[2] = scorer(4, FLIP, 100);
        scoring.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
        let mut deaf = scoring;
        deaf.skills[8] = clocked(TriggerType::Perfects, 1, FLIP, 1);
        deaf.accessories[8] = worn(ALWAYS, 1);
        assert_eq!(Triggers::build(&queue, &deaf).perfects().rolls(), 51, "scheduled all the same");

        for seed in 0..32 {
            let config = sloppy(0, seed);
            let out = at(&queue, &deaf, &config);
            assert_eq!(out.perfects, 0, "seed {seed}");
            assert_eq!(out, at(&queue, &scoring, &config), "seed {seed}");
        }
    }

    fn star(ms: u32, lane: u8) -> (Time, Event) {
        (Time(ms), Event::new(lane, EventKind::Tap, true, false))
    }

    // TODO: Come up with a better name LMAO.
    fn starry(chance: Rate, value: u32) -> Skill {
        clocked(TriggerType::StarNotes, 1, chance, value)
    }

    #[expect(clippy::arithmetic_side_effects, reason = "test helper")]
    fn perfect_stars(queue: &Queue, judged: &Judgements) -> u32 {
        let mut heads = [true; 9];
        let mut count = 0;
        for (i, event) in queue.events().iter().enumerate() {
            let lane = event.lane() as usize;
            match event.kind() {
                EventKind::HoldStart => heads[lane] = judged.at(i),
                EventKind::HoldRelease if event.is_star() => {
                    count += u32::from(heads[lane] && judged.at(i));
                }
                EventKind::Tap if event.is_star() => count += u32::from(judged.at(i)),
                EventKind::Tap | EventKind::HoldRelease | EventKind::End => {}
            }
        }
        count
    }

    /// For the record, star notes are only regular tap notes, never a swing, never a hold. Although I suppose
    /// nothing would stop a custom charter from doing that.
    #[test]
    fn check_star_notes_roll_on_perfect_stars() {
        let stream = [
            star(2000, 0),
            tap(2300, 1),
            star(2600, 2),
            (Time(2900), Event::new(3, EventKind::HoldStart, true, false)),
            star(3200, 4),
            (Time(3500), Event::new(3, EventKind::HoldRelease, true, false)),
            tap(3800, 5),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.stars(), 4, "three taps and a hold");

        let mut scoring = Scoring::flat(0);
        scoring.skills[6] = starry(ALWAYS, 1);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!(triggers.stars().rolls(), 4, "every star during an all-PERFECT");
        assert_eq!(triggers.rolls(), 4, "no others");
        assert_eq!(play(&queue, &scoring).activations, 4);

        let mut seen = [false; 5];
        for seed in 0..64 {
            let config = sloppy(FLIP, seed);
            let judged = Judgements::build(&queue, &config);
            let want = perfect_stars(&queue, &judged);
            let out = run(&queue, &config, &scoring, &triggers, &judged);
            assert_eq!(out.activations, want, "seed {seed}");
            assert_eq!(out.skill, u64::from(want), "seed {seed}");
            seen[want as usize] = true;
        }
        assert!(seen.iter().filter(|&&s| s).count() > 3, "{seen:?}");
    }

    /// A bit of a modeling assumption here. Say you have two simultaneous star notes on the same millisecond and
    /// get a perfect on both of them. This will process them tap -> skill -> tap -> skill.
    #[test]
    fn check_simultaneous_stars_roll_after_their_own_tap() {
        let twins = arena(&[star(2000, 0), star(2000, 1)]);
        let both = Queue::build(&twins, &only(&twins)).expect("terminated");
        let mut lone = Scoring::flat(0);
        lone.skills[4] = starry(ALWAYS, 1);
        assert_eq!(play(&both, &lone).activations, 2);

        let mixed = arena(&[star(2000, 0), tap(2000, 1)]);
        let queue = Queue::build(&mixed, &only(&mixed)).expect("terminated");
        let mut scoring = Scoring::flat(0);
        scoring.skills[1] = starry(FLIP, 1);
        scoring.skills[7] = clocked(TriggerType::Combo, 2, FLIP, 1000);

        let seed = split_seed();
        let d = draws(seed, 2);
        assert_ne!(d[0] + d[1] * 1000, d[0] * 1000 + d[1]);
        assert_eq!(seeded(&queue, &scoring, seed).skill, d[0] + d[1] * 1000, "seed {seed}");
    }

    #[test]
    fn check_a_star_failure_rolls_acc() {
        let stream = [star(2000, 0), tap(2300, 1), star(2600, 2), star(2900, 3)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[0] = starry(0, 1);
        scoring.accessories[0] = worn(ALWAYS, 9000);
        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 3));
        assert_eq!(out.skill, 3 * 9000);
    }

    #[test]
    fn check_no_star_perfects_no_rolls() {
        let stream: Vec<_> = (0_u8..27)
            .map(|i| {
                let note = if i % 3 == 0 { star } else { tap };
                note(2000 + u32::from(i) * 100, i % 9)
            })
            .collect();
        let starred = arena(&stream);
        let plain = fifty_one();

        for arena in [&starred, &plain] {
            let queue = Queue::build(arena, &only(arena)).expect("terminated");
            let mut scoring = Scoring::flat(80_000);
            scoring.skills[2] = scorer(4, FLIP, 100);
            scoring.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
            let mut deaf = scoring;
            deaf.skills[8] = starry(FLIP, 1);
            deaf.accessories[8] = worn(ALWAYS, 1);
            assert_eq!(Triggers::build(&queue, &deaf).stars().rolls(), queue.stars());

            let accuracy = if queue.stars() == 0 { FLIP } else { 0 };
            for seed in 0..32 {
                let config = sloppy(accuracy, seed);
                assert_eq!(
                    at(&queue, &deaf, &config),
                    at(&queue, &scoring, &config),
                    "seed {seed}"
                );
            }
        }
    }
}
