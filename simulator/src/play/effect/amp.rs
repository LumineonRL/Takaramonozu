//! Amplify, aka amp.
//! Boosts the skill level of the next card to activate by this skill's value.
//! e.g. Amp activates and gives +6, the next card to activate has skill level 4
//! they activate instead as if they were skill level 10.
//! This skill (and encore too) have a ridiculous amount of interactions with other skills so
//! I'll document some of them here.
//!
//! Encores activating don't spend an amplify charge
//! Encore copying an Amp re-amps, unless an Amp already proc'd at that exact moment.
//! Encore copying a skill that was Amp'd, replays the Amp/
//! Accs get Amp'd.
//! Only one amplify can activate at a given moment. If more than one proc at the exact same time,
//! successive ones do nothing. They roll right to left, of course.
//! Amp boosts amp. hot amp on amp action.
//! Amp skills count for character activations for year-group-based triggers.
//! Duration skills that lock themselves from activating while they're already active such as
//! SRU or Param Boost still count as a skill activation and will consume an amp charge.
//! For example, SRU activates, amp activates while SRU is active, SRU activates again
//! while SRU is still active. SRU was blocked since the skill is still active, so the skill
//! does absolutely nothing, but the amp charge is still consumed. Tragic.
//! Boosts can go up to skill level 16. Never displayed in game, but cards had hidden values
//! for skill levels past the usual cap of 8. Only non-promo URs had values up to 16, lower rarity
//! and promo cards capped out well below 16.
//! This simulator just pads everything to go up to skill level 16 by copying the max until everything
//! is at 16 because it's less branchy, less bounds checking.
//! Only value and duration get boosted. activation rate and threshold do not.
//! There's a coinflip mechanic regarding skill activation order. I'm not going to type it up again,
//! search up `coinflip` in this codebase, I don't remember which module it's in. `rng.rs`, I think.

use super::Lasting;
use super::encore::{Copied, Repeats};
use crate::play::Scoring;
use crate::play::activation::{AMP_LEVELS, Payload};
use crate::play::queue::Queue;
use crate::play::rng::Rng;
use crate::scalar::{Ms, RATE_UNIT};
use crate::skill::SkillEffect;
use crate::song::{LANES, Time};

#[expect(clippy::cast_possible_truncation, reason = "16")]
const MOST: u32 = AMP_LEVELS as u32;

pub(in crate::play) struct Amp<E, R> {
    lasting: E,
    /// Number of levels the next skill will be boosted by
    standing: u32,
    /// Amps can't activate on the exact frame that they triggered.
    /// so this tracks if any time has passed
    fresh: u32,
    amps: u16,
    others: u16,
    encores: u16,
    first: u16,
    second: u16,
    /// card then acc you know the drill by now.
    base: [[Payload; 2]; LANES],
    /// Unfortunately `E` was already taken, so`R` is for Encore's slot aka Repeater.
    /// And the funny thing is I don't even remember what the `E` on `lasting` was supposed to stand for LMAO.
    /// Like why wouldn't I use `L`?
    slot: R,
}

