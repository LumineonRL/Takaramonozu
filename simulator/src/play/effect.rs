//! Skill effects. What a skill does once it activates.

/// Implements each named `Lasting` method by handing it to the wrapped table, `self.lasting`.
macro_rules! forward {
    ($($method:ident),+ $(,)?) => {
        $(forward!(@ $method);)+
    };
    (@ admit) => { forward!(fn admit(&mut self, mask: u16, now: crate::song::Time) -> u16); };
    (@ next) => { forward!(fn next(&self) -> crate::song::Time); };
    (@ release) => { forward!(fn release(&mut self, at: crate::song::Time) -> u16); };
    (@ converted) => { forward!(fn converted(&self, now: crate::song::Time) -> bool); };
    (@ locked) => { forward!(fn locked(&self, now: crate::song::Time) -> bool); };
    (@ lock) => {
        forward!(fn lock(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms
        ));
    };
    (@ trick) => {
        forward!(fn trick(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms
        ));
    };
    (@ occupy) => {
        forward!(fn occupy(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms
        ) -> crate::song::Time);
    };
    (@ fever) => {
        forward!(fn fever(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32,
            notes: u32
        ));
    };
    (@ fevered) => { forward!(fn fevered(&mut self, now: crate::song::Time, combo: u32) -> u64); };
    (@ read_running) => { forward!(fn read_running(&mut self)); };
    (@ fever_paid) => { forward!(fn fever_paid(&self) -> u64); };
    (@ psu) => {
        forward!(fn psu(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32,
            perfects: u32
        ));
    };
    (@ expire) => { forward!(fn expire(&mut self, now: crate::song::Time, perfects: u32)); };
    (@ psu_tap) => { forward!(fn psu_tap(&self, perfect: bool) -> u64); };
    (@ psu_paid) => { forward!(fn psu_paid(&self, perfects: u32) -> u64); };
    (@ tap) => {
        forward!(fn tap(
            &self, now: crate::song::Time, head: bool, tail: bool, combo: u16, lane: u16, note: u16
        ) -> u64);
    };
    (@ worth) => {
        forward!(fn worth(&self, now: crate::song::Time, head: bool, tail: bool) -> u64);
    };
    (@ heal) => { forward!(fn heal(&mut self, hp: u8)); };
    (@ overheal_strength) => { forward!(fn overheal_strength(&self, locked: bool) -> u64); };
    (@ begin) => { forward!(fn begin(&mut self)); };
    (@ boosted) => { forward!(fn boosted(&self, now: crate::song::Time) -> bool); };
    (@ rates) => { forward!(fn rates(&self, lane: u32) -> [crate::scalar::Rate; 2]); };
    (@ sru) => {
        forward!(fn sru(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32
        ));
    };
    (@ heart_multiplier) => { forward!(fn heart_multiplier(&self) -> u16); };
    (@ strength) => { forward!(fn strength(&self, locked: bool) -> u32); };
    (@ hearted_strength) => { forward!(fn hearted_strength(&self, locked: bool) -> u64); };
    (@ param) => {
        forward!(fn param(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32
        ));
    };
    (fn $name:ident(&self $(, $arg:ident: $ty:ty)* $(,)?) -> $ret:ty) => {
        #[inline(always)]
        fn $name(&self $(, $arg: $ty)*) -> $ret {
            self.lasting.$name($($arg),*)
        }
    };
    (fn $name:ident(&mut self $(, $arg:ident: $ty:ty)* $(,)?) -> $ret:ty) => {
        #[inline(always)]
        fn $name(&mut self $(, $arg: $ty)*) -> $ret {
            self.lasting.$name($($arg),*)
        }
    };
    (fn $name:ident(&mut self $(, $arg:ident: $ty:ty)* $(,)?)) => {
        #[inline(always)]
        fn $name(&mut self $(, $arg: $ty)*) {
            self.lasting.$name($($arg),*);
        }
    };
}

pub(super) mod fever;
pub(super) mod healing;
pub(super) mod locks;
pub(super) mod param;
pub(super) mod psu;
pub(super) mod sru;

