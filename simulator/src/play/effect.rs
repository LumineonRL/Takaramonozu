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
    (@ lock_land) => {
        forward!(fn lock_land(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms
        ));
    };
    (@ total_trick_land) => {
        forward!(fn total_trick_land(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms
        ));
    };
    (@ occupy) => {
        forward!(fn occupy(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms
        ) -> crate::song::Time);
    };
    (@ fever_land) => {
        forward!(fn fever_land(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32,
            notes: u32
        ));
    };
    (@ fever_tap) => { forward!(fn fever_tap(&mut self, now: crate::song::Time, combo: u32) -> u64); };
    (@ fever_per_note) => { forward!(fn fever_per_note(&mut self)); };
    (@ fever_paid) => { forward!(fn fever_paid(&self) -> u64); };
    (@ psu_land) => {
        forward!(fn psu_land(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32,
            perfects: u32
        ));
    };
    (@ expire) => { forward!(fn expire(&mut self, now: crate::song::Time, perfects: u32)); };
    (@ psu_tap) => { forward!(fn psu_tap(&self, perfect: bool) -> u64); };
    (@ psu_paid) => { forward!(fn psu_paid(&self, perfects: u32) -> u64); };
    (@ total_trick_tap) => {
        forward!(fn total_trick_tap(
            &self, now: crate::song::Time, head: bool, tail: bool, scale: u32, lane: u16
        ) -> u64);
    };
    (@ total_trick_worth) => {
        forward!(fn total_trick_worth(&self, now: crate::song::Time, head: bool, tail: bool) -> u64);
    };
    (@ healer_land) => { forward!(fn healer_land(&mut self, hp: u8)); };
    (@ heart_strength) => { forward!(fn heart_strength(&self, locked: bool) -> u64); };
    (@ begin) => { forward!(fn begin(&mut self)); };
    (@ sru_boosted) => { forward!(fn sru_boosted(&self, now: crate::song::Time) -> bool); };
    (@ sru_rates) => { forward!(fn sru_rates(&self, lane: u32) -> [crate::scalar::Rate; 2]); };
    (@ sru_land) => {
        forward!(fn sru_land(
            &mut self, lane: u32, now: crate::song::Time, duration: crate::scalar::Ms, value: u32
        ));
    };
    (@ heart_multiplier) => { forward!(fn heart_multiplier(&self) -> u16); };
    (@ param_strength) => { forward!(fn param_strength(&self, locked: bool) -> u32); };
    (@ param_hearted_strength) => { forward!(fn param_hearted_strength(&self, locked: bool) -> u64); };
    (@ param_land) => {
        forward!(fn param_land(
            &mut self, lane: u32, worn: bool, now: crate::song::Time, duration: crate::scalar::Ms,
            value: u32
        ));
    };
    (@ sync_land) => {
        forward!(fn sync_land(
            &mut self, lane: u32, worn: bool, now: crate::song::Time, duration: crate::scalar::Ms
        ) -> u16);
    };
    (@ param_land_at) => {
        forward!(fn param_land_at(
            &mut self, lane: u32, worn: bool, now: crate::song::Time, duration: crate::scalar::Ms,
            value: u32, percent: u16
        ));
    };
    (@ seed) => { forward!(fn seed(&mut self, seed: u64)); };
    (@ encore_param) => {
        forward!(fn encore_param(
            &mut self, hold: u32, from: crate::play::effect::encore::Copied,
            now: crate::song::Time, payload: crate::play::activation::Payload
        ));
    };
    (@ encore_sync) => {
        forward!(fn encore_sync(
            &mut self, hold: u32, targets: u16, now: crate::song::Time, duration: crate::scalar::Ms
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

pub(super) mod amp;
pub(super) mod encore;
pub(super) mod fever;
pub(super) mod healing;
pub(super) mod locks;
pub(super) mod param;
pub(super) mod psu;
pub(super) mod spark;
pub(super) mod sru;

use super::activation::Payload;
use super::queue::Queue;
use super::tap::HEART_UNIT;
use super::{Outcome, Scoring};
use crate::scalar::{Centi, Ms, Rate};
use crate::skill::SkillEffect;
use crate::song::{LANES, Time};
use encore::Copied;

/// No lane is waiting to roll.
const NEVER: Time = Time(u32::MAX);

/// Identifies which lanes have finished their duration effect
#[inline(always)]
fn ended(until: &[Time; LANES], now: Time) -> u16 {
    let mut lanes = 0;
    for (lane, &end) in until.iter().enumerate() {
        lanes |= u16::from(end <= now) << lane;
    }
    lanes
}

/// Finds the lane whose effect will expire next. This is only for duration effects.
#[inline(always)]
fn earliest(until: &[Time; LANES], lanes: u16, now: Time) -> Time {
    let start = now.0.wrapping_add(1);
    let gaps: [u32; LANES] = core::array::from_fn(|lane| match until.get(lane) {
        Some(end) if lanes & (1 << lane) != 0 => end.0.wrapping_sub(start),
        _ => u32::MAX,
    });
    // Performance hacking magic. This was originally a loop but the LLVM did awful at vectorizing it.
    // This specific shape with the manually-implemented tree reduction
    // allowed for some baller SIMD instructions on AVX-512 I don't know how it does on other chips.
    // I know these look like they're prefaced with ones (ten, eleven, twelve...), but they're actually lower-case ls.
    // So lane 0 through lane 8.
    let [l0, l1, l2, l3, l4, l5, l6, l7, l8] = gaps;
    let [l0, l1, l2, l3] = [l0.min(l4), l1.min(l5), l2.min(l6), l3.min(l7)];
    let [l0, l1] = [l0.min(l2), l1.min(l3)];
    // except there's no clean way to tree reduce with 9 elements so we just manually handle the leftover lane.
    let next = Time(l0.min(l1).min(l8).wrapping_add(start));
    if next > now { next } else { NEVER }
}

/// For Duration effects. Perfect Lock, Total Trick, Combo Fever, Perfect Score Up, Skill Rate Up, Appeal Boost and Sync.
///
/// A method only one skill uses is prefixed with that skill (`lock_`, `total_trick_`, `healer_`, `heart_`,
/// `fever_`, `psu_`, `sru_`, `param_`, `sync_`, `amp_`, `encore_`, `spark_`), and its landing is `<skill>_land`.
/// The unprefixed ones (`admit`, `next`, `release`, `occupy`, `expire`, `converted`, `locked`, `begin`, `end`,
/// `seed`) are shared.
pub(in crate::play) trait Lasting<'q> {
    /// Whether anything on the team is active.
    const ANY: bool;
    /// Whether a Total Trick may run
    const TRICKS: bool;

    const HEARTS: bool = false;

    const FEVER: bool = false;

    const PSU: bool = false;

    /// Both param boost and sync
    const PARAM: bool = false;

    const AMP: bool = false;

    const REPEAT: bool = false;

    const SPARK: bool = false;

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
    fn lock_land(&mut self, lane: u32, now: Time, duration: Ms);
    fn total_trick_land(&mut self, lane: u32, now: Time, duration: Ms);

    /// This is another mechanic that apparently does not work as I had remembered it working but
    /// the docs say otherwise.
    /// Lanes get blocked for a duration when a duration skills is active (I thought they would just overlap)
    /// so this is used by duration skills to schedule when the lane is available for timed effects.
    fn occupy(&mut self, _lane: u32, now: Time, _duration: Ms) -> Time {
        debug_assert!(false, "no timed skills on this team, buddy.");
        now
    }

    fn fever_land(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32, _notes: u32) {
        debug_assert!(false, "no combo fever on this team, buddy.");
    }

    fn fever_tap(&mut self, _now: Time, _combo: u32) -> u64 {
        debug_assert!(false, "no combo fever on this team, buddy.");
        0
    }

    /// Flips to reading fever note-by-note.
    fn fever_per_note(&mut self) {}

    /// Total fever bonus for the whole song.  For teams that never look at the running score.
    fn fever_paid(&self) -> u64 {
        debug_assert!(false, "a trial paid as if a Combo Fever could run");
        0
    }

    fn psu_land(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32, _perfects: u32) {
        debug_assert!(false, "no PSU on this team, buddy.");
    }

    /// Ends the PSU, Appeal Boost and Sync windows that are over.
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
    fn total_trick_tap(&self, _: Time, _: bool, _: bool, _: u32, _: u16) -> u64 {
        debug_assert!(false, "a tap scored as if a Total Trick could run");
        0
    }

    /// A Total Trick team's strength times the judgement, before combo, lane and note.
    /// Why Total Trick you ask? Because profiling showed it was faster to make it its own thing.
    fn total_trick_worth(&self, _: Time, _: bool, _: bool) -> u64 {
        debug_assert!(false, "value as if Total Trick could proc.");
        0
    }

    #[inline(always)]
    fn healer_land(&mut self, _hp: u8) {}

    /// Basically the "hot" team strength - the team strength with mid-trial effects applied like Trick and Overheal.
    /// other effects like param boost may get rolled into this too.
    /// name may need to change if everything does get rolled into here because the name implies it's just
    /// overheal but that's a tbd.
    fn heart_strength(&self, _locked: bool) -> u64 {
        debug_assert!(false, "tap scored as if heart could be hearted.");
        0
    }

    /// For SRU since they have a weird locking mechanism where only one can be active at a time
    /// so this signals that an SRU is active and none others can active until it finishes.
    #[inline(always)]
    fn begin(&mut self) {}

    /// End of one specific moment of tick or step or whatever you want to call it.
    /// It's for determining if an Amp is active or not.
    #[inline(always)]
    fn end(&mut self) {}

    /// SRU currently active flags.
    #[inline(always)]
    fn sru_boosted(&self, _now: Time) -> bool {
        false
    }

    /// card and acc activations rates while current SRU is active.
    fn sru_rates(&self, _lane: u32) -> [Rate; 2] {
        debug_assert!(false, "no SRU on this team, buddy.");
        [0; 2]
    }

    fn sru_land(&mut self, _lane: u32, _now: Time, _duration: Ms, _value: u32) {
        debug_assert!(false, "no SRU on this team, buddy.");
    }

    fn heart_multiplier(&self) -> u16 {
        debug_assert!(false, "no hearts on this team, buddy.");
        HEART_UNIT
    }

    /// Team strength with any Appeal Boost running and the Trick if `locked`.
    fn param_strength(&self, _locked: bool) -> u32 {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
        0
    }

    /// `param_strength` * heart multiplier.
    fn param_hearted_strength(&self, _locked: bool) -> u64 {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
        0
    }

    /// `worn` just means the accessory rolled and the card didn't.
    fn param_land(&mut self, _lane: u32, _worn: bool, _now: Time, _duration: Ms, _value: u32) {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
    }

    /// Bit of the lane it copied (or 0 if nothing to copy)
    fn sync_land(&mut self, _lane: u32, _worn: bool, _now: Time, _duration: Ms) -> u16 {
        debug_assert!(false, "no Sync on this team, buddy.");
        0
    }

    /// `param_land` at the percent an Amplify raised it to.
    fn param_land_at(
        &mut self,
        _lane: u32,
        _worn: bool,
        _now: Time,
        _duration: Ms,
        _value: u32,
        _percent: u16,
    ) {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
    }

    /// Sync gets its own RNG stream, and so does the Amplify coin flip.
    #[inline(always)]
    fn seed(&mut self, _seed: u64) {}

    /// For lanes that are going off at the exact same time, this orders them in the order they're set to go off.
    /// Depending on how many there are it uses different bits.
    /// the first group's go in bits 32..48
    /// the second's in 16..32
    /// the last's in 0..16
    /// Each of these represents a right to left activation.
    fn amp_order(&self, mask: u16) -> u64 {
        debug_assert!(false, "no Amplify on this team, buddy.");
        u64::from(mask)
    }

    /// For amp, spends an active amp.
    fn amp_spend(&mut self, _take: bool) -> u32 {
        debug_assert!(false, "no Amplify on this team, buddy.");
        0
    }

    /// This is what's in charge of bumping the skill level itself for an amp.
    fn amp_payload(&self, _lane: u32, _worn: bool, _levels: u32, _scoring: &Scoring) -> Payload {
        debug_assert!(false, "no Amplify on this team, buddy.");
        Payload::default()
    }

    fn amp_land(&mut self, _levels: u32) {
        debug_assert!(false, "no Amplify on this team, buddy.");
    }

    /// The first skill to proc at a givenl moment that isn't Encore
    fn encore_write(&mut self, _landed: Copied) {
        debug_assert!(false, "no Encore on this team, buddy.");
    }

    /// Reads the skill written by `encore_write`.
    fn encore_copy(&mut self) -> Option<Copied> {
        debug_assert!(false, "no Encore on this team, buddy.");
        None
    }

    /// Trying to track down the interactions between Encore and Param Boost / Sync was a nightmare.
    /// I feel quite confident that what's modeled here is true to the game and sifcord Discord logs
    /// and my own testing seem to support it.
    ///
    /// Encore's copy of Param Boost.
    /// Boosts the original's targets, blocks the lane aka `hold`.
    fn encore_param(&mut self, _hold: u32, _from: Copied, _now: Time, _payload: Payload) {
        debug_assert!(false, "no Appeal Boost on this team, buddy.");
    }

    /// Encore's copy of a Sync.
    /// Here,`hold` copies one of `targets` (the lane that the original copied).
    fn encore_sync(&mut self, _hold: u32, _targets: u16, _now: Time, _duration: Ms) {
        debug_assert!(false, "no Sync on this team, buddy.");
    }

    /// For Encore+Sync. It's the bit of the lane Sync last copied (or 0 if no valid target).
    /// Encore needs to keep a copy of this in its slot else it'd have no way to resolve which was picked (hence the name).
    fn encore_picked(&self, _lane: u32) -> u16 {
        debug_assert!(false, "no Encore on this team, buddy.");
        0
    }

    /// Have enough charges of spark stored up so that the skill could activate.
    fn spark_ready(&self, _threshold: u8) -> bool {
        debug_assert!(false, "no Spark on this team, buddy.");
        true
    }

    /// Spends spark chages.
    /// Going to document this since it was something I was unsure of, but as many
    /// charges that can be spent get spent. If you have 17 charges and the Spark skill needs 5,
    /// it will spend 15 charges.
    fn spark_spend(&mut self, _threshold: u8) -> u32 {
        debug_assert!(false, "no Spark on this team, buddy.");
        0
    }

    /// There's some nuance to what actually banks a spark. See the top of `spark.rs`.
    fn spark_bank(&mut self, _lane: u32, _effect: SkillEffect) {
        debug_assert!(false, "no Spark on this team, buddy.");
    }

    fn spark_land(&mut self, _lane: u32, _now: Time, _duration: Ms, _bonus: u32, _notes: u32) {
        debug_assert!(false, "no Spark on this team, buddy.");
    }

    fn spark_tap(&mut self, _now: Time) -> u64 {
        debug_assert!(false, "no Spark on this team, buddy.");
        0
    }

    fn spark_per_note(&mut self) {}

    fn spark_paid(&self) -> u64 {
        debug_assert!(false, "no Spark on this team, buddy.");
        0
    }
}

pub(in crate::play) struct Instant;

impl Lasting<'_> for Instant {
    const ANY: bool = false;
    const TRICKS: bool = false;

    #[inline(always)]
    fn new(_: Time, _: &Queue<'_>, _: &Scoring) -> Self {
        Self
    }

    #[inline(always)]
    fn admit(&mut self, mask: u16, _: Time) -> u16 {
        mask
    }

    #[inline(always)]
    fn next(&self) -> Time {
        NEVER
    }

    #[inline(always)]
    fn release(&mut self, _: Time) -> u16 {
        0
    }

    #[inline(always)]
    fn converted(&self, _: Time) -> bool {
        false
    }

    #[inline(always)]
    fn locked(&self, _: Time) -> bool {
        false
    }

    #[inline(always)]
    fn lock_land(&mut self, _: u32, _: Time, _: Ms) {
        debug_assert!(false, "A PL did not schedule an expiration event.");
    }

    #[inline(always)]
    fn total_trick_land(&mut self, _: u32, _: Time, _: Ms) {
        debug_assert!(false, "A Total Trick did not schedule an expiration event.");
    }
}

pub(super) const fn lasts(effect: SkillEffect) -> bool {
    matches!(
        effect,
        SkillEffect::PerfectLock
            | SkillEffect::TotalTrick
            | SkillEffect::ComboFever
            | SkillEffect::PerfectScoreUp
            | SkillEffect::SkillRateUp
            | SkillEffect::AppealBoost
            | SkillEffect::Sync
            | SkillEffect::Spark
    )
}

#[inline(always)]
#[expect(
    clippy::too_many_arguments,
    reason = "I like difficult to read code. (just kidding. Maybe can try to break it up as a todo)"
)]
pub(super) fn pay<'q>(
    lane: u32,
    worn: bool,
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
        SkillEffect::PerfectLock => expiry.lock_land(lane, now, duration),
        SkillEffect::TotalTrick => expiry.total_trick_land(lane, now, duration),
        SkillEffect::Healer => expiry.healer_land(hp),
        SkillEffect::ComboFever => expiry.fever_land(lane, now, duration, value, out.notes),
        SkillEffect::PerfectScoreUp => expiry.psu_land(lane, now, duration, value, out.perfects),
        SkillEffect::SkillRateUp => expiry.sru_land(lane, now, duration, value),
        SkillEffect::AppealBoost => expiry.param_land(lane, worn, now, duration, value),
        SkillEffect::Sync => {
            expiry.sync_land(lane, worn, now, duration);
        }
        SkillEffect::Scorer | SkillEffect::Encore | SkillEffect::Amplify | SkillEffect::Spark => {}
    }
}

