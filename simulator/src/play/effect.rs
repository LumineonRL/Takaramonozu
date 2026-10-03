//! Skill effects. What a skill does once it activates.

use super::Outcome;
use crate::scalar::{Centi, Ms};
use crate::skill::SkillEffect;
use crate::song::{LANES, Time};

/// No lane is waiting to roll.
const NEVER: Time = Time(u32::MAX);

// For Duration effects. For now, just Perfect Lock, but SRU, CBU, etc will
/// likely utilize this too.
pub(in crate::play) trait Lasting {
    /// Whether anything on the team is active.
    const ANY: bool;

    /// `last` is the final judgement aka end of song.
    fn new(last: Time) -> Self;

    /// So a PL can't activate if that lane is already active. Instead it holds the
    /// activation until the active on expires.
    /// `admit` is for scheduling those activations that happen mid-duration
    /// `next` holds the time the lane rolls again.
    /// `release` holds the time when the duration ends.
    fn admit(&mut self, mask: u16, now: Time) -> u16;
    fn next(&self) -> Time;
    fn release(&mut self, at: Time) -> u16;

    fn locked(&self, now: Time) -> bool;
    fn lock(&mut self, lane: u32, now: Time, duration: Ms);
}

pub(in crate::play) struct Instant;

impl Lasting for Instant {
    const ANY: bool = false;

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn new(_: Time) -> Self {
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
    fn locked(&self, _: Time) -> bool {
        false
    }

    #[inline(always)]
    #[expect(clippy::inline_always, reason = "nothing")]
    fn lock(&mut self, _: u32, _: Time, _: Ms) {
        debug_assert!(false, "A PL did not schedule an expiration event.");
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

    #[inline]
    fn new(last: Time) -> Self {
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
    fn locked(&self, now: Time) -> bool {
        now < self.lock
    }

    #[inline]
    fn lock(&mut self, lane: u32, now: Time, duration: Ms) {
        let end = self.start(lane, now, duration);
        self.lock = self.lock.max(end);
    }
}

/// TODO: every effect with a duration.
pub(super) const fn lasts(effect: SkillEffect) -> bool {
    matches!(effect, SkillEffect::PerfectLock)
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
        // TODO: Total Trick will work, but without the 1.08x or Trick SIS.
        SkillEffect::PerfectLock => expiry.lock(lane, now, duration),
        SkillEffect::Healer
        | SkillEffect::Encore
        | SkillEffect::Amplify
        | SkillEffect::TotalTrick
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
    use crate::play::{Scoring, run};
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

    fn lock(
        scoring: &mut Scoring,
        lane: usize,
        trigger: TriggerType,
        threshold: u16,
        chance: Rate,
        duration: Ms,
    ) {
        scoring.skills[lane] = Skill {
            effect: SkillEffect::PerfectLock,
            ..clocked(trigger, threshold, chance, 0)
        };
        scoring.durations[lane][0] = duration;
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
        }
        assert_eq!(seen, [[true; 4]; 2], "something wasn't tested properly.");
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
        let mut laddered = plain;
        laddered.skills[1] = clocked(TriggerType::Score, 1500, FLIP, 50);

        for team in [plain, laddered] {
            let mut locked = team;
            lock(&mut locked, 8, TriggerType::RhythmIcons, 5, 0, 2000);
            assert_eq!(Triggers::build(&queue, &locked).lasting(), 1 << 8);
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
}