use super::queue::Queue;
use super::tap::HEART_UNIT;
use super::{Outcome, Scoring};
use crate::scalar::{Centi, Ms, Rate};
use crate::skill::SkillEffect;
use crate::song::Time;

/// No lane is waiting to roll.
const NEVER: Time = Time(u32::MAX);

/// For Duration effects. Perfect Lock, Total Trick, Combo Fever, Perfect Score Up, Skill Rate Up, and Appeal Boost
pub(in crate::play) trait Lasting<'q> {
    /// Whether anything on the team is active.
    const ANY: bool;
    /// Whether a Total Trick may run
    const TRICKS: bool;

    const HEARTS: bool = false;

    const FEVER: bool = false;

    const PSU: bool = false;

    const PARAM: bool = false;

    /// `last` is the final judgement aka end of song.
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self;

    /// So a PL can't activate if that lane is already active. Instead it holds the
    /// activation until the active on expires.
    /// `admit` is for scheduling those activations that happen mid-duration
    /// `next` holds the time the lane rolls again.
    /// `release` holds the time when the duration ends.
    fn admit(&mut self, mask: u16, now: Time) -> u16;
    fn next(&self) -> Time;
    fn release(&mut self, at: Time) -> u16;

    /// A Perfect Lock or a Total Trick is running. Every note is a PERFECT, and the Trick SIS can work.
    fn converted(&self, now: Time) -> bool;
    /// A Perfect Lock is running. True PERFECTs score 1.08x.
    fn locked(&self, now: Time) -> bool;
    fn lock(&mut self, lane: u32, now: Time, duration: Ms);
    fn trick(&mut self, lane: u32, now: Time, duration: Ms);

    /// This is another mechanic that apparently does not work as I had remembered it working but
    /// the docs say otherwise.
    /// Lanes get blocked for a duration when a duration skills is active (I thought they would just overlap)
    /// so this is used by duration skills to schedule when the lane is available for timed effects.
    fn occupy(&mut self, _lane: u32, now: Time, _duration: Ms) -> Time {
        debug_assert!(false, "no timed skills on this team, buddy.");
        now
    }

    fn fever(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32, _notes: u32) {
        debug_assert!(false, "no combo fever on this team, buddy.");
    }

    fn fevered(&mut self, _now: Time, _combo: u32) -> u64 {
        debug_assert!(false, "no combo fever on this team, buddy.");
        0
    }

    /// Flips to reading fever note-by-note.
    fn read_running(&mut self) {}

    /// Total fever bonus for the whole song.  For teams that never look at the running score.
    fn fever_paid(&self) -> u64 {
        debug_assert!(false, "a trial paid as if a Combo Fever could run");
        0
    }

    fn psu(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32, _perfects: u32) {
        debug_assert!(false, "no PSU on this team, buddy.");
    }

    /// Specifically for PSU. TODO rename to make clear?
    fn expire(&mut self, _now: Time, _perfects: u32) {
        debug_assert!(false, "no PSU on this team, buddy.");
    }

    /// Value of the PSU bonus
    fn psu_tap(&self, _perfect: bool) -> u64 {
        debug_assert!(false, "no PSU on this team, buddy.");
        0
    }

    /// Total PSU bonus for the whole song
    fn psu_paid(&self, _perfects: u32) -> u64 {
        0
    }

    /// PPN with tricks.
    fn tap(&self, _: Time, _: bool, _: bool, _: u16, _: u16, _: u16) -> u64 {
        debug_assert!(false, "a tap scored as if a Total Trick could run");
        0
    }

    /// A Total Trick team's strength times the judgement, before combo, lane and note.
    /// Why Total Trick you ask? Because profiling showed it was faster to make it its own thing.
    fn worth(&self, _: Time, _: bool, _: bool) -> u64 {
        debug_assert!(false, "value as if Total Trick could proc.");
        0
    }

    #[inline(always)]
    fn heal(&mut self, _hp: u8) {}

    /// Basically the "hot" team strength - the team strength with mid-trial effects applied like Trick and Overheal.
    /// other effects like param boost may get rolled into this too.
    /// name may need to change if everything does get rolled into here because the name implies it's just
    /// overheal but that's a tbd.
    fn overheal_strength(&self, _locked: bool) -> u64 {
        debug_assert!(false, "tap scored as if heart could be hearted.");
        0
    }

    /// For SRU since they have a weird locking mechanism where only one can be active at a time
    /// so this signals that an SRU is active and none others can active until it finishes.
    #[inline(always)]
    fn begin(&mut self) {}

    /// SRU currently active flags.
    #[inline(always)]
    fn boosted(&self, _now: Time) -> bool {
        false
    }

    /// card and acc activations rates while current SRU is active.
    fn rates(&self, _lane: u32) -> [Rate; 2] {
        debug_assert!(false, "no SRU on this team, buddy.");
        [0; 2]
    }

    fn sru(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32) {
        debug_assert!(false, "no SRU on this team, buddy.");
    }

    fn heart_multiplier(&self) -> u16 {
        debug_assert!(false, "no hearts on this team, buddy.");
        HEART_UNIT
    }

    /// Team strength with any Appeal Boost running and the Trick if `locked`.
    fn strength(&self, _locked: bool) -> u32 {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
        0
    }

    /// `strength` * heart multiplier.
    fn hearted_strength(&self, _locked: bool) -> u64 {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
        0
    }

    fn param(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32) {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
    }
}

