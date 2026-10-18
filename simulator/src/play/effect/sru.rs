//! Skill Rate Up aka SRU.

use super::Lasting;
use crate::play::Scoring;
use crate::play::queue::Queue;
use crate::scalar::{Centi, Ms, Rate};
use crate::skill::SkillEffect;
use crate::song::{LANES, Time};

/// Skill Rate Up. Multiplies every other skill's chance, one at a time.
pub(in crate::play) struct Sru<E> {
    lasting: E,
    until: Time,
    boosting: Time,
    rates: [[Rate; 2]; LANES],
    base: [[Rate; 2]; LANES],
    /// SRU does not boost other SRUs.
    exempt: [[bool; 2]; LANES],
}

impl<'q, E> Lasting<'q> for Sru<E>
where
    E: Lasting<'q>,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = E::HEARTS;
    const FEVER: bool = E::FEVER;
    const PSU: bool = E::PSU;
    const PARAM: bool = E::PARAM;

    forward! {
        admit, next, release, converted, locked, lock, trick, occupy, tap, worth,
        heal, overheal_strength,
        fever, fevered, read_running, fever_paid,
        psu, expire, psu_tap, psu_paid,
        strength, hearted_strength, param, param_at, sync, seed,
        param_copy, sync_to,
    }

    #[inline]
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self {
        let mut base = [[0; 2]; LANES];
        let mut exempt = [[false; 2]; LANES];
        let lanes = base.iter_mut().zip(&mut exempt).zip(&scoring.skills).zip(&scoring.accessories);
        for (((chances, sru), skill), worn) in lanes {
            *chances = [skill.chance, worn.chance];
            *sru = [skill.effect, worn.effect].map(|e| e == SkillEffect::SkillRateUp);
        }
        Self {
            lasting: E::new(last, queue, scoring),
            until: Time(0),
            boosting: Time(0),
            rates: base,
            base,
            exempt,
        }
    }

    #[inline(always)]
    fn begin(&mut self) {
        self.boosting = self.until;
    }

    #[inline(always)]
    fn boosted(&self, now: Time) -> bool {
        now < self.boosting
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    fn rates(&self, lane: u32) -> [Rate; 2] {
        self.rates[lane as usize]
    }

    /// SRU activating while another is active does not do anything.
    #[inline(never)]
    fn sru(&mut self, lane: u32, now: Time, duration: Ms, value: u32) {
        if now < self.until {
            return;
        }
        self.until = self.lasting.occupy(lane, now, duration);
        let lanes = self.rates.iter_mut().zip(&self.base).zip(&self.exempt);
        for ((rates, chances), srus) in lanes {
            for ((rate, &chance), &sru) in rates.iter_mut().zip(chances).zip(srus) {
                *rate = if sru { chance } else { boost(chance, value) };
            }
        }
    }
}

/// SRU boost is `rate * (1 + value%)`. I actually wasn't sure at first if this was
/// additive or multiplicitive.
#[expect(clippy::integer_division, reason = "game mechanic")]
fn boost(rate: Rate, value: Centi) -> Rate {
    let boosted = u32::from(rate).saturating_mul(value.saturating_add(100)) / 100;
    Rate::try_from(boosted).unwrap_or(Rate::MAX)
}

#[cfg(test)]
mod tests {
    use super::boost;
    use crate::play::Scoring;
    use crate::play::activation::{AccessorySkill, Triggers};
    use crate::play::effect::tests::{
        FLIP, TEAM, clocked, fever, lasting, lock, overhealing, psu, trick,
    };
    use crate::play::queue::Queue;
    use crate::play::rng::Rng;
    use crate::play::tests::{
        ALWAYS, arena, at, fifty_one, only, play, scorer, seeded, sloppy, worn,
    };
    use crate::scalar::{Ms, Rate};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{Event, EventKind, Time};

