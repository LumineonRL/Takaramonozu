//! Appeal Boost / Param Boost whatever you want to call it.
//! And part of Sync because Sync reuses a lot of this. Fundamentally they're very similar
//! in that they both modify PPN mid-play.

mod sync;

use super::{Lasting, NEVER};
use crate::play::Scoring;
use crate::play::queue::Queue;
use crate::play::tap::{Worth, overheal_score_val, worth_score};
use crate::scalar::Ms;
use crate::song::{LANES, Time};
use sync::Copies;

/// Appeal Boost and Sync.
/// A lane starts by broadcasting its stat. 99% of the time this is just its regular stat, but can also
/// be its sync'd stat.
/// An active param boost gets added onto that.
pub(in crate::play) struct Param<E> {
    lasting: E,
    /// Sorry for the confusing names. This one represents when a Param Boost expires.
    until: Time,
    /// and this is the earliest end of any param boost OR sync.
    ending: Time,
    wake: Time,
    trick: u32,
    /// The active boost's points, the lanes it applies to its percententage.
    boost: u32,
    reach: u16,
    percent: u16,
    copies: Copies,
    targets: [[u16; 2]; LANES],
    percents: [[u16; 2]; LANES],
    /// The team strength, then with the Trick. Yeah Trick SIS is completely separate.
    strengths: [u32; 2],
    /// `strengths` * heart multiplier.
    hearts: [u64; 2],
    /// `strengths` times every judgement. For a Total Trick team.
    /// isn't it funny how much special handling I have to give to such a simple
    /// and useless skill like Total Trick?
    table: Worth,
}

impl<'q, E> Lasting<'q> for Param<E>
where
    E: Lasting<'q>,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = E::HEARTS;
    const PARAM: bool = true;

    forward! {
        converted, locked, lock, trick, occupy,
        overheal_strength, heart_multiplier,
    }

    #[inline]
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self {
        let mut param = Self {
            lasting: E::new(last, queue, scoring),
            until: Time(0),
            ending: NEVER,
            wake: NEVER,
            trick: scoring.trick,
            boost: 0,
            reach: 0,
            percent: 0,
            copies: Copies::new(scoring),
            targets: scoring.targets,
            percents: scoring.boosts,
            strengths: [0; 2],
            hearts: [0; 2],
            table: Worth::default(),
        };
        param.refresh();
        param
    }

    #[inline(always)]
    fn admit(&mut self, mask: u16, now: Time) -> u16 {
        let admitted = self.lasting.admit(mask, now);
        self.wake = self.wake.min(self.lasting.next());
        admitted
    }

    #[inline(always)]
    fn next(&self) -> Time {
        self.wake
    }

    #[inline(always)]
    fn release(&mut self, at: Time) -> u16 {
        let due = self.lasting.release(at);
        self.wake = self.lasting.next().min(self.ending);
        due
    }

    #[inline(always)]
    fn expire(&mut self, now: Time, _perfects: u32) {
        if now >= self.ending {
            self.lapse(now);
        }
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "bool")]
    fn strength(&self, locked: bool) -> u32 {
        self.strengths[usize::from(locked)]
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "bool")]
    fn hearted_strength(&self, locked: bool) -> u64 {
        self.hearts[usize::from(locked)]
    }

    #[inline(always)]
    fn tap(&self, now: Time, head: bool, tail: bool, combo: u16, lane: u16, note: u16) -> u64 {
        let worth = self.worth(now, head, tail);
        if E::HEARTS {
            overheal_score_val(worth, combo, lane, note, self.lasting.heart_multiplier())
        } else {
            worth_score(worth, combo, lane, note)
        }
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "bool")]
    fn worth(&self, now: Time, head: bool, tail: bool) -> u64 {
        let row = self.table.row(self.lasting.converted(now), self.lasting.locked(now));
        row[usize::from(head)][usize::from(tail)]
    }

    #[inline(always)]
    fn heal(&mut self, hp: u8) {
        self.lasting.heal(hp);
        if E::HEARTS {
            let multiplier = self.lasting.heart_multiplier();
            self.hearts = self.strengths.map(|s| hearted(s, multiplier));
        }
    }

    /// Param Boost rolling while already active doesn't do anything.
    /// well it should be a valid target for encore so it does do *something*
    /// but I really just mean that it doesn't extend or overwrite the current duration.
    #[inline(never)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    fn param(&mut self, lane: u32, worn: bool, now: Time, duration: Ms, value: u32) {
        if now < self.until {
            return;
        }
        self.until = self.lasting.occupy(lane, now, duration);
        self.ending = self.ending.min(self.until);
        self.wake = self.wake.min(self.until);
        self.boost = value;
        let half = usize::from(worn);
        self.reach = self.targets[lane as usize][half];
        self.percent = self.percents[lane as usize][half];
        self.refresh();
    }

    #[inline(never)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    fn sync(&mut self, lane: u32, worn: bool, now: Time, duration: Ms) {
        let until = self.lasting.occupy(lane, now, duration);
        let targets = self.targets[lane as usize][usize::from(worn)];
        if !self.copies.land(lane, targets, until, (self.reach, self.percent)) {
            return;
        }
        self.ending = self.ending.min(until);
        self.wake = self.wake.min(until);
        self.refresh();
    }

    #[inline(always)]
    fn seed(&mut self, seed: u64) {
        self.copies.seed(seed);
    }
}

