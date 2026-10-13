//! Overheal logic.
//! For Heal SIS scoring logic see `compile.rs` and `effect.rs`

use super::Lasting;
use crate::play::Scoring;
use crate::play::queue::Queue;
use crate::play::tap::{HEART_CAP, HEART_UNIT, overheal_score_val};
use crate::song::Time;

/// A team that can heal. We don't care about tracking heals if there's no healer cards or accs.
pub(in crate::play) struct Healing<E> {
    lasting: E,
    hearts: Hearts,
}

impl<'q, E> Lasting<'q> for Healing<E>
where
    E: Lasting<'q>,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = true;

    forward! {
        admit, next, release, converted, locked, lock, trick, occupy, worth,
    }

    #[inline]
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self {
        Self {
            lasting: E::new(last, queue, scoring),
            hearts: Hearts::new(scoring),
        }
    }

    #[inline(always)]
    fn tap(&self, now: Time, head: bool, tail: bool, combo: u16, lane: u16, note: u16) -> u64 {
        let worth = self.lasting.worth(now, head, tail);
        overheal_score_val(worth, combo, lane, note, self.hearts.multiplier)
    }

    #[inline(always)]
    fn heal(&mut self, hp: u8) {
        self.hearts.heal(hp);
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "bool")]
    fn overheal_strength(&self, locked: bool) -> u64 {
        self.hearts.strength[usize::from(locked)]
    }

    #[inline(always)]
    fn heart_multiplier(&self) -> u16 {
        self.hearts.multiplier
    }
}

struct Hearts {
    max_hp: u32,
    per_heart: u32,
    max: u32,
    count: u32,
    pool: u32,
    multiplier: u16,
    base: [u64; 2],
    strength: [u64; 2],
}

impl Hearts {
    fn new(scoring: &Scoring) -> Self {
        let base = [
            scoring.team_strength,
            scoring.team_strength.saturating_add(scoring.trick),
        ]
        .map(u64::from);
        Self {
            max_hp: u32::from(scoring.max_hp),
            per_heart: u32::from(scoring.per_heart),
            max: u32::from(scoring.max_hearts),
            count: 0,
            pool: 0,
            multiplier: HEART_UNIT,
            base,
            strength: base.map(|s| s.saturating_mul(u64::from(HEART_UNIT))),
        }
    }

    #[inline(always)]
    fn heal(&mut self, hp: u8) {
        if self.count >= self.max {
            return;
        }
        let pool = self.pool.saturating_add(u32::from(hp));
        if pool < self.max_hp {
            self.pool = pool;
            return;
        }
        self.bank(pool);
    }

    /// When an HP bar is filled up from healing, that overheal level is "banked".
    #[inline(never)]
    fn bank(&mut self, pool: u32) {
        let (Some(banked), Some(left)) =
            (pool.checked_div(self.max_hp), pool.checked_rem(self.max_hp))
        else {
            return;
        };
        let count = self.count.saturating_add(banked);
        (self.count, self.pool) = if count >= self.max {
            (self.max, 0)
        } else {
            (count, left)
        };
        let bonus = self.count.saturating_mul(self.per_heart).min(u32::from(HEART_CAP));
        self.multiplier = HEART_UNIT.saturating_add(u16::try_from(bonus).unwrap_or(HEART_CAP));
        self.strength = self.base.map(|s| s.saturating_mul(u64::from(self.multiplier)));
    }
}