/// Specifically for teams with Amplify, Encore or Spark. No amp means we don't worry about this (actually that applies to nearly every skill
/// in this repo).
/// This processes an Amp activation so the increased skill level reading the enhanced value/duration
/// and a skill check because Encore does not consume Amp.
#[inline(always)]
pub(super) fn land<'q, E>(
    lane: u32,
    worn: bool,
    effect: SkillEffect,
    now: Time,
    scoring: &Scoring,
    out: &mut Outcome,
    expiry: &mut E,
) where
    E: Lasting<'q>,
{
    let from = if E::REPEAT && effect == SkillEffect::Encore {
        let Some(copy) = expiry.encore_copy() else {
            return;
        };
        copy
    } else {
        let levels = expiry.amp_spend(effect != SkillEffect::Encore);
        let landed = Copied { lane, worn, effect, levels };
        expiry.encore_write(landed);
        landed
    };
    let payload = expiry.amp_payload(from.lane, from.worn, from.levels, scoring);
    // Spark, (or I guess also a copy of one because Encore can do that)
    // It activates for a multiple of its threshold times how many charges get spent.
    let value = if E::SPARK && from.effect == SkillEffect::Spark {
        let threshold = scoring.sparks.get(from.lane as usize).copied().unwrap_or(0);
        payload.value.saturating_mul(expiry.spark_spend(threshold))
    } else {
        payload.value
    };
    // spark copies don't spend their own charges.
    if E::SPARK {
        expiry.spark_bank(lane, from.effect);
    }
    // If Param Boost or Sync are copied, they need to read the original's lane AND the the lane holding this.
    if E::REPEAT
        && effect == SkillEffect::Encore
        && matches!(from.effect, SkillEffect::AppealBoost | SkillEffect::Sync)
    {
        if from.effect == SkillEffect::AppealBoost {
            expiry.encore_param(lane, from, now, payload);
        } else {
            let targets = expiry.encore_picked(from.lane);
            expiry.encore_sync(lane, targets, now, payload.duration);
        }
        return;
    }
    let (duration, extra) = (payload.duration, payload.extra);
    pay_at(lane, worn, from.effect, value, duration, extra, now, out, expiry);
}

