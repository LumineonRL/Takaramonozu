//! Skill effects. What a skill does once it activates.

use super::tap::{HEART_CAP, HEART_UNIT, Worth, overheal_score_val, worth_score};
use super::{Outcome, Scoring};
use crate::scalar::{Centi, Ms};
use crate::skill::SkillEffect;
use crate::song::{LANES, Time};

/// No lane is waiting to roll.
const NEVER: Time = Time(u32::MAX);

// For Duration effects. For now, just Perfect Lock and Total Trick, but SRU, CBU, etc will
/// likely utilize this too.
pub(in crate::play) trait Lasting {
    /// Whether anything on the team is active.
    const ANY: bool;
    /// Whether a Total Trick may run
    const TRICKS: bool;

    const HEARTS: bool = false;

    /// `last` is the final judgement aka end of song.
    fn new(last: Time, scoring: &Scoring) -> Self;

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
}

pub(in crate::play) struct Instant;

impl Lasting for Instant {
    const ANY: bool = false;
    const TRICKS: bool = false;

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn new(_: Time, _: &Scoring) -> Self {
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

/// Schedules when a duration effect expires.
/// There's an interesting mechanic where  a skill with a duration blocks
/// its own lane, card and accessory both, until it runs out.
pub(in crate::play) struct Expiry {
    until: [Time; LANES],
    /// Lanes whose `until` hasn't been seen to pass.
    open: u16,
    /// Lanes that roll at their `until`.
    held: u16,
    /// The earliest `until` in `held`.
    next: Time,
    last: Time,
    lock: Time,
}

impl Expiry {
    #[expect(clippy::indexing_slicing, reason = "`open` holds lane bits")]
    fn hold(&mut self, open: u16, now: Time) -> u16 {
        let mut blocked = 0;
        let mut bits = open;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let until = self.until[lane as usize];
            if now < until {
                blocked |= bit;
                if until <= self.last {
                    self.held |= bit;
                    self.next = self.next.min(until);
                }
            } else {
                self.open ^= bit;
            }
        }
        blocked
    }

    #[inline]
    #[expect(clippy::indexing_slicing, reason = "I solemnly swear this this will not panic.")]
    fn start(&mut self, lane: u32, now: Time, duration: Ms) -> Time {
        let end = Time(now.0.saturating_add(u32::from(duration)));
        self.until[lane as usize] = end;
        self.open |= 1_u16.checked_shl(lane).unwrap_or(0);
        end
    }
}

impl Lasting for Expiry {
    const ANY: bool = true;
    const TRICKS: bool = false;

    #[inline]
    fn new(last: Time, _: &Scoring) -> Self {
        Self {
            until: [Time(0); LANES],
            open: 0,
            held: 0,
            next: NEVER,
            last,
            lock: Time(0),
        }
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "benchmarked")]
    fn admit(&mut self, mask: u16, now: Time) -> u16 {
        let open = mask & self.open;
        if open == 0 {
            return mask;
        }
        mask & !self.hold(open, now)
    }

    #[inline]
    fn next(&self) -> Time {
        self.next
    }

    #[expect(clippy::indexing_slicing, reason = "`held` has all of the bits of the lane.")]
    fn release(&mut self, at: Time) -> u16 {
        let mut due = 0;
        let mut next = NEVER;
        let mut bits = self.held;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let until = self.until[lane as usize];
            if until <= at {
                due |= bit;
            } else {
                next = next.min(until);
            }
        }
        self.held ^= due;
        self.next = next;
        due
    }

    #[inline]
    fn converted(&self, now: Time) -> bool {
        self.locked(now)
    }

    #[inline]
    fn locked(&self, now: Time) -> bool {
        now < self.lock
    }

    #[inline]
    fn lock(&mut self, lane: u32, now: Time, duration: Ms) {
        let end = self.start(lane, now, duration);
        self.lock = self.lock.max(end);
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn trick(&mut self, _: u32, _: Time, _: Ms) {
        debug_assert!(false, "A Total Trick needs to schedule a duration end.");
    }
}

pub(in crate::play) struct Tricked {
    expiry: Expiry,
    /// The latest end of a Perfect Lock or a Total Trick.
    convert: Time,
    worth: Worth,
}

impl Lasting for Tricked {
    const ANY: bool = true;
    const TRICKS: bool = true;