impl<'q, E, R> Lasting<'q> for Amp<E, R>
where
    E: Lasting<'q>,
    R: Repeats,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = E::HEARTS;
    const FEVER: bool = E::FEVER;
    const PSU: bool = E::PSU;
    const PARAM: bool = E::PARAM;
    const AMP: bool = true;
    const REPEAT: bool = R::ON;

    forward! {
        admit, next, release, converted, locked, lock_land, total_trick_land, occupy, total_trick_tap, total_trick_worth,
        healer_land, heart_strength, heart_multiplier,
        fever_land, fever_tap, fever_per_note, fever_paid,
        psu_land, expire, psu_tap, psu_paid,
        sru_boosted, sru_rates, sru_land,
        param_strength, param_hearted_strength, param_land, param_land_at,
        encore_param, encore_sync,
    }

    #[inline]
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self {
        let cards = |effect| {
            (0_u32..)
                .zip(&scoring.skills)
                .filter(|(_, skill)| skill.threshold != 0 && skill.effect == effect)
                .fold(0, |mask, (lane, _)| mask | 1_u16.checked_shl(lane).unwrap_or(0))
        };
        let (amps, encores) = (cards(SkillEffect::Amplify), cards(SkillEffect::Encore));
        let others = !(amps | encores);
        let mut base = [[Payload::default(); 2]; LANES];
        let columns = (scoring.skills.iter().zip(&scoring.accessories))
            .zip(scoring.durations.iter().zip(&scoring.heals).zip(&scoring.boosts));
        for (rows, ((skill, worn), ((durations, heals), boosts))) in base.iter_mut().zip(columns) {
            let halves = [(skill.effect, skill.value), (worn.effect, worn.value)];
            let rest = durations.iter().zip(heals).zip(boosts);
            for ((row, (effect, value)), ((&duration, &hp), &boost)) in
                rows.iter_mut().zip(halves).zip(rest)
            {
                let extra = if effect == SkillEffect::Healer {
                    u16::from(hp)
                } else {
                    boost
                };
                *row = Payload { value, duration, extra };
            }
        }
        Self {
            lasting: E::new(last, queue, scoring),
            standing: 0,
            fresh: 0,
            amps,
            others,
            encores,
            first: amps,
            second: others,
            base,
            slot: R::default(),
        }
    }

    #[inline(always)]
    fn begin(&mut self) {
        self.lasting.begin();
    }

    #[inline(always)]
    fn end(&mut self) {
        self.standing = if self.fresh == 0 {
            self.standing
        } else {
            self.fresh
        };
        self.fresh = 0;
        self.slot.end();
    }

    /// This might need to come out of this module since it's relevant for Encore too, not
    /// exclusive to amp. todo I guess.
    #[inline]
    #[expect(clippy::integer_division, reason = "game mechanic")]
    fn seed(&mut self, seed: u64) {
        if self.amps != 0 && Rng::new(seed, Rng::COIN).roll(RATE_UNIT / 2) {
            (self.first, self.second) = (self.others, self.amps);
        }
        self.lasting.seed(seed);
    }

    #[inline(always)]
    fn amp_order(&self, mask: u16) -> u64 {
        let [first, second, last] =
            [self.first, self.second, self.encores].map(|g| u64::from(mask & g));
        (first << 32) | (second << 16) | last
    }

    #[inline(always)]
    fn amp_spend(&mut self, take: bool) -> u32 {
        let levels = if take { self.standing } else { 0 };
        self.standing = if take { 0 } else { self.standing };
        levels
    }

    #[inline(always)]
    #[expect(
        clippy::indexing_slicing,
        reason = "same lanes as everywhere else in the repo, a bool, amp levels bounded to 16."
    )]
    fn amp_payload(&self, lane: u32, worn: bool, levels: u32, scoring: &Scoring) -> Payload {
        let (lane, half) = (lane as usize, usize::from(worn));
        let row = (levels.saturating_sub(1) & (MOST - 1)) as usize;
        *if levels == 0 {
            &self.base[lane][half]
        } else {
            &scoring.amplified[lane][half][row]
        }
    }

    /// Only one amp can trigger per moment.
    #[inline(always)]
    fn amp_land(&mut self, levels: u32) {
        self.fresh = if self.fresh == 0 {
            levels.min(MOST)
        } else {
            self.fresh
        };
    }

    #[inline(always)]
    fn encore_write(&mut self, landed: Copied) {
        self.slot.write(landed);
    }

    #[inline(always)]
    fn encore_copy(&mut self) -> Option<Copied> {
        self.slot.copy()
    }

    #[inline(always)]
    fn sync_land(&mut self, lane: u32, worn: bool, now: Time, duration: Ms) -> u16 {
        let picked = self.lasting.sync_land(lane, worn, now, duration);
        self.slot.pick(lane, picked);
        picked
    }

    #[inline(always)]
    fn encore_picked(&self, lane: u32) -> u16 {
        self.slot.picked(lane)
    }
}