/// `extra` can either be Healer's HP or an Appeal Boost's percent I wasn't sure what to do with them.
#[inline(always)]
#[expect(clippy::too_many_arguments, reason = "agreed")]
fn pay_at<'q>(
    lane: u32,
    worn: bool,
    effect: SkillEffect,
    value: Centi,
    duration: Ms,
    extra: u16,
    now: Time,
    out: &mut Outcome,
    expiry: &mut impl Lasting<'q>,
) {
    let paid = if matches!(effect, SkillEffect::Scorer | SkillEffect::Healer) {
        value
    } else {
        0
    };
    out.skill = out.skill.saturating_add(u64::from(paid));
    match effect {
        SkillEffect::PerfectLock => expiry.lock_land(lane, now, duration),
        SkillEffect::TotalTrick => expiry.total_trick_land(lane, now, duration),
        SkillEffect::Healer => expiry.healer_land(u8::try_from(extra).unwrap_or(u8::MAX)),
        SkillEffect::ComboFever => expiry.fever_land(lane, now, duration, value, out.notes),
        SkillEffect::PerfectScoreUp => expiry.psu_land(lane, now, duration, value, out.perfects),
        SkillEffect::SkillRateUp => expiry.sru_land(lane, now, duration, value),
        SkillEffect::AppealBoost => expiry.param_land_at(lane, worn, now, duration, value, extra),
        SkillEffect::Sync => {
            expiry.sync_land(lane, worn, now, duration);
        }
        SkillEffect::Amplify => expiry.amp_land(value),
        SkillEffect::Spark => expiry.spark_land(lane, now, duration, value, out.notes),
        SkillEffect::Scorer | SkillEffect::Encore => {}
    }
}

