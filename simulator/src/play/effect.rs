//! Skill effects. What a skill does once it activates.

use super::tap::{Worth, worth_score};
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
    #[expect(clippy::indexing_slicing, reason = "bool")]
    fn tap(&self, now: Time, head: bool, tail: bool, combo: u16, lane: u16, note: u16) -> u64 {
        let row = self.worth.row(self.converted(now), self.locked(now));
        worth_score(row[usize::from(head)][usize::from(tail)], combo, lane, note)
    }
}

/// TODO: every effect with a duration.
pub(super) const fn lasts(effect: SkillEffect) -> bool {
    matches!(effect, SkillEffect::PerfectLock | SkillEffect::TotalTrick)
}

/// TODO: every other effect.
#[inline(always)]
#[expect(clippy::inline_always, reason = "benchmarked")]
pub(super) fn pay(
    lane: u32,
    effect: SkillEffect,
    value: Centi,
    duration: Ms,
    now: Time,
    out: &mut Outcome,
    expiry: &mut impl Lasting,
) {
    match effect {
        SkillEffect::Scorer => out.skill = out.skill.saturating_add(u64::from(value)),
        SkillEffect::PerfectLock => expiry.lock(lane, now, duration),
        SkillEffect::TotalTrick => expiry.trick(lane, now, duration),
        SkillEffect::Healer
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
    use crate::play::activation::{AccessorySkill, Skill, Triggers};
    use crate::play::judgement::Judgements;
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, arena, at, fifty_one, only, play, scorer, sloppy, tap};
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
}