pub(in crate::play) struct Instant;

impl Lasting<'_> for Instant {
    const ANY: bool = false;
    const TRICKS: bool = false;

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn new(_: Time, _: &Queue<'_>, _: &Scoring) -> Self {
        Self
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn admit(&mut self, mask: u16, _: Time) -> u16 {
        mask
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn next(&self) -> Time {
        NEVER
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn release(&mut self, _: Time) -> u16 {
        0
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn converted(&self, _: Time) -> bool {
        false
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn locked(&self, _: Time) -> bool {
        false
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn lock(&mut self, _: u32, _: Time, _: Ms) {
        debug_assert!(false, "A PL did not schedule an expiration event.");
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn trick(&mut self, _: u32, _: Time, _: Ms) {
        debug_assert!(false, "A Total Trick did not schedule an expiration event.");
    }
}

/// TODO: Sync. Maybe Spark too.
pub(super) const fn lasts(effect: SkillEffect) -> bool {
    matches!(
        effect,
        SkillEffect::PerfectLock
            | SkillEffect::TotalTrick
            | SkillEffect::ComboFever
            | SkillEffect::PerfectScoreUp
            | SkillEffect::SkillRateUp
            | SkillEffect::AppealBoost
    )
}

/// TODO: every other effect.
#[inline(always)]
#[expect(
    clippy::too_many_arguments,
    reason = "I like difficult to read code. (just kidding. Maybe can try to break it up as a todo)"
)]
pub(super) fn pay<'q>(
    lane: u32,
    effect: SkillEffect,
    value: Centi,
    duration: Ms,
    hp: u8,
    now: Time,
    out: &mut Outcome,
    expiry: &mut impl Lasting<'q>,
) {
    // Select was faster than a match arm for Heal SIS.
    let paid = if matches!(effect, SkillEffect::Scorer | SkillEffect::Healer) {
        value
    } else {
        0
    };
    out.skill = out.skill.saturating_add(u64::from(paid));
    match effect {
        SkillEffect::PerfectLock => expiry.lock(lane, now, duration),
        SkillEffect::TotalTrick => expiry.trick(lane, now, duration),
        SkillEffect::Healer => expiry.heal(hp),
        SkillEffect::ComboFever => expiry.fever(lane, now, duration, value, out.notes),
        SkillEffect::PerfectScoreUp => expiry.psu(lane, now, duration, value, out.perfects),
        SkillEffect::SkillRateUp => expiry.sru(lane, now, duration, value),
        SkillEffect::AppealBoost => expiry.param(lane, now, duration, value),
        SkillEffect::Scorer
        | SkillEffect::Encore
        | SkillEffect::Amplify
        | SkillEffect::Sync
        | SkillEffect::Spark => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::play::activation::{AccessorySkill, Skill};
    use crate::play::queue::Queue;
    use crate::play::tests::{fifty_one, only, scorer, seeded, worn};
    use crate::play::{Outcome, Scoring};
    use crate::scalar::{Ms, RATE_UNIT, Rate};
    use crate::skill::{SkillEffect, TriggerType};

    #[expect(clippy::integer_division, reason = "a coin flip")]
    pub(super) const FLIP: Rate = RATE_UNIT / 2;

    pub(super) const TEAM: u32 = 80_000;

    /// All notes of a 51 note-map with PL active.
    pub(super) const ALL_LOCKED: u64 = 50 * 1080 + 1188;

    pub(super) fn clocked(trigger: TriggerType, threshold: u16, chance: Rate, value: u32) -> Skill {
        Skill { trigger, ..scorer(threshold, chance, value) }
    }

    pub(super) fn lasting(
        scoring: &mut Scoring,
        lane: usize,
        effect: SkillEffect,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
    ) {
        scoring.skills[lane] = Skill { effect, ..clocked(trigger, threshold, chance, 0) };
        scoring.durations[lane][0] = duration;
    }

    pub(super) fn lock(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::PerfectLock, trigger, threshold, chance, duration);
    }

    pub(super) fn trick(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::TotalTrick, trigger, threshold, chance, duration);
    }

    /// Fixture heart worth +10%. Twenty of them would be +200% and 200% is the cap.
    pub(super) fn overhealing(scoring: &mut Scoring, max_hp: u8) {
        (scoring.max_hp, scoring.per_heart, scoring.max_hearts) = (max_hp, 1000, 20);
    }

    pub(super) fn healer(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        hp: u8,
    ) {
        scoring.skills[lane] = Skill {
            effect: SkillEffect::Healer,
            ..clocked(trigger, threshold, chance, 0)
        };
        scoring.heals[lane][0] = hp;
    }

    /// Every icon heals 10 = 1 full HP bar.
    /// Each bar grants a heart.
    /// Icon 10 spawns at 2300, on note 4's tap
    /// So the n-th heart applies from note 10n - 5.
    pub(super) const ONE_HEART_EVERY_TEN: u64 =
        4 * 1000 + 10 * 1100 + 10 * 1200 + 10 * 1300 + 10 * 1400 + 6 * 1500 + 1650;

    pub(super) fn fever(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        value: u32,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::ComboFever, trigger, threshold, chance, duration);
        scoring.skills[lane].value = value;
    }

    pub(super) fn ladder(scoring: &mut Scoring, lane: usize) {
        scoring.skills[lane] = clocked(TriggerType::Score, 1500, 0, 0);
    }

    pub(super) fn psu(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        value: u32,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::PerfectScoreUp, trigger, threshold, chance, duration);
        scoring.skills[lane].value = value;
    }

    pub(super) fn param(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        value: u32,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::AppealBoost, trigger, threshold, chance, duration);
        scoring.skills[lane].value = value;
    }

    /// Half of `TEAM`.
    pub(super) const BOOST: u32 = 40_000;

    /// 480 * heal value
    #[test]
    fn check_heal_sis() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut healed = Scoring::flat(TEAM);
        healed.skills[3] = Skill { effect: SkillEffect::Healer, ..scorer(5, FLIP, 4 * 480) };
        healed.accessories[3] =
            AccessorySkill { effect: SkillEffect::Healer, ..worn(FLIP, 8 * 480) };
        let mut scored = healed;
        scored.skills[3].effect = SkillEffect::Scorer;
        scored.accessories[3].effect = SkillEffect::Scorer;

        let mut seen = [0; 2];
        for seed in 0..32 {
            let out = seeded(&queue, &healed, seed);
            assert_eq!(out, seeded(&queue, &scored, seed), "seed {seed}");
            let want = out.activations * 4 * 480 + out.accessory_activations * 8 * 480;
            assert_eq!(out.skill, u64::from(want), "seed {seed}");
            seen[0] += out.activations;
            seen[1] += out.accessory_activations;
        }
        assert!(seen[0] > 0 && seen[1] > 0, "{seen:?}");

        // No Heal SIS.
        healed.skills[3].value = 0;
        healed.accessories[3].value = 0;
        let bare = seeded(&queue, &healed, 0);
        assert_eq!(bare, Outcome { skill: 0, ..seeded(&queue, &scored, 0) });
        assert!(bare.activations > 0);
    }
}