impl<'q, E> Param<E>
where
    E: Lasting<'q>,
{
    fn refresh(&mut self) {
        let shown = self.copies.shown().saturating_add(self.boost);
        self.strengths = [shown, shown.saturating_add(self.trick)];
        if E::HEARTS {
            let multiplier = self.lasting.heart_multiplier();
            self.hearts = self.strengths.map(|s| hearted(s, multiplier));
        }
        if E::TRICKS {
            self.table = Worth::new(shown, self.trick);
        }
    }

    /// Ends an active boost and Sync whose window is over.
    #[inline(never)]
    fn lapse(&mut self, now: Time) {
        let mut ending = NEVER;
        if self.until <= now {
            (self.boost, self.reach) = (0, 0);
        } else {
            ending = self.until;
        }
        let ending = ending.min(self.copies.lapse(now));
        self.ending = ending;
        self.refresh();
        self.wake = self.lasting.next().min(ending);
    }
}

fn hearted(strength: u32, multiplier: u16) -> u64 {
    u64::from(strength).saturating_mul(u64::from(multiplier))
}

#[cfg(test)]
mod tests {
    use crate::play::activation::{AccessorySkill, Triggers};
    use crate::play::effect::tests::{
        BOOST, FLIP, ONE_HEART_EVERY_TEN, TEAM, clocked, fever, healer, lock, overhealing, param,
        psu, trick,
    };
    use crate::play::queue::Queue;

    use crate::play::tests::{ALWAYS, arena, at, fifty_one, only, play, scorer, sloppy, worn};
    use crate::play::{Outcome, Scoring};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{Event, EventKind, Time};

    /// Every note of the 51-note map at `TEAM + BOOST`.
    const ALL_BOOSTED: u64 = 50 * 1500 + 1650;