#[cfg(test)]
mod tests {
    use super::Hearts;
    use crate::play::activation::{AccessorySkill, Triggers};
    use crate::play::effect::tests::{
        FLIP, ONE_HEART_EVERY_TEN, TEAM, clocked, healer, lock, overhealing, trick,
    };
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, arena, at, fifty_one, only, play, scorer, sloppy, tap, worn};
    use crate::play::{Outcome, Scoring};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{Event, EventKind, Time};

    fn hearts(max_hp: u8, per_heart: u16, max_hearts: u16) -> Hearts {
        let mut scoring = Scoring::flat(TEAM);
        (scoring.max_hp, scoring.per_heart, scoring.max_hearts) = (max_hp, per_heart, max_hearts);
        Hearts::new(&scoring)
    }

    #[test]
    fn check_heart_affects_score() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        overhealing(&mut scoring, 30);
        healer(&mut scoring, 4, TriggerType::RhythmIcons, 10, ALWAYS, 30);
        assert_eq!(Triggers::build(&queue, &scoring).heals(), 1 << 4);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill), (5, 0));
        assert_eq!(out.tap, ONE_HEART_EVERY_TEN);

        scoring.max_hearts = 0;
        assert_eq!(Triggers::build(&queue, &scoring).heals(), 0, "nothing to bank");
        assert_eq!(play(&queue, &scoring).tap, 50 * 1000 + 1100);
    }

    #[test]
    fn check_accessory_heal_overheals() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        overhealing(&mut scoring, 30);
        scoring.skills[4] = scorer(10, 0, 1);
        scoring.accessories[4] = AccessorySkill { effect: SkillEffect::Healer, ..worn(ALWAYS, 0) };
        scoring.heals[4] = [0, 30];
        assert_eq!(Triggers::build(&queue, &scoring).heals(), 1 << 4);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 5));
        assert_eq!(out.tap, ONE_HEART_EVERY_TEN);

        scoring.accessories[4].chance = 0;
        assert_eq!(Triggers::build(&queue, &scoring).heals(), 0, "can't roll");
    }

    #[test]
    fn check_hearts_dont_need_heal_sis() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut bare = Scoring::flat(TEAM);
        overhealing(&mut bare, 30);
        healer(&mut bare, 4, TriggerType::RhythmIcons, 10, ALWAYS, 30);
        let mut healed = bare;
        healed.skills[4].value = 30 * 480;

        let out = play(&queue, &healed);
        assert_eq!(out, Outcome { skill: 5 * 30 * 480, ..play(&queue, &bare) });
        assert_eq!(out.tap, ONE_HEART_EVERY_TEN);
    }

    /// 54 max HP is 3.38% a heart and 60 hearts. The 60th would be +202.8%.
    #[test]
    fn check_overheal_cap_is_two_hundred_percent() {
        let mut h = hearts(54, 338, 60);
        for _ in 0..59 {
            h.heal(54);
        }
        assert_eq!((h.count, h.pool, h.multiplier), (59, 0, 10_000 + 59 * 338));
        h.heal(53);
        assert_eq!((h.count, h.pool), (59, 53));
        h.heal(u8::MAX);
        assert_eq!((h.count, h.pool, h.multiplier), (60, 0, 30_000), "not 30,280");
        h.heal(u8::MAX);
        assert_eq!((h.count, h.pool, h.multiplier), (60, 0, 30_000), "it stops");
        assert_eq!(h.strength, [u64::from(TEAM) * 30_000; 2]);
    }

    /// 7 hearts at +30% would be +210%.
    #[test]
    fn check_overheal_bonus_stops_at_two_hundred_percent() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        (scoring.max_hp, scoring.per_heart, scoring.max_hearts) = (9, 3000, 7);
        healer(&mut scoring, 4, TriggerType::RhythmIcons, 1, ALWAYS, 63);
        assert_eq!(play(&queue, &scoring).tap, 50 * 3000 + 3300);
    }

    /// For the scenario where you start a hold, you get healed to a new heart and then you release.
    #[test]
    fn check_hold_takes_its_hearts_at_release() {
        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            tap(2500, 2),
            tap(3000, 1),
            (Time(4000), Event::new(0, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        overhealing(&mut scoring, 30);
        healer(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 30);
        assert_eq!(play(&queue, &scoring).tap, 1000 + 1100 + 1375);
    }

    /// Makes sure that the bonus score from overheal contributes to score-based scorers' threshold.
    #[test]
    fn check_hearts_score_affects_score_scorers() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        overhealing(&mut scoring, 30);
        healer(&mut scoring, 4, TriggerType::RhythmIcons, 10, ALWAYS, 30);
        scoring.skills[6] = clocked(TriggerType::Score, 10_000, ALWAYS, 0);

        let out = play(&queue, &scoring);
        assert_eq!((out.tap, out.activations), (ONE_HEART_EVERY_TEN, 5 + 6));
        scoring.max_hearts = 0;
        assert_eq!(play(&queue, &scoring).activations, 5 + 5);
    }

    #[test]
    fn check_heal_that_banks_nothing_changes_nothing() {
        let stream: Vec<_> = (0..51_u32)
            .map(|i| {
                (Time(2000 + i * 100), Event::new((i % 9) as u8, EventKind::Tap, i % 3 == 0, false))
            })
            .collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut base = Scoring::flat(TEAM);
        base.trick = 9_999;
        overhealing(&mut base, 30);
        base.skills[2] = scorer(4, FLIP, 100);
        base.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
        base.skills[6] = clocked(TriggerType::Time, 1, FLIP, 7);
        base.skills[7] = clocked(TriggerType::Perfects, 2, FLIP, 3);
        healer(&mut base, 1, TriggerType::RhythmIcons, 3, FLIP, 0);
        base.accessories[1] = AccessorySkill { effect: SkillEffect::Healer, ..worn(FLIP, 0) };

        let mut copies = 0;
        let shapes = (0..3).flat_map(|lasting| (0..8).map(move |i| (i & 1, i & 2, i & 4, lasting)));
        for (score, year, star, lasting) in shapes {
            let mut team = base;
            if score != 0 {
                team.skills[3] = clocked(TriggerType::Score, 1500, FLIP, 50);
            }
            if year != 0 {
                team.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), FLIP, 70);
                (team.members[0], team.members[2], team.members[5]) = (1, 1 << 2, 1 << 5);
            }
            if star != 0 {
                team.skills[8] = clocked(TriggerType::StarNotes, 1, FLIP, 5);
            }
            match lasting {
                0 => {}
                1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
            }
            let off = Scoring { max_hearts: 0, ..team };
            let triggers = Triggers::build(&queue, &team);
            assert_eq!(triggers.heals(), 1 << 1);
            assert_eq!(Triggers::build(&queue, &off).heals(), 0);
            assert_eq!(triggers.score() != 0, score != 0);
            assert_eq!(triggers.year() != 0, year != 0);
            assert_eq!(!triggers.stars().is_empty(), star != 0);
            assert_eq!(
                (triggers.lasting() != 0, triggers.tricks() != 0),
                (lasting != 0, lasting == 2)
            );

            for accuracy in [ALWAYS, 8000, 0] {
                for seed in 0..16 {
                    let config = sloppy(accuracy, seed);
                    assert_eq!(
                        at(&queue, &team, &config),
                        at(&queue, &off, &config),
                        "seed {seed}"
                    );
                }
            }

            let mut heals = team;
            heals.heals[1] = [12, 18];
            if score == 0 {
                for seed in 0..16 {
                    let config = sloppy(8000, seed);
                    let (on, off) = (at(&queue, &heals, &config), at(&queue, &off, &config));
                    assert_eq!(Outcome { tap: on.tap, ..off }, on, "seed {seed}");
                    assert!(on.tap > off.tap, "seed {seed}");
                }
            }
            copies += 1;
        }
        assert_eq!(copies, 24, "all heal copies");
    }
}