    fn sru(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        value: u32,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::SkillRateUp, trigger, threshold, chance, duration);
        scoring.skills[lane].value = value;
    }

    fn worn_sru(scoring: &mut Scoring, lane: usize, chance: Rate, value: u32, duration: Ms) {
        scoring.accessories[lane] =
            AccessorySkill { effect: SkillEffect::SkillRateUp, ..worn(chance, value) };
        scoring.durations[lane][1] = duration;
    }

    /// The first `n` skill rolls to compare if it's boosting the rate with and without SRU.
    fn hits(rng: &mut Rng, n: usize, rate: Rate) -> u32 {
        core::iter::repeat_with(|| u32::from(rng.roll(rate))).take(n).sum()
    }

    #[test]
    fn check_sru_boost() {
        assert_eq!(boost(5000, 48), 7400);
        assert_eq!(boost(2500, 100), 5000, "multiplicitive");
        assert_eq!(boost(9300, 308), 37_944, "most a card rolled");
        assert_eq!(boost(60_000, 100), Rate::MAX);
    }

    #[test]
    fn check_sru() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        // Before first note and stays active past final note.
        sru(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, 100, 60_000);
        scoring.skills[2] = scorer(1, 2500, 1);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.sru(), triggers.lasting()), (1 << 8, 1 << 8));

        let mut short = false;
        for seed in 0..32 {
            let mut rng = Rng::new(seed, Rng::SKILLS);
            assert!(rng.roll(ALWAYS));
            let landed = hits(&mut rng, 51, 5000);
            let out = seeded(&queue, &scoring, seed);
            assert_eq!((out.activations, out.skill), (1 + landed, u64::from(landed)), "{seed}");
            assert_eq!(out.tap, 50 * 1000 + 1100, "seed {seed}: no tap score");
            short |= landed < 51;
        }
        assert!(short, "More than 100% activation");
    }

    #[test]
    fn check_sru_skips_its_own_rolls() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        for (lane, other) in [(6, 2), (2, 6)] {
            let mut scoring = Scoring::flat(TEAM);
            sru(&mut scoring, lane, TriggerType::RhythmIcons, 10, ALWAYS, 100, 60_000);
            scoring.skills[other] = scorer(10, FLIP, 1);

            let mut missed = false;
            for seed in 0..32 {
                // Icon 10 rolls right to left.
                // Icons 20 to 50 roll just the scorer.
                let mut rng = Rng::new(seed, Rng::SKILLS);
                let first = if lane > other {
                    assert!(rng.roll(ALWAYS));
                    rng.roll(FLIP)
                } else {
                    let first = rng.roll(FLIP);
                    assert!(rng.roll(ALWAYS));
                    first
                };
                let out = seeded(&queue, &scoring, seed);
                assert_eq!(out.skill, u64::from(first) + 4, "lane {lane}, seed {seed}");
                missed |= !first;
            }
            assert!(missed, "lane {lane}");
        }
    }

    #[test]
    fn check_sru_duration() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        // [3000, 3500) and [6000, 6500): notes 11 to 15 and 41 to 45.
        sru(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 100, 500);
        scoring.skills[2] = clocked(TriggerType::Combo, 1, FLIP, 1);

        let replay = |seed, from: u32, to: u32| {
            let mut rng = Rng::new(seed, Rng::SKILLS);
            let mut landed = 0;
            for t in (0..51).map(|i| 2000 + i * 100) {
                if t % 3000 == 0 {
                    assert!(rng.roll(ALWAYS));
                }
                let on = [3000, 6000].iter().any(|&s| t >= s + from && t < s + to);
                landed += u32::from(rng.roll(if on { ALWAYS } else { FLIP }));
            }
            landed
        };
        let (mut first, mut last) = (false, false);
        for seed in 0..64 {
            let landed = replay(seed, 0, 500);
            let out = seeded(&queue, &scoring, seed);
            assert_eq!((out.activations, out.skill), (2 + landed, u64::from(landed)), "{seed}");
            first |= replay(seed, 1, 500) != landed;
            last |= replay(seed, 0, 501) != landed;
        }
        assert!(first && last, "both edges are seen");
    }

    #[test]
    fn check_sru_doesnt_stack() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        sru(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, 0, 60_000);
        sru(&mut scoring, 7, TriggerType::RhythmIcons, 1, ALWAYS, 100, 60_000);
        scoring.skills[2] = clocked(TriggerType::Combo, 1, FLIP, 1);
        let mut idle = scoring;
        idle.skills[7] = scorer(1, ALWAYS, 0);

        for seed in 0..16 {
            let out = seeded(&queue, &scoring, seed);
            assert_eq!(out, seeded(&queue, &idle, seed), "seed {seed}");
            assert_eq!(u64::from(out.activations), 1 + 51 + out.skill, "seed {seed}");
            assert!(out.skill < 51, "seed {seed}: never boosted");
        }
    }

    #[test]
    fn check_sru_doesnt_affect_sru() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut boosted = Scoring::flat(TEAM);
        sru(&mut boosted, 8, TriggerType::Time, 1, ALWAYS, 100, 60_000);
        sru(&mut boosted, 7, TriggerType::RhythmIcons, 1, FLIP, 100, 60_000);
        boosted.skills[5] = scorer(2, 0, 1);
        worn_sru(&mut boosted, 5, FLIP, 100, 60_000);
        sru(&mut boosted, 4, TriggerType::RhythmIcons, 3, 0, 100, 60_000);
        boosted.accessories[4] = worn(2500, 10);
        boosted.skills[2] = clocked(TriggerType::Combo, 1, 2500, 100);

        let mut plain = boosted;
        plain.skills[8].value = 0;
        plain.accessories[4].chance = 5000;
        plain.skills[2].chance = 5000;
        for seed in 0..16 {
            assert_eq!(seeded(&queue, &boosted, seed), seeded(&queue, &plain, seed), "seed {seed}");
        }
    }

    #[test]
    fn check_sru_acc() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[6] = scorer(1, 0, 1);
        worn_sru(&mut scoring, 6, ALWAYS, 100, 60_000);
        scoring.skills[2] = clocked(TriggerType::Combo, 1, 2500, 1);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.sru(), triggers.lasting()), (1 << 6, 1 << 6));

        for seed in 0..16 {
            let mut rng = Rng::new(seed, Rng::SKILLS);
            assert!(!rng.roll(0) && rng.roll(ALWAYS));
            let landed = hits(&mut rng, 51, 5000);
            let out = seeded(&queue, &scoring, seed);
            assert_eq!((out.activations, out.accessory_activations), (landed, 1), "{seed}");
            assert_eq!(out.skill, u64::from(landed), "seed {seed}");
        }
    }

    /// For blocking other SRU lanes.
    #[test]
    fn check_sru_lane_blocking() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        sru(&mut scoring, 4, TriggerType::Perfects, 5, ALWAYS, 100, 550);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 9);

        let mut paying_nothing = scoring;
        psu(&mut paying_nothing, 4, TriggerType::Perfects, 5, ALWAYS, 0, 550);
        assert_eq!(play(&queue, &paying_nothing), out);
    }

    /// I probably don't need to keep writing checks that skills proc year group skills but they're
    /// finnicky.
    #[test]
    fn check_sru_chain() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), FLIP, 70);
        (scoring.members[0], scoring.members[2], scoring.members[5]) = (1, 1 << 2, 1 << 5);
        sru(&mut scoring, 5, TriggerType::RhythmIcons, 1, ALWAYS, 100, 60_000);
        scoring.skills[2] = scorer(1, ALWAYS, 1);
        let mut flat = scoring;
        flat.skills[5].value = 0;

        let mut missed = false;
        for seed in 0..32 {
            let out = seeded(&queue, &scoring, seed);
            assert_eq!((out.activations, out.skill), (1 + 51 + 1, 51 + 70), "seed {seed}");
            missed |= seeded(&queue, &flat, seed).skill == 51;
        }
        assert!(missed, "only guaranteed with SRU");
    }

    /// same comment as above.
    #[test]
    fn check_sru_score_scorers() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut boosted = Scoring::flat(TEAM);
        sru(&mut boosted, 8, TriggerType::Time, 1, ALWAYS, 100, 60_000);
        boosted.skills[6] = clocked(TriggerType::Score, 1000, FLIP, 1);
        let mut certain = boosted;
        certain.skills[8].value = 0;
        certain.skills[6].chance = ALWAYS;
        for seed in 0..8 {
            assert_eq!(seeded(&queue, &boosted, seed), seeded(&queue, &certain, seed), "{seed}");
        }
    }

    /// SRU never rolling should be the same as there being no SRU at all.
    #[test]
    fn check_sru_that_boosts_nothing_changes_nothing() {
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
        for shape in 0..64_u32 {
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
                match lasting {
                    0 => {}
                    1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                    _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                }

                let mut never = team;
                sru(&mut never, 1, TriggerType::RhythmIcons, 3, 0, 100, 700);
                let mut cold = team;
                cold.skills[1] = scorer(3, 0, 0);
                let mut nothing = team;
                sru(&mut nothing, 1, TriggerType::RhythmIcons, 3, FLIP, 0, 700);
                let mut feverish = team;
                fever(&mut feverish, 1, TriggerType::RhythmIcons, 3, FLIP, 0, 700);

                let triggers = Triggers::build(&queue, &never);
                assert_eq!(triggers.sru(), 1 << 1);
                assert_eq!(triggers.score() != 0, shape & 1 != 0);
                assert_eq!(triggers.year() != 0, shape & 2 != 0);
                assert_eq!(!triggers.stars().is_empty(), shape & 4 != 0);
                assert_eq!(triggers.heals() != 0, shape & 8 != 0);
                assert_eq!(triggers.fever() != 0, shape & 16 != 0);
                assert_eq!(triggers.psu() != 0, shape & 32 != 0);
                assert_eq!(triggers.tricks() != 0, lasting == 2);

                for accuracy in [ALWAYS, 8000, 0] {
                    for seed in 0..8 {
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
                    }
                }
                copies += 1;
            }
        }
        assert_eq!(copies, 192, "every SRU copy");
    }
}