#[cfg(test)]
mod tests {
    use super::NEVER;
    use crate::play::activation::{AccessorySkill, Skill};
    use crate::play::queue::Queue;
    use crate::play::rng::Rng;
    use crate::play::tests::{fifty_one, only, scorer, seeded, worn};
    use crate::play::{Outcome, Scoring};
    use crate::scalar::{Ms, RATE_UNIT, Rate};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{LANES, Time};

    #[expect(clippy::integer_division, reason = "a coin flip")]
    pub(super) const FLIP: Rate = RATE_UNIT / 2;

    /// First `n` skill draws of a given seed based on a coin flip.
    pub(super) fn draws(seed: u64, n: usize) -> Vec<u64> {
        let mut rng = Rng::new(seed, Rng::SKILLS);
        core::iter::repeat_with(|| u64::from(rng.roll(FLIP))).take(n).collect()
    }

    /// Whether other skills go before Amplify or not
    pub(super) fn others_first(seed: u64) -> bool {
        Rng::new(seed, Rng::COIN).roll(FLIP)
    }

    pub(super) const TEAM: u32 = 80_000;

    /// All notes of a 51 note-map with PL active.
    pub(super) const ALL_LOCKED: u64 = 50 * 1080 + 1188;

    pub(super) fn clocked(trigger: TriggerType, threshold: u16, chance: Rate, value: u32) -> Skill {
        Skill { trigger, ..scorer(threshold, chance, value) }
    }