// This test module is a behemoth but there's so many interactions with this mechanic it's kind of needed.
// coin flip in particular makes this absolute hell to test.
// Every test is a fun seed search to get the mechanic you're trying to observe to occur.
#[cfg(test)]
mod tests {
    use crate::play::Scoring;
    use crate::play::activation::{AMP_LEVELS, AccessorySkill, Payload, Skill, Triggers};
    use crate::play::effect::tests::{
        BOOST, FLIP, STATS, TEAM, clocked, draws, lasting, lock, others_first, overhealing, param,
        psu, sync, trick,
    };
    use crate::play::queue::Queue;
    use crate::play::tests::{
        ALWAYS, arena, at, fifty_one, only, play, scorer, seeded, sloppy, worn,
    };
    use crate::scalar::{Ms, Rate};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{Event, EventKind, LANES, Time};

    fn amp(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        levels: u32,
    ) {
        scoring.skills[lane] = Skill {
            effect: SkillEffect::Amplify,
            ..clocked(trigger, threshold, chance, levels)
        };
    }

    /// skill level go up under amp
    #[expect(clippy::arithmetic_side_effects, reason = "small test numbers")]
    fn climbing(scoring: &mut Scoring, lane: usize, half: usize, base: u32, step: u32) {
        for (row, levels) in scoring.amplified[lane][half].iter_mut().zip(1..) {
            row.value = base + step * levels;
        }
    }

    /// skill level stay the same
    fn flat(scoring: &mut Scoring) {
        for lane in 0..LANES {
            let halves = [
                (scoring.skills[lane].effect, scoring.skills[lane].value),
                (scoring.accessories[lane].effect, scoring.accessories[lane].value),
            ];
            for (half, (effect, value)) in halves.into_iter().enumerate() {
                let extra = if effect == SkillEffect::Healer {
                    u16::from(scoring.heals[lane][half])
                } else {
                    scoring.boosts[lane][half]
                };
                let duration = scoring.durations[lane][half];
                scoring.amplified[lane][half] = [Payload { value, duration, extra }; AMP_LEVELS];
            }
        }
    }

