//! Perfect Score Up aka PSU.

use super::{Lasting, NEVER};
use crate::play::Scoring;
use crate::play::queue::Queue;
use crate::scalar::Ms;
use crate::song::{LANES, Time};

/// Flat points for every perfect while the skill is active. Stacks.
pub(in crate::play) struct Psu<E> {
    lasting: E,
    sum: u32,
    mark: u32,
    paid: u64,
    ending: Time,
    wake: Time,
    open: u16,
    until: [Time; LANES],
    value: [u32; LANES],
}

impl<'q, E> Psu<E>
where
    E: Lasting<'q>,
{
    #[inline(always)]
    fn settle(&mut self, perfects: u32) {
        let new = u64::from(perfects.saturating_sub(self.mark));
        self.paid = self.paid.saturating_add(u64::from(self.sum).saturating_mul(new));
        self.mark = perfects;
    }

    /// Turns skill off when time duration ends.
    #[inline(never)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    fn lapse(&mut self, now: Time, perfects: u32) {
        self.settle(perfects);
        let mut next = NEVER;
        let mut bits = self.open;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let until = self.until[lane as usize];
            if until <= now {
                self.open ^= bit;
                self.sum = self.sum.saturating_sub(self.value[lane as usize]);
            } else {
                next = next.min(until);
            }
        }
        self.ending = next;
        self.wake = self.lasting.next().min(next);
    }
}