    #[test]
    fn check_param() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        // Before first note, lasts until after final note.
        param(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, BOOST, 60_000);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.param(), triggers.lasting()), (1 << 8, 1 << 8));

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill, out.tap), (1, 0, ALL_BOOSTED));
    }

    #[test]
    fn check_param_from_next_note() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut combo = Scoring::flat(TEAM);
        param(&mut combo, 8, TriggerType::Combo, 10, ALWAYS, BOOST, 60_000);
        let hit = play(&queue, &combo);
        assert_eq!((hit.activations, hit.tap), (1, 10 * 1000 + 40 * 1500 + 1650));

        // Icon 10 spawns on note 4's tap. Tap processed first.
        let mut icon = Scoring::flat(TEAM);
        param(&mut icon, 8, TriggerType::RhythmIcons, 10, ALWAYS, BOOST, 60_000);
        let spawn = play(&queue, &icon);
        assert_eq!((spawn.activations, spawn.tap), (1, 4 * 1000 + 46 * 1500 + 1650));
    }

    #[test]
    fn check_param_duration() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        // [3000, 3500) and [6000, 6500)
        //  notes 11 to 15 and 41 to 45.
        param(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, BOOST, 500);
        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.tap), (2, 40 * 1000 + 10 * 1500 + 1100));
    }

    #[test]
    fn check_param_doesnt_stack() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        param(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, BOOST, 60_000);
        param(&mut scoring, 7, TriggerType::RhythmIcons, 1, ALWAYS, 2 * BOOST, 60_000);
        let mut idle = scoring;
        idle.skills[7] = scorer(1, ALWAYS, 0);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.tap), (1 + 51, ALL_BOOSTED));
        assert_eq!(out, play(&queue, &idle));
    }

    #[test]
    fn check_param_lane_blocking() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        param(&mut scoring, 4, TriggerType::Perfects, 5, ALWAYS, 0, 550);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 9);

        let mut paying_nothing = scoring;
        psu(&mut paying_nothing, 4, TriggerType::Perfects, 5, ALWAYS, 0, 550);
        assert_eq!(play(&queue, &paying_nothing), out);
    }

    #[test]
    fn check_param_acc() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[6] = clocked(TriggerType::Time, 1, 0, 1);
        scoring.accessories[6] =
            AccessorySkill { effect: SkillEffect::AppealBoost, ..worn(ALWAYS, BOOST) };
        scoring.durations[6][1] = 60_000;
        assert_eq!(Triggers::build(&queue, &scoring).param(), 1 << 6);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 1));
        assert_eq!((out.skill, out.tap), (0, ALL_BOOSTED));
    }

    /// Gets 1.08x true perfect
    /// Does not get Trick SIS.
    #[test]
    fn check_param_under_a_lock() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.trick = 8_000;
        param(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, BOOST, 60_000);
        lock(&mut scoring, 4, TriggerType::Time, 1, ALWAYS, 60_000);
        // 128_000 a note = 1728 at 1.08x -> 1600 converted.
        assert_eq!(play(&queue, &scoring).tap, 50 * 1728 + 1900);
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).tap, 50 * 1600 + 1760);

        let mut unboosted = scoring;
        unboosted.skills[8].value = 0;
        assert_eq!(play(&queue, &unboosted).tap, ALL_LOCKED_TRICKED);
    }

    /// `(TEAM + 8000) * 1.08`.
    const ALL_LOCKED_TRICKED: u64 = 50 * 1188 + 1306;

    /// Total Trick should have no affect.
    #[test]
    fn check_param_under_a_total_trick() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.trick = 8_000;
        param(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, BOOST, 60_000);
        trick(&mut scoring, 4, TriggerType::Time, 1, ALWAYS, 60_000);
        assert_eq!(Triggers::build(&queue, &scoring).tricks(), 1 << 4);
        for accuracy in [ALWAYS, FLIP, 0] {
            assert_eq!(at(&queue, &scoring, &sloppy(accuracy, 0)).tap, 50 * 1600 + 1760);
        }

        let mut locked = scoring;
        lock(&mut locked, 3, TriggerType::Time, 1, ALWAYS, 60_000);
        assert_eq!(play(&queue, &locked).tap, 50 * 1728 + 1900);
    }

    #[test]
    #[expect(clippy::integer_division, reason = "it's a test.")]
    fn check_param_with_hearts() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        overhealing(&mut scoring, 30);
        healer(&mut scoring, 4, TriggerType::RhythmIcons, 10, ALWAYS, 30);
        param(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, BOOST, 60_000);
        // Every heal rigged to proc.
        assert_eq!(play(&queue, &scoring).tap, ONE_HEART_EVERY_TEN * 3 / 2);

        // Boost lands on note 25, after two hearts -> 1.2x and up.
        let mut late = scoring;
        param(&mut late, 8, TriggerType::Combo, 25, ALWAYS, BOOST, 60_000);
        let want: u64 = (1..=51_u64)
            .map(|note| {
                let heart = 10 + (note + 5) / 10;
                let strength = if note > 25 { 120 } else { 80 };
                let combo = if note > 50 { 110 } else { 100 };
                strength * heart * combo / 80
            })
            .sum();
        assert_eq!(play(&queue, &late).tap, want);
    }

    /// for hold releases when one param boost ends mid hold
    #[test]
    fn check_param_ends_through_releases() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut locked = Scoring::flat(TEAM);
        lock(&mut locked, 4, TriggerType::RhythmIcons, 1, ALWAYS, 250);
        let mut boosted = locked;
        param(&mut boosted, 1, TriggerType::Time, 3, ALWAYS, BOOST, 500);

        let (with, without) = (play(&queue, &boosted), play(&queue, &locked));
        assert!(without.activations > 20, "the lock re-lands");
        // Notes 11 to 15 and 41 to 45, each `BOOST * 1.08`.
        assert_eq!(with.tap, without.tap + 10 * 540);
        assert_eq!(
            Outcome {
                tap: with.tap,
                activations: without.activations + 2,
                ..without
            },
            with
        );
    }

    #[test]
    fn check_param_window_copies() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        // [3000, 3500) and [6000, 6500).
        let mut bare = Scoring::flat(TEAM);
        param(&mut bare, 1, TriggerType::Time, 3, ALWAYS, BOOST, 500);
        assert_eq!(play(&queue, &bare).tap, 40 * 1000 + 10 * 1500 + 1100);

        let mut copies = 0;
        for shape in 0..128_u32 {
            for lasting in 0..3 {
                let mut team = bare;
                if shape & 1 != 0 {
                    team.skills[3] = clocked(TriggerType::Score, 1500, 0, 50);
                }
                if shape & 2 != 0 {
                    team.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), 0, 70);
                    (team.members[0], team.members[2], team.members[5]) = (1, 1 << 2, 1 << 5);
                }
                if shape & 4 != 0 {
                    team.skills[8] = clocked(TriggerType::StarNotes, 1, 0, 5);
                }
                if shape & 8 != 0 {
                    overhealing(&mut team, 30);
                    healer(&mut team, 6, TriggerType::RhythmIcons, 4, 0, 12);
                }
                // acc can't roll with no card skill
                if shape & 16 != 0 {
                    team.skills[7] = clocked(TriggerType::RhythmIcons, u16::MAX, 0, 3);
                    team.accessories[7] =
                        AccessorySkill { effect: SkillEffect::ComboFever, ..worn(ALWAYS, 30) };
                    team.durations[7][1] = 400;
                }
                if shape & 32 != 0 {
                    team.skills[2] = scorer(u16::MAX, 0, 100);
                    team.accessories[2] =
                        AccessorySkill { effect: SkillEffect::PerfectScoreUp, ..worn(ALWAYS, 50) };
                    team.durations[2][1] = 500;
                }
                if shape & 64 != 0 {
                    team.skills[5] = scorer(u16::MAX, 0, 10);
                    team.accessories[5] =
                        AccessorySkill { effect: SkillEffect::SkillRateUp, ..worn(ALWAYS, 50) };
                    team.durations[5][1] = 300;
                }
                match lasting {
                    0 => {}
                    1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, 0, 600),
                    _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, 0, 600),
                }
                let triggers = Triggers::build(&queue, &team);
                assert_eq!(triggers.heals() != 0, shape & 8 != 0);
                assert_eq!(triggers.fever() != 0, shape & 16 != 0);
                assert_eq!(triggers.psu() != 0, shape & 32 != 0);
                assert_eq!(triggers.sru() != 0, shape & 64 != 0);
                assert_eq!(triggers.tricks() != 0, lasting == 2);

                for accuracy in [ALWAYS, 8000] {
                    for seed in 0..2 {
                        let config = sloppy(accuracy, seed);
                        assert_eq!(
                            at(&queue, &team, &config),
                            at(&queue, &bare, &config),
                            "shape {shape}, lasting {lasting}, seed {seed}"
                        );
                    }
                }
                copies += 1;
            }
        }
        assert_eq!(copies, 384);
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "I agree, clippy")]
    #[expect(clippy::cognitive_complexity, reason = "agreed, again and don't feel like rewriting.")]
    fn check_param_copies() {
        let stream: Vec<_> = (0..51_u32)
            .map(|i| {
                (Time(2000 + i * 100), Event::new((i % 9) as u8, EventKind::Tap, i % 3 == 0, false))
            })
            .collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut base = Scoring::flat(TEAM);
        base.trick = 9_999;
        base.skills[2] = scorer(4, FLIP, 100);
        base.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
        base.accessories[5] = worn(FLIP, 20);
        base.skills[6] = clocked(TriggerType::Time, 1, FLIP, 7);
        base.skills[7] = clocked(TriggerType::Perfects, 2, FLIP, 3);

        let mut copies = 0;
        for shape in 0..128_u32 {
            for lasting in 0..3 {
                let mut team = base;
                if shape & 1 != 0 {
                    team.skills[3] = clocked(TriggerType::Score, 1500, FLIP, 50);
                }
                if shape & 2 != 0 {
                    team.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), FLIP, 70);
                    (team.members[0], team.members[2], team.members[5]) = (1, 1 << 2, 1 << 5);
                }
                if shape & 4 != 0 {
                    team.skills[8] = clocked(TriggerType::StarNotes, 1, FLIP, 5);
                }
                if shape & 8 != 0 {
                    overhealing(&mut team, 30);
                    team.accessories[6] =
                        AccessorySkill { effect: SkillEffect::Healer, ..worn(FLIP, 0) };
                    team.heals[6][1] = 12;
                }
                if shape & 16 != 0 {
                    team.accessories[7] =
                        AccessorySkill { effect: SkillEffect::ComboFever, ..worn(FLIP, 30) };
                    team.durations[7][1] = 400;
                }
                if shape & 32 != 0 {
                    team.accessories[2] =
                        AccessorySkill { effect: SkillEffect::PerfectScoreUp, ..worn(FLIP, 50) };
                    team.durations[2][1] = 500;
                }
                if shape & 64 != 0 {
                    team.accessories[5] =
                        AccessorySkill { effect: SkillEffect::SkillRateUp, ..worn(FLIP, 50) };
                    team.durations[5][1] = 300;
                }
                match lasting {
                    0 => {}
                    1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                    _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                }

                let mut never = team;
                param(&mut never, 1, TriggerType::RhythmIcons, 3, 0, BOOST, 700);
                let mut cold = team;
                cold.skills[1] = scorer(3, 0, 0);
                let mut nothing = team;
                param(&mut nothing, 1, TriggerType::RhythmIcons, 3, FLIP, 0, 700);
                let mut feverish = team;
                fever(&mut feverish, 1, TriggerType::RhythmIcons, 3, FLIP, 0, 700);
                let mut boosted = team;
                param(&mut boosted, 1, TriggerType::Time, 1, ALWAYS, BOOST, 60_000);
                let mut strong = Scoring { team_strength: TEAM + BOOST, ..team };
                fever(&mut strong, 1, TriggerType::Time, 1, ALWAYS, 0, 60_000);

                let triggers = Triggers::build(&queue, &never);
                assert_eq!(triggers.param(), 1 << 1);
                assert_eq!(triggers.score() != 0, shape & 1 != 0);
                assert_eq!(triggers.year() != 0, shape & 2 != 0);
                assert_eq!(!triggers.stars().is_empty(), shape & 4 != 0);
                assert_eq!(triggers.heals() != 0, shape & 8 != 0);
                assert_eq!(triggers.fever() != 0, shape & 16 != 0);
                assert_eq!(triggers.psu() != 0, shape & 32 != 0);
                assert_eq!(triggers.sru() != 0, shape & 64 != 0);
                assert_eq!(triggers.tricks() != 0, lasting == 2);

                for accuracy in [ALWAYS, 8000, 0] {
                    for seed in 0..4 {
                        let config = sloppy(accuracy, seed);
                        let what = format!("shape {shape}, lasting {lasting}, seed {seed}");
                        assert_eq!(
                            at(&queue, &never, &config),
                            at(&queue, &cold, &config),
                            "{what}"
                        );
                        assert_eq!(
                            at(&queue, &nothing, &config),
                            at(&queue, &feverish, &config),
                            "{what}"
                        );
                        assert_eq!(
                            at(&queue, &boosted, &config),
                            at(&queue, &strong, &config),
                            "{what}"
                        );
                    }
                }
                copies += 1;
            }
        }
        assert_eq!(copies, 384, "every Appeal Boost copy");
    }
}