    /// Amp triggered on notes 10 and 50.
    /// Scorer triggers every 5 icons at 15, 25, 35 and 45.
    #[test]
    fn check_amp() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::RhythmIcons, 10, ALWAYS, 4);
        scoring.skills[2] = scorer(5, ALWAYS, 1000);
        climbing(&mut scoring, 2, 0, 1000, 100);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.amp(), triggers.lasting()), (1 << 8, 0));

        for seed in 0..8 {
            let out = seeded(&queue, &scoring, seed);
            assert_eq!((out.activations, out.skill), (15, 6 * 1000 + 4 * 1400), "seed {seed}");
            assert_eq!(out.tap, 50 * 1000 + 1100, "seed {seed}");
        }
    }

    /// Amps triggers on 10, 20, ...
    /// Scorer activating on same icon does not get amp boost,
    /// next scorer activation does get amp boost.
    #[test]
    fn check_an_amp_skips_its_own_tick() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        for lane in [0, 8] {
            let mut scoring = Scoring::flat(TEAM);
            amp(&mut scoring, lane, TriggerType::RhythmIcons, 10, ALWAYS, 4);
            scoring.skills[4] = scorer(1, ALWAYS, 1000);
            climbing(&mut scoring, 4, 0, 1000, 100);
            for seed in 0..8 {
                let out = seeded(&queue, &scoring, seed);
                assert_eq!(out.skill, 46 * 1000 + 5 * 1400, "lane {lane}, seed {seed}");
            }
        }
    }

    /// Amp and scorer both every 10 icons
    /// Amplify first, amp takes +4.
    /// Other first, scorer does.
    #[test]
    fn check_the_coin_flip() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 2, TriggerType::RhythmIcons, 10, ALWAYS, 4);
        climbing(&mut scoring, 2, 0, 4, 2);
        scoring.skills[6] = scorer(10, ALWAYS, 1000);
        climbing(&mut scoring, 6, 0, 1000, 100);

        let mut seen = [false; 2];
        for seed in 0..32 {
            let first = others_first(seed);
            let want = if first { 1000 + 4 * 1400 } else { 5 * 1000 };
            assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}");
            seen[usize::from(first)] = true;
        }
        assert_eq!(seen, [true; 2]);
    }

    /// Icon 10 rolls the +4.
    /// Icon 12 rolls another Amp that with a +4, 2 + 2 * 4 = 10
    /// Scorer on icon 13 gets +10
    #[test]
    fn check_amp_on_amp() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::RhythmIcons, 10, ALWAYS, 4);
        climbing(&mut scoring, 8, 0, 4, 2);
        amp(&mut scoring, 7, TriggerType::RhythmIcons, 12, ALWAYS, 2);
        climbing(&mut scoring, 7, 0, 2, 2);
        scoring.skills[2] = scorer(13, ALWAYS, 1000);
        climbing(&mut scoring, 2, 0, 1000, 100);

        let out = play(&queue, &scoring);
        assert_eq!(out.skill, 3 * 2000, "13, 26 and 39");
        assert_eq!(out.activations, 5 + 4 + 3);
    }

    /// Two amps activate at the same time.
    /// right one activates, left does nothing.
    #[test]
    fn check_one_amp_a_moment() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::RhythmIcons, 10, ALWAYS, 4);
        amp(&mut scoring, 7, TriggerType::RhythmIcons, 10, ALWAYS, 2);
        scoring.skills[2] = scorer(13, ALWAYS, 1000);
        climbing(&mut scoring, 2, 0, 1000, 100);

        let out = play(&queue, &scoring);
        assert_eq!(out.skill, 3 * 1400);
        assert_eq!(out.activations, 10 + 3);
    }

    #[test]
    fn check_encore_ignores_amp() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::RhythmIcons, 7, ALWAYS, 4);
        climbing(&mut scoring, 8, 0, 4, 0);
        scoring.skills[5] = Skill { effect: SkillEffect::Encore, ..scorer(1, ALWAYS, 0) };
        scoring.skills[2] = scorer(15, ALWAYS, 1000);
        climbing(&mut scoring, 2, 0, 1000, 100);
        let encored = play(&queue, &scoring);
        assert_eq!(encored.skill, 3 * 3 * 1400, "each +4 scorer, copied on its icon and the next");

        let mut stolen = scoring;
        stolen.skills[5].effect = SkillEffect::Scorer;
        let out = play(&queue, &stolen);
        assert_eq!((out.activations, out.skill), (encored.activations, 3 * 1000));
    }

    #[test]
    fn check_failed_activation_doesnt_spend_amp() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 4);
        scoring.skills[2] = scorer(1, FLIP, 1000);
        climbing(&mut scoring, 2, 0, 1000, 100);

        let mut differs = false;
        for seed in 0..32 {
            let d = draws(seed, 53);
            let (mut standing, mut want, mut rolled_off) = (false, 0, 0);
            for (index, &hit) in d.iter().enumerate() {
                if index == 16 || index == 47 {
                    standing = true;
                    continue;
                }
                if hit == 1 {
                    want += if standing { 1400 } else { 1000 };
                    standing = false;
                }
                rolled_off += if standing && hit == 1 {
                    0
                } else {
                    u64::from(standing)
                };
            }
            assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}");
            differs |= rolled_off > 0;
        }
        assert!(differs, "a miss that would have spent it");
    }

    /// Goes through each skill effect and makes sure they're boosted by amplify.
    #[test]
    fn check_amp_skills_are_boosted() {
        type Case = (SkillEffect, u32, Ms, u16, u32, Ms, u16);
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let cases: [Case; 9] = [
            (SkillEffect::Scorer, 1000, 0, 0, 3000, 0, 0),
            (SkillEffect::Healer, 480 * 4, 0, 4, 480 * 9, 0, 9),
            (SkillEffect::PerfectLock, 0, 500, 0, 0, 2500, 0),
            (SkillEffect::TotalTrick, 0, 500, 0, 0, 2500, 0),
            (SkillEffect::ComboFever, 40, 500, 0, 90, 2500, 0),
            (SkillEffect::PerfectScoreUp, 300, 500, 0, 700, 2500, 0),
            (SkillEffect::SkillRateUp, 20, 500, 0, 100, 2500, 0),
            (SkillEffect::AppealBoost, BOOST, 500, 50, 2 * BOOST, 2500, 100),
            (SkillEffect::Sync, 0, 500, 0, 0, 2500, 0),
        ];
        for (effect, value, duration, extra, amped, longer, more) in cases {
            let mut team = Scoring { stats: STATS, trick: 9000, ..Scoring::flat(TEAM) };
            overhealing(&mut team, 30);
            team.skills[6] = clocked(TriggerType::Combo, 20, FLIP, 1);
            lasting(&mut team, 4, effect, TriggerType::RhythmIcons, 25, ALWAYS, duration);
            team.skills[4].value = value;
            if effect == SkillEffect::Healer {
                team.heals[4][0] = u8::try_from(extra).expect("hp");
            }
            if matches!(effect, SkillEffect::AppealBoost | SkillEffect::Sync) {
                (team.targets[4][0], team.boosts[4][0]) = (0b10, extra);
            }
            let mut amped_team = team;
            amp(&mut amped_team, 8, TriggerType::Time, 3, ALWAYS, 4);
            amped_team.amplified[4][0][3] = Payload { value: amped, duration: longer, extra: more };

            let mut plain = team;
            plain.skills[8] = clocked(TriggerType::Time, 3, ALWAYS, 0);
            let mut as_amped = plain;
            as_amped.skills[4].value = amped;
            as_amped.durations[4][0] = longer;
            if effect == SkillEffect::Healer {
                as_amped.heals[4][0] = u8::try_from(more).expect("hp");
            }
            as_amped.boosts[4][0] = more;

            let mut moved = false;
            for accuracy in [ALWAYS, 7000] {
                for seed in 0..8 {
                    let config = sloppy(accuracy, seed);
                    let out = at(&queue, &amped_team, &config);
                    assert_eq!(out, at(&queue, &as_amped, &config), "{effect:?}, seed {seed}");
                    moved |= out != at(&queue, &plain, &config);
                }
            }
            assert!(moved, "{effect:?}: you done messed up, Aampron.");
        }
    }

    /// Param Boost w/ +4 amp boosts param by 100%
    /// Sync copying its target should get that too.
    #[test]
    fn check_amped_boost_applies_to_sync() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut team = Scoring { stats: STATS, ..Scoring::flat(TEAM) };
        param(&mut team, 4, TriggerType::RhythmIcons, 25, ALWAYS, 12_000, 2500);
        (team.targets[4][0], team.boosts[4][0]) = (0b10, 50);
        sync(&mut team, 0, TriggerType::Combo, 23, ALWAYS, 1000, 0b10);
        let mut amped = team;
        amp(&mut amped, 8, TriggerType::Time, 3, ALWAYS, 4);
        amped.amplified[4][0][3] = Payload { value: 24_000, duration: 2500, extra: 100 };

        let mut plain = team;
        plain.skills[8] = clocked(TriggerType::Time, 3, ALWAYS, 0);
        plain.skills[4].value = 24_000;
        plain.boosts[4][0] = 100;
        let out = play(&queue, &amped);
        assert_eq!(out, play(&queue, &plain));
        plain.boosts[4][0] = 50;
        assert_ne!(out, play(&queue, &plain), "copy took boost's percent");
    }

    /// That amps contribute to year group triggers.
    #[test]
    fn check_triggers_year_group() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::RhythmIcons, 10, ALWAYS, 4);
        scoring.skills[7] = scorer(10, ALWAYS, 0);
        (scoring.members[8], scoring.members[7], scoring.members[0]) = (1 << 8, 1 << 7, 1);
        scoring.skills[0] = clocked(TriggerType::YearGroup, (1 << 8) | (1 << 7), ALWAYS, 1000);
        climbing(&mut scoring, 0, 0, 1000, 100);
        for seed in 0..8 {
            let out = seeded(&queue, &scoring, seed);
            assert_eq!(out.skill, 5 * 1400, "seed {seed}");
        }
    }

    /// Amped Param Boost with PSU up
    /// trigger on icon 25's spawn at 3800,
    /// Tap at 3900 is not boosted.
    #[test]
    fn check_duration_effect_ends() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut team = Scoring::flat(TEAM);
        psu(&mut team, 2, TriggerType::Time, 6, ALWAYS, 1, 100);
        param(&mut team, 4, TriggerType::RhythmIcons, 25, ALWAYS, BOOST, 50);
        let mut amped = team;
        amp(&mut amped, 8, TriggerType::Time, 3, ALWAYS, 4);
        amped.amplified[4][0][3] = Payload { value: 2 * BOOST, duration: 50, extra: 0 };
        let mut plain = team;
        plain.skills[8] = clocked(TriggerType::Time, 3, ALWAYS, 0);
        plain.skills[4].value = 2 * BOOST;
        let out = play(&queue, &amped);
        assert_eq!(out, play(&queue, &plain));
        assert_eq!(out.tap, 50 * 1000 + 1100, "no tap boosted");
    }

    #[test]
    fn check_amp_on_score_scorer() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::RhythmIcons, 50, ALWAYS, 4);
        scoring.skills[2] = scorer(50, ALWAYS, 100_000);
        climbing(&mut scoring, 2, 0, 100_000, 100);
        scoring.skills[3] = clocked(TriggerType::Score, 60_000, ALWAYS, 7);
        climbing(&mut scoring, 3, 0, 7, 1);
        for seed in 0..8 {
            let out = seeded(&queue, &scoring, seed);
            assert_eq!(out.skill, 100_000 + 11 + 7, "seed {seed}");
        }
    }

    #[test]
    fn check_amp_accessory() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[8] = scorer(10, 0, 1);
        scoring.accessories[8] = AccessorySkill { effect: SkillEffect::Amplify, ..worn(ALWAYS, 4) };
        scoring.skills[2] = scorer(5, ALWAYS, 1000);
        climbing(&mut scoring, 2, 0, 1000, 100);
        assert_eq!(Triggers::build(&queue, &scoring).amp(), 1 << 8);
        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (10, 5));
        assert_eq!(out.skill, 6 * 1000 + 4 * 1400);
    }

    #[test]
    fn check_amped_duration_holds_lane() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        amp(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, 4);
        lock(&mut scoring, 4, TriggerType::RhythmIcons, 3, ALWAYS, 100);
        scoring.amplified[4][0][3].duration = 1000;
        let out = play(&queue, &scoring);

        let mut plain = scoring;
        plain.skills[8] = clocked(TriggerType::Time, 1, ALWAYS, 0);
        let short = play(&queue, &plain);
        assert!(out.activations < short.activations, "{} {}", out.activations, short.activations);
    }

    /// Differential test to make sure amp's state isn't leaking into other unrelated skills.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "agreed, but we do one of these for every skill effect."
    )]
    fn check_amp_copies() {
        let stream: Vec<_> = (0..51_u32)
            .map(|i| {
                (Time(2000 + i * 100), Event::new((i % 9) as u8, EventKind::Tap, i % 3 == 0, false))
            })
            .collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let amp_first: Vec<u64> = (0..64).filter(|&s| !others_first(s)).take(3).collect();

        let mut base = Scoring { stats: STATS, ..Scoring::flat(TEAM) };
        base.trick = 9_999;
        base.skills[1] = scorer(3, FLIP, 1);
        base.skills[2] = scorer(4, FLIP, 100);
        base.skills[3] = scorer(7, ALWAYS, 0);
        base.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
        base.accessories[5] = worn(FLIP, 20);
        base.skills[6] = clocked(TriggerType::Time, 1, FLIP, 7);
        base.skills[7] = clocked(TriggerType::Perfects, 2, FLIP, 3);

        let mut copies = 0;
        for shape in 0..256_u32 {
            for lasting in 0..3 {
                let mut team = base;
                if shape & 1 != 0 {
                    team.skills[3] = clocked(TriggerType::Score, 1500, ALWAYS, 50);
                }
                if shape & 2 != 0 {
                    team.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), FLIP, 70);
                    (team.members[0], team.members[2], team.members[5]) = (1, 1 << 2, 1 << 5);
                }
                if shape & 4 != 0 {
                    team.skills[1] = clocked(TriggerType::StarNotes, 1, FLIP, 5);
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
                if shape & 128 != 0 {
                    team.accessories[1] =
                        AccessorySkill { effect: SkillEffect::AppealBoost, ..worn(FLIP, BOOST) };
                    (team.durations[1][1], team.targets[1][1], team.boosts[1][1]) = (700, 0b11, 50);
                }
                match lasting {
                    0 => {}
                    1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                    _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                }

                let mut ghost = team;
                ghost.accessories[3] =
                    AccessorySkill { effect: SkillEffect::Amplify, ..worn(FLIP, 4) };
                let mut cold = team;
                cold.accessories[3] = worn(FLIP, 0);
                let mut landing = ghost;
                amp(&mut landing, 8, TriggerType::RhythmIcons, 6, ALWAYS, 4);
                flat(&mut landing);
                let mut idle = ghost;
                idle.skills[8] = scorer(6, ALWAYS, 0);
                let mut crossed = cold;
                crossed.skills[8] = scorer(6, ALWAYS, 0);

                let triggers = Triggers::build(&queue, &landing);
                assert_eq!(triggers.amp(), (1 << 3) | (1 << 8));
                assert_eq!(Triggers::build(&queue, &cold).amp(), 0);
                assert_eq!(triggers.score() != 0, shape & 1 != 0);
                assert_eq!(triggers.year() != 0, shape & 2 != 0);
                assert_eq!(!triggers.stars().is_empty(), shape & 4 != 0);
                assert_eq!(triggers.heals() != 0, shape & 8 != 0);
                assert_eq!(triggers.fever() != 0, shape & 16 != 0);
                assert_eq!(triggers.psu() != 0, shape & 32 != 0);
                assert_eq!(triggers.sru() != 0, shape & 64 != 0);
                assert_eq!(triggers.param() != 0, shape & 128 != 0);
                assert_eq!(triggers.tricks() != 0, lasting == 2);

                for accuracy in [ALWAYS, 8000, 0] {
                    for &seed in &amp_first {
                        let config = sloppy(accuracy, seed);
                        let what = format!("shape {shape}, lasting {lasting}, seed {seed}");
                        let out = at(&queue, &ghost, &config);
                        assert_eq!(out, at(&queue, &cold, &config), "{what}");
                        let landed = at(&queue, &landing, &config);
                        assert_eq!(landed, at(&queue, &idle, &config), "{what}");
                        assert_eq!(landed, at(&queue, &crossed, &config), "{what}");
                    }
                }
                copies += 1;
            }
        }
        assert_eq!(copies, 768);
    }

    #[test]
    fn check_coinflip_others_first() {
        let heads = (0..4096).filter(|&s| others_first(s)).count();
        assert!((1900..2200).contains(&heads), "{heads}");
    }
}