impl<'q, E> Lasting<'q> for Psu<E>
where
    E: Lasting<'q>,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = E::HEARTS;
    const PSU: bool = true;
    const PARAM: bool = E::PARAM;

    forward! {
        converted, locked, lock, trick, occupy, tap, worth,
        heal, overheal_strength,
        begin, boosted, rates, sru,
        strength, hearted_strength, seed,
    }

    #[inline]
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self {
        Self {
            lasting: E::new(last, queue, scoring),
            sum: 0,
            mark: 0,
            paid: 0,
            ending: NEVER,
            wake: NEVER,
            open: 0,
            until: [Time(0); LANES],
            value: [0; LANES],
        }
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

    #[expect(clippy::indexing_slicing, reason = "is safe")]
    #[inline(never)]
    fn psu(&mut self, lane: u32, now: Time, duration: Ms, value: u32, perfects: u32) {
        let end = self.lasting.occupy(lane, now, duration);
        if now >= self.ending {
            self.lapse(now, perfects);
        } else {
            self.settle(perfects);
        }
        self.until[lane as usize] = end;
        self.value[lane as usize] = value;
        self.open |= 1_u16.checked_shl(lane).unwrap_or(0);
        self.sum = self.sum.saturating_add(value);
        self.ending = self.ending.min(end);
        self.wake = self.wake.min(end);
    }

    #[inline(always)]
    fn expire(&mut self, now: Time, perfects: u32) {
        if E::PARAM {
            self.lasting.expire(now, perfects);
            self.wake = self.lasting.next().min(self.ending);
        }
        if now >= self.ending {
            self.lapse(now, perfects);
        }
    }

    #[inline(always)]
    fn psu_tap(&self, perfect: bool) -> u64 {
        if perfect { u64::from(self.sum) } else { 0 }
    }

    #[inline(always)]
    fn psu_paid(&self, perfects: u32) -> u64 {
        let new = u64::from(perfects.saturating_sub(self.mark));
        self.paid.saturating_add(u64::from(self.sum).saturating_mul(new))
    }

    #[inline(always)]
    fn param(&mut self, lane: u32, worn: bool, now: Time, duration: Ms, value: u32) {
        self.lasting.param(lane, worn, now, duration, value);
        if E::PARAM {
            self.wake = self.wake.min(self.lasting.next());
        }
    }

    #[inline(always)]
    fn sync(&mut self, lane: u32, worn: bool, now: Time, duration: Ms) {
        self.lasting.sync(lane, worn, now, duration);
        if E::PARAM {
            self.wake = self.wake.min(self.lasting.next());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::play::Scoring;
    use crate::play::activation::{AccessorySkill, Triggers};
    use crate::play::effect::tests::{ALL_LOCKED, FLIP, TEAM, clocked, ladder, lock, psu};
    use crate::play::judgement::Judgements;
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, arena, at, fifty_one, only, play, sloppy, tap};
    use crate::skill::{SkillEffect, TriggerType};

    #[test]
    fn check_psu() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 4, TriggerType::Perfects, 10, ALWAYS, 200, 10_000);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.psu(), triggers.lasting()), (1 << 4, 1 << 4));
        assert_eq!(triggers.fever(), 0);

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 1, "does nothing after last note.");
        assert_eq!(out.skill, 41 * 200, "notes 11 to 51");
        assert_eq!(out.tap, 50 * 1000 + 1100, "doesn't affect tap score itself");

        let mut matched = scoring;
        matched.lanes = [121; 9];
        assert_eq!(play(&queue, &matched).skill, out.skill, "no lane bonus");

        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out, "read note by note");
    }

    #[test]
    fn check_psu_only_activates_on_perfects() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 8, TriggerType::Time, 1, ALWAYS, 200, 60_000);
        assert_eq!(play(&queue, &scoring).skill, 51 * 200, "before any note");
        let mut read = scoring;
        ladder(&mut read, 6);

        let mut seen = 0;
        for seed in 0..16 {
            let config = sloppy(FLIP, seed);
            let judged = Judgements::build(&queue, &config);
            let perfects = (0..51).filter(|&i| judged.at(i)).count();
            let out = at(&queue, &scoring, &config);
            assert_eq!(out.perfects as usize, perfects, "seed {seed}");
            assert_eq!(out.skill, 200 * perfects as u64, "seed {seed}: greats, no psu bonus");
            assert_eq!(at(&queue, &read, &config), out, "seed {seed}");
            seen += usize::from(perfects != 0 && perfects != 51);
            assert_eq!(at(&queue, &scoring, &sloppy(0, seed)).skill, 0, "seed {seed}");
        }
        assert!(seen > 0);
    }

    #[test]
    fn check_psu_duration() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 100, 500);

        // [3000, 3500) is notes 11 to 15
        // [6000, 6500) notes 41 to 45.
        // time activation before the tap
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 2);
        assert_eq!(out.skill, 10 * 100);

        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out);
    }

    #[test]
    fn check_psu_stacks() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 2, TriggerType::Time, 1, ALWAYS, 3000, 60_000);
        psu(&mut scoring, 6, TriggerType::Time, 1, ALWAYS, 3000, 60_000);

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 2);
        assert_eq!(out.skill, 51 * 6000, "no cap");

        ladder(&mut scoring, 4);
        assert_eq!(play(&queue, &scoring), out);
    }

    /// Same windows as `check_fever_lane_blocking`.
    #[test]
    fn check_psu_lane_blocking() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 4, TriggerType::Perfects, 5, ALWAYS, 100, 550);

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 9);
        assert_eq!(out.skill, 46 * 100);

        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out);
    }

    #[test]
    fn check_psu_acc() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[4] = clocked(TriggerType::Perfects, 10, 0, 1);
        scoring.accessories[4] = AccessorySkill {
            value: 200,
            chance: ALWAYS,
            effect: SkillEffect::PerfectScoreUp,
        };
        scoring.durations[4][1] = 10_000;
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.psu(), triggers.lasting()), (1 << 4, 1 << 4));

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 1));
        assert_eq!(out.skill, 41 * 200);

        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out);
    }

    /// Greats turned into a perfect by a PL or TT still get the bonus
    #[test]
    fn check_psu_works_with_converted_greats() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 2, TriggerType::Time, 1, ALWAYS, 100, 60_000);
        // [3000, 4000) and [6000, 7000): notes 11 to 20 and 41 to 50.
        lock(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 1000);
        let mut tricked = scoring;
        tricked.skills[8].effect = SkillEffect::TotalTrick;

        for team in [scoring, tricked] {
            let mut read = team;
            ladder(&mut read, 6);
            for seed in 0..8 {
                let config = sloppy(0, seed);
                let out = at(&queue, &team, &config);
                assert_eq!((out.perfects, out.skill), (20, 20 * 100), "seed {seed}");
                assert_eq!(at(&queue, &read, &config), out, "seed {seed}");
            }
            assert_eq!(play(&queue, &team).skill, 51 * 100, "flat with pl");
        }
    }

    #[test]
    fn check_psu_timing() {
        let tied = arena(&[tap(2000, 0), tap(2100, 1), tap(2100, 2), tap(2200, 3)]);
        let queue = Queue::build(&tied, &only(&tied)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 8, TriggerType::Combo, 2, ALWAYS, 100, 10_000);
        let out = play(&queue, &scoring);
        assert_eq!(out.skill, 2 * 100, "notes 3 and 4");
        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out);

        // Icon 2 spawns on the first tap and spawns at 1400
        let spawned = arena(&[tap(1400, 0), tap(2000, 1), tap(2600, 2)]);
        let icons = Queue::build(&spawned, &only(&spawned)).expect("terminated");
        let mut iconed = Scoring::flat(TEAM);
        psu(&mut iconed, 8, TriggerType::RhythmIcons, 2, ALWAYS, 100, 10_000);
        let late = play(&icons, &iconed);
        assert_eq!(late.skill, 2 * 100, "notes 2 and 3");
        ladder(&mut iconed, 6);
        assert_eq!(play(&icons, &iconed), late);
    }

    #[test]
    fn check_psu_contribs_to_score_scorers() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 4, TriggerType::Perfects, 10, ALWAYS, 300, 10_000);
        scoring.skills[6] = clocked(TriggerType::Score, 10_000, ALWAYS, 0);

        let out = play(&queue, &scoring);
        assert_eq!(out.skill, 41 * 300);
        assert_eq!(out.activations, 1 + 6);
        scoring.skills[4].value = 0;
        assert_eq!(play(&queue, &scoring).activations, 1 + 5);
    }

    /// The lock holds its lane through every window, as in `check_pl_duration`, and has to
    /// wake the walk on a PSU team too.
    #[test]
    fn check_psu_team_releases_held_lanes() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut plain = Scoring::flat(TEAM);
        lock(&mut plain, 4, TriggerType::RhythmIcons, 1, ALWAYS, 450);
        plain.skills[8] = clocked(TriggerType::Perfects, 3, 0, 0);
        let mut never = plain;
        psu(&mut never, 8, TriggerType::Perfects, 3, 0, 200, 700);
        assert_eq!(Triggers::build(&queue, &never).psu(), 1 << 8);

        let out = play(&queue, &never);
        assert_eq!((out.activations, out.tap), (13, ALL_LOCKED));
        assert_eq!(out, play(&queue, &plain));
    }

    #[test]
    fn check_a_psu_ending_on_a_tap_gives_nothing() {
        // Icon 2 spawns at 4400 and lasts until the end.
        let stream = [tap(2000, 1), tap(5000, 1)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        psu(&mut scoring, 0, TriggerType::RhythmIcons, 2, ALWAYS, 100, 600);
        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill), (1, 0));

        let mut longer = scoring;
        longer.durations[0][0] = 601;
        assert_eq!(play(&queue, &longer).skill, 100);
    }
}