    #[inline]
    fn new(last: Time, scoring: &Scoring) -> Self {
        Self {
            expiry: Expiry::new(last, scoring),
            convert: Time(0),
            worth: Worth::new(scoring.team_strength, scoring.trick),
        }
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "as `Expiry`'s")]
    fn admit(&mut self, mask: u16, now: Time) -> u16 {
        self.expiry.admit(mask, now)
    }

    #[inline]
    fn next(&self) -> Time {
        self.expiry.next()
    }

    #[inline]
    fn release(&mut self, at: Time) -> u16 {
        self.expiry.release(at)
    }

    #[inline]
    fn converted(&self, now: Time) -> bool {
        now < self.convert
    }

    #[inline]
    fn locked(&self, now: Time) -> bool {
        self.expiry.locked(now)
    }

    #[inline]
    fn lock(&mut self, lane: u32, now: Time, duration: Ms) {
        let end = self.expiry.start(lane, now, duration);
        self.expiry.lock = self.expiry.lock.max(end);
        self.convert = self.convert.max(end);
    }

    #[inline]
    fn trick(&mut self, lane: u32, now: Time, duration: Ms) {
        let end = self.expiry.start(lane, now, duration);
        self.convert = self.convert.max(end);
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "benchmarked")]
    fn tap(&self, now: Time, head: bool, tail: bool, combo: u16, lane: u16, note: u16) -> u64 {
        worth_score(self.worth(now, head, tail), combo, lane, note)
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "as `tap`")]
    #[expect(clippy::indexing_slicing, reason = "bool")]
    fn worth(&self, now: Time, head: bool, tail: bool) -> u64 {
        self.worth.row(self.converted(now), self.locked(now))[usize::from(head)][usize::from(tail)]
    }
}

/// A team that can heal. We don't care about tracking heals if there's no healer cards or accs.
pub(in crate::play) struct Healing<E> {
    lasting: E,
    hearts: Hearts,
}

impl<E> Lasting for Healing<E>
where
    E: Lasting,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = true;

    #[inline]
    fn new(last: Time, scoring: &Scoring) -> Self {
        Self { lasting: E::new(last, scoring), hearts: Hearts::new(scoring) }
    }

    #[inline(always)]
    fn admit(&mut self, mask: u16, now: Time) -> u16 {
        self.lasting.admit(mask, now)
    }

    #[inline(always)]
    fn next(&self) -> Time {
        self.lasting.next()
    }

    #[inline(always)]
    fn release(&mut self, at: Time) -> u16 {
        self.lasting.release(at)
    }

    #[inline(always)]
    fn converted(&self, now: Time) -> bool {
        self.lasting.converted(now)
    }

    #[inline(always)]
    fn locked(&self, now: Time) -> bool {
        self.lasting.locked(now)
    }

    #[inline(always)]
    fn lock(&mut self, lane: u32, now: Time, duration: Ms) {
        self.lasting.lock(lane, now, duration);
    }

    #[inline(always)]
    fn trick(&mut self, lane: u32, now: Time, duration: Ms) {
        self.lasting.trick(lane, now, duration);
    }

    #[inline(always)]
    fn tap(&self, now: Time, head: bool, tail: bool, combo: u16, lane: u16, note: u16) -> u64 {
        let worth = self.lasting.worth(now, head, tail);
        overheal_score_val(worth, combo, lane, note, self.hearts.multiplier)
    }

    #[inline(always)]
    fn worth(&self, now: Time, head: bool, tail: bool) -> u64 {
        self.lasting.worth(now, head, tail)
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

/// TODO: every effect with a duration.
pub(super) const fn lasts(effect: SkillEffect) -> bool {
    matches!(effect, SkillEffect::PerfectLock | SkillEffect::TotalTrick)
}

/// TODO: every other effect.
#[inline(always)]
#[expect(
    clippy::too_many_arguments,
    reason = "I like difficult to read code. (just kidding. Maybe can try to break it up as a todo)"
)]
pub(super) fn pay(
    lane: u32,
    effect: SkillEffect,
    value: Centi,
    duration: Ms,
    hp: u8,
    now: Time,
    out: &mut Outcome,
    expiry: &mut impl Lasting,
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
        SkillEffect::Scorer
        | SkillEffect::Encore
        | SkillEffect::Amplify
        | SkillEffect::PerfectScoreUp
        | SkillEffect::SkillRateUp
        | SkillEffect::AppealBoost
        | SkillEffect::ComboBonusUp
        | SkillEffect::Sync
        | SkillEffect::Spark => {}
    }
}