    pub(super) fn encore(threshold: u16, chance: Rate) -> Skill {
        Skill { effect: SkillEffect::Encore, ..scorer(threshold, chance, 0) }
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
    pub(super) const STATS: [u32; 9] = [8000, 24_000, 6000, 6000, 6000, 6000, 8000, 8000, 8000];

    /// `targets` are the lanes it may copy.
    pub(super) fn sync(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
        targets: u16,
    ) {
        lasting(scoring, lane, SkillEffect::Sync, trigger, threshold, chance, duration);
        scoring.targets[lane][0] = targets;
    }

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

    /// For the manually implemented binary tree reduction to find the smallest element in `earliest`
    #[test]
    fn check_scans() {
        const EDGES: [u32; 9] = [0, 1, 2, 499, 500, 501, u32::MAX - 2, u32::MAX - 1, u32::MAX];
        let mut rng = Rng::new(0, Rng::SKILLS);
        for _ in 0..256 {
            let until: [Time; LANES] = core::array::from_fn(|_| Time(EDGES[rng.below(9) as usize]));
            for now in EDGES.map(Time) {
                let ended =
                    until.iter().enumerate().fold(0, |m, (l, &t)| m | (u16::from(t <= now) << l));
                assert_eq!(super::ended(&until, now), ended);
                for lanes in 0..1 << LANES {
                    let live = |&(l, t): &(usize, &Time)| lanes & (1 << l) != 0 && *t > now;
                    let next = until.iter().enumerate().filter(live).map(|(_, &t)| t).min();
                    let got = super::earliest(&until, lanes, now);
                    assert_eq!(got, next.unwrap_or(NEVER), "{until:?} {lanes:#b} {now:?}");
                }
            }
        }
    }
}