#[cfg(test)]
mod tests {
    use super::Hearts;
    use crate::play::activation::{AccessorySkill, Skill, Triggers};
    use crate::play::judgement::Judgements;
    use crate::play::queue::Queue;
    use crate::play::tests::{
        ALWAYS, arena, at, fifty_one, only, play, scorer, seeded, sloppy, tap, worn,
    };
    use crate::play::{Outcome, Scoring, run};
    use crate::scalar::{Ms, RATE_UNIT, Rate};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{Event, EventKind, Time};

    #[expect(clippy::integer_division, reason = "a coin flip")]
    const FLIP: Rate = RATE_UNIT / 2;

    const TEAM: u32 = 80_000;

    /// All notes of a 51 note-map with PL active.
    const ALL_LOCKED: u64 = 50 * 1080 + 1188;

    fn clocked(trigger: TriggerType, threshold: u16, chance: Rate, value: u32) -> Skill {
        Skill { trigger, ..scorer(threshold, chance, value) }
    }

    fn lasting(
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

    fn lock(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::PerfectLock, trigger, threshold, chance, duration);
    }

    fn trick(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
    ) {
        lasting(scoring, lane, SkillEffect::TotalTrick, trigger, threshold, chance, duration);
    }

    #[test]
    fn check_pl_true_perfect_multiplier() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        lock(&mut scoring, 4, TriggerType::RhythmIcons, 51, ALWAYS, 1000);
        assert_eq!(Triggers::build(&queue, &scoring).lasting(), 1 << 4);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill), (1, 0));
        assert_eq!(out.tap, 45 * 1000 + 5 * 1080 + 1188);
        assert_eq!(out.perfects, 51);
    }

    #[test]
    fn check_a_lock_converts_greats() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        lock(&mut scoring, 4, TriggerType::RhythmIcons, 51, ALWAYS, 1000);
        scoring.skills[7] = clocked(TriggerType::Perfects, 1, ALWAYS, 1);

        for seed in 0..8 {
            let out = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(out.tap, 45 * 880 + 5 * 1000 + 1100, "seed {seed}");
            assert_eq!(out.perfects, 6, "seed {seed}: only locked notes");
            assert_eq!(out.skill, 6, "seed {seed}: converted notes");
        }
    }

    /// For the star note trigger that requires perfects on star notes.
    #[test]
    fn check_a_converted_star_rolls() {
        let star = |ms, lane| (Time(ms), Event::new(lane, EventKind::Tap, true, false));
        let arena = arena(&[star(2000, 0), star(2100, 1), star(2200, 2), tap(2300, 3)]);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[6] = clocked(TriggerType::StarNotes, 1, ALWAYS, 1);
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).skill, 0, "zero perfects on stars");

        lock(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, 5000);
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).skill, 3, "three perfects on stars");

        trick(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, 5000);
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).skill, 3, "Total Trick converts them too");
    }

    /// This was a big edge case for me that required a lot of searching random videos on Youtube
    /// to confirm the behavior. The question was what happens if you get a great at the beginning of a hold,
    /// then PL activates, and then you get a true perfect on the release? After all, that's great+perfect, which
    /// should be a great, but then get converted to a regular perfect, rather than a 1.08x perfect.
    /// But apparently it doesn't actually work that way. If you get a perfect at the end in that scenario,
    /// you get a 1.08x perfect even if you get a great at the start.
    #[test]
    fn check_a_hold_takes_the_lock_at_its_release() {
        let stream = [
            (Time(2500), Event::new(0, EventKind::HoldStart, false, false)),
            (Time(3500), Event::new(0, EventKind::HoldRelease, false, false)),
            (Time(3700), Event::new(1, EventKind::HoldStart, false, false)),
            (Time(4500), Event::new(1, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(800_000);
        lock(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 1000);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!(triggers.time().columns().0, [3000, u32::MAX]);
        let mut total = scoring;
        total.skills[8].effect = SkillEffect::TotalTrick;
        let total_triggers = Triggers::build(&queue, &total);
        assert_eq!(total_triggers.time(), triggers.time());
        assert_eq!((total_triggers.lasting(), total_triggers.tricks()), (1 << 8, 1 << 8));
        assert_eq!(triggers.tricks(), 0);

        let mut seen = [[false; 4]; 2];
        for seed in 0..128 {
            let config = sloppy(FLIP, seed);
            let judged = Judgements::build(&queue, &config);
            let a = (judged.at(0), judged.at(1));
            let b = (judged.at(2), judged.at(3));
            let locked_a = if a == (true, true) { 13_500 } else { 12_500 };
            let free_b = match b {
                (true, true) => 12_500,
                (false, false) => 9_680,
                _ => 11_000,
            };

            let out = run(&queue, &config, &scoring, &triggers, &judged);
            assert_eq!(out.tap, locked_a + free_b, "seed {seed}: {a:?} {b:?}");
            assert_eq!(out.perfects, 1 + u32::from(b == (true, true)), "seed {seed}");
            seen[0][usize::from(a.0) * 2 + usize::from(a.1)] = true;
            seen[1][usize::from(b.0) * 2 + usize::from(b.1)] = true;

            let tricked = run(&queue, &config, &total, &total_triggers, &judged);
            assert_eq!(tricked.tap, 12_500 + free_b, "seed {seed}: converted, never 1.08x");
            assert_eq!(Outcome { tap: out.tap, ..tricked }, out, "seed {seed}");
        }
        assert_eq!(seen, [[true; 4]; 2], "something wasn't tested properly.");
    }

    /// Trick raises the team's strength during a PL / modifies the PPN calc.
    #[test]
    fn check_trick_sis_stat_recalc() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.trick = 8_000;
        assert_eq!(play(&queue, &scoring).tap, 50 * 1000 + 1100, "no lock, no Trick SIS");

        lock(&mut scoring, 4, TriggerType::RhythmIcons, 51, ALWAYS, 1000);
        assert_eq!(play(&queue, &scoring).tap, 45 * 1000 + 5 * 1188 + 1306, "88,000 * 1.08");
        for seed in 0..8 {
            let out = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(out.tap, 45 * 880 + 5 * 1100 + 1210, "seed {seed}");
        }
    }

    #[test]
    fn check_pl_duration() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        lock(&mut scoring, 4, TriggerType::RhythmIcons, 1, ALWAYS, 450);

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 13);
        assert_eq!(out.tap, ALL_LOCKED);
    }

    /// An accessory's lock blocks its own card too.
    #[test]
    fn check_an_accessory_lock_blocks_its_card() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[4] = scorer(1, 0, 1);
        scoring.accessories[4] =
            AccessorySkill { value: 0, chance: ALWAYS, effect: SkillEffect::PerfectLock };
        scoring.durations[4][1] = 450;
        assert_eq!(Triggers::build(&queue, &scoring).lasting(), 1 << 4);

        let missed = play(&queue, &scoring);
        assert_eq!((missed.activations, missed.accessory_activations), (0, 13));
        assert_eq!(missed.tap, ALL_LOCKED);

        scoring.skills[4] = scorer(1, ALWAYS, 1);
        let landed = play(&queue, &scoring);
        assert_eq!((landed.activations, landed.accessory_activations, landed.skill), (51, 0, 51));
        assert_eq!(landed.tap, 51_100, "card didn't miss, so nothing locked");

        scoring.accessories[4].chance = 0;
        scoring.skills[4] = Skill { threshold: 0, ..scoring.skills[4] };
        assert_eq!(Triggers::build(&queue, &scoring).lasting(), 0, "neither can roll");
    }

    /// For score-based scorers, makes sure a PL skill acc counts towards their activation limit.
    #[test]
    fn check_a_held_score_lane_spends_an_activation() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[4] = Skill {
            limit: core::num::NonZeroU8::new(3),
            ..clocked(TriggerType::Score, 1500, 0, 0)
        };
        scoring.accessories[4] =
            AccessorySkill { value: 0, chance: ALWAYS, effect: SkillEffect::PerfectLock };
        scoring.durations[4][1] = 250;

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 3));

        scoring.skills[4].limit = None;
        assert!(play(&queue, &scoring).accessory_activations > 3, "the limit did nothing");
    }

    /// Just a quick validation that a PL contributes nothing if it doesn't activate.
    /// Score would be unchanged.
    #[test]
    fn check_a_lock_that_never_activates_affects_nothing() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut plain = Scoring::flat(TEAM);
        plain.skills[2] = scorer(4, FLIP, 100);
        plain.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
        plain.skills[6] = clocked(TriggerType::Time, 1, FLIP, 7);
        plain.skills[7] = clocked(TriggerType::Perfects, 2, FLIP, 3);
        plain.skills[8] = scorer(5, 0, 0);
        plain.skills[0] = clocked(TriggerType::Time, 2, 0, 0);
        let mut laddered = plain;
        laddered.skills[1] = clocked(TriggerType::Score, 1500, FLIP, 50);

        for team in [plain, laddered] {
            let mut locked = team;
            locked.trick = 9_999;
            lock(&mut locked, 8, TriggerType::RhythmIcons, 5, 0, 2000);
            trick(&mut locked, 0, TriggerType::Time, 2, 0, 1500);
            assert_eq!(Triggers::build(&queue, &locked).lasting(), (1 << 8) | 1);
            for accuracy in [ALWAYS, 8000, 0] {
                for seed in 0..32 {
                    let config = sloppy(accuracy, seed);
                    assert_eq!(
                        at(&queue, &locked, &config),
                        at(&queue, &team, &config),
                        "seed {seed}"
                    );
                }
            }
        }
    }

    #[test]
    fn check_total_trick_doesnt_give_true_perfect_bonus() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        trick(&mut scoring, 4, TriggerType::RhythmIcons, 51, ALWAYS, 1000);
        assert_eq!(Triggers::build(&queue, &scoring).lasting(), 1 << 4);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill), (1, 0));
        assert_eq!(out.tap, 50 * 1000 + 1100, "no 1.08x");
        assert_eq!(out.perfects, 51);

        scoring.skills[7] = clocked(TriggerType::Perfects, 1, ALWAYS, 1);
        for seed in 0..8 {
            let greats = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(greats.tap, 45 * 880 + 5 * 1000 + 1100, "seed {seed}: greats converted");
            assert_eq!(greats.perfects, 6, "seed {seed}: only converted notes");
            assert_eq!(greats.skill, 6, "seed {seed}: converted notes count for Perfects");
        }
    }

    #[test]
    fn check_total_trick_switches_the_trick_sis_on() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.trick = 8_000;
        trick(&mut scoring, 4, TriggerType::RhythmIcons, 51, ALWAYS, 1000);
        assert_eq!(play(&queue, &scoring).tap, 45 * 1000 + 5 * 1100 + 1210, "88,000, no 1.08x");
        for seed in 0..8 {
            let out = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(out.tap, 45 * 880 + 5 * 1100 + 1210, "seed {seed}");
        }
    }

    #[test]
    fn check_pl_and_a_trick_overlap() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // A Total Trick at 3s and 6s for 2s, a pl at 4s for 0.5s.
        // 2.0-2.9 none, 3.0-3.9 trick, 4.0-4.4 both, 4.5-4.9 trick, 5.0-5.9 none, 6.0-7.0 trick.
        let mut scoring = Scoring::flat(TEAM);
        trick(&mut scoring, 2, TriggerType::Time, 3, ALWAYS, 2000);
        lock(&mut scoring, 6, TriggerType::Time, 4, ALWAYS, 500);
        let inside = play(&queue, &scoring);
        assert_eq!(inside.activations, 3);
        assert_eq!(inside.tap, 50 * 1000 + 1100 + 5 * 80, "five notes with the pl");
        let inside_greats = at(&queue, &scoring, &sloppy(0, 0));
        assert_eq!(inside_greats.tap, 20 * 880 + 30 * 1000 + 1100);
        assert_eq!(inside_greats.perfects, 31);

        scoring.trick = 8_000;
        let tricked = play(&queue, &scoring);
        assert_eq!(tricked.tap, 20 * 1000 + 25 * 1100 + 1210 + 5 * 1188);

        // A lock at 3s and 6s for 2s, a Total Trick at 4s for 1.5s.
        // 2.0-2.9 none, 3.0-4.9 lock (and trick from 4.0), 5.0-5.4 trick, 5.5-5.9 none, 6.0-7.0 lock.
        let mut outlasting = Scoring::flat(TEAM);
        lock(&mut outlasting, 2, TriggerType::Time, 3, ALWAYS, 2000);
        trick(&mut outlasting, 6, TriggerType::Time, 4, ALWAYS, 1500);
        let after = play(&queue, &outlasting);
        assert_eq!(after.tap, 15 * 1000 + 5 * 1000 + 30 * 1080 + 1188, "trick drops no 1.08x");
        let after_greats = at(&queue, &outlasting, &sloppy(0, 0));
        assert_eq!(after_greats.tap, 15 * 880 + 35 * 1000 + 1100);
        assert_eq!(after_greats.perfects, 36);
    }

    /// Swaps lanes between a total trick and pl to make sure the result is otherwise the same.
    #[test]
    fn check_trick_is_pl_without_bonus() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut locks = Scoring::flat(TEAM);
        locks.trick = 5_000;
        lock(&mut locks, 8, TriggerType::RhythmIcons, 7, FLIP, 600);
        lock(&mut locks, 3, TriggerType::Time, 2, FLIP, 900);
        locks.skills[5] = clocked(TriggerType::Perfects, 3, FLIP, 10);
        locks.skills[1] = scorer(4, FLIP, 100);
        let mut tricks = locks;
        for lane in [8, 3] {
            tricks.skills[lane].effect = SkillEffect::TotalTrick;
        }

        let mut cheaper = 0;
        for accuracy in [ALWAYS, 8000, FLIP, 0] {
            for seed in 0..32 {
                let config = sloppy(accuracy, seed);
                let (lock, trick) = (at(&queue, &locks, &config), at(&queue, &tricks, &config));
                assert_eq!(Outcome { tap: lock.tap, ..trick }, lock, "seed {seed}");
                assert!(trick.tap <= lock.tap, "seed {seed}");
                if accuracy == 0 {
                    assert_eq!(trick, lock, "seed {seed}: no true PERFECT to pay");
                }
                cheaper += u32::from(trick.tap < lock.tap);
            }
        }
        assert!(cheaper > 0, "no lock gave 1.08x");
    }

    /// This actually affects every double duration combo between a card and an accessory but this
    /// is the first time it's relevant.
    /// So imagine a perfect lock accessory on a total trick card (because of course there's no total trick)
    /// accessory. Only one duration effect can be active at a time. So every roll in this test will proc one or the other, so the
    /// lane holds as a lock alone would. Only the accessory's windows give the 1.08x bonus.
    #[test]
    fn check_a_trick_card_wearing_a_lock() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        trick(&mut scoring, 4, TriggerType::RhythmIcons, 1, FLIP, 450);
        scoring.accessories[4] =
            AccessorySkill { value: 0, chance: ALWAYS, effect: SkillEffect::PerfectLock };
        scoring.durations[4][1] = 450;
        assert_eq!(Triggers::build(&queue, &scoring).lasting(), 1 << 4);

        let mut seen = [false; 2];
        for seed in 0..32 {
            let out = at(&queue, &scoring, &sloppy(ALWAYS, seed));
            assert_eq!(out.activations + out.accessory_activations, 13, "seed {seed}");
            assert_eq!(out.tap == 51_100, out.accessory_activations == 0, "seed {seed}");
            assert_eq!(out.tap == ALL_LOCKED, out.activations == 0, "seed {seed}");
            assert!((51_100..=ALL_LOCKED).contains(&out.tap), "seed {seed}");
            seen[0] |= out.activations > 0;
            seen[1] |= out.accessory_activations > 0;

            let greats = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(greats.tap, 51_100, "seed {seed}: every note converted");
        }
        assert_eq!(seen, [true; 2]);

        scoring.accessories[4] = AccessorySkill::NONE;
        scoring.skills[4].chance = ALWAYS;
        let alone = play(&queue, &scoring);
        assert_eq!((alone.activations, alone.tap), (13, 51_100));
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

        // No Heal SIS. These teams bank no hearts, so the heal pays nothing at all.
        healed.skills[3].value = 0;
        healed.accessories[3].value = 0;
        let bare = seeded(&queue, &healed, 0);
        assert_eq!(bare, Outcome { skill: 0, ..seeded(&queue, &scored, 0) });
        assert!(bare.activations > 0);
    }

    /// Fixture heart worth +10%. Twenty of them would be +200% and 200% is the cap.
    fn overhealing(scoring: &mut Scoring, max_hp: u8) {
        (scoring.max_hp, scoring.per_heart, scoring.max_hearts) = (max_hp, 1000, 20);
    }

    fn healer(
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

    fn hearts(max_hp: u8, per_heart: u16, max_hearts: u16) -> Hearts {
        let mut scoring = Scoring::flat(TEAM);
        (scoring.max_hp, scoring.per_heart, scoring.max_hearts) = (max_hp, per_heart, max_hearts);
        Hearts::new(&scoring)
    }

    /// Every icon heals 10 = 1 full HP bar.
    /// Each bar grants a heart.
    /// Icon 10 spawns at 2300, on note 4's tap
    /// So the n-th heart applies from note 10n - 5.
    const ONE_HEART_EVERY_TEN: u64 =
        4 * 1000 + 10 * 1100 + 10 * 1200 + 10 * 1300 + 10 * 1400 + 6 * 1500 + 1650;

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
