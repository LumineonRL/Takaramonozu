//! Combo Fever. Or Combo Bonus Up as schoolido.lu calls it.

use super::{Lasting, NEVER};
use crate::play::Scoring;
use crate::play::queue::{BUCKET, Queue};
use crate::play::tap::{FEVER_UNIT, fever_multiplier};
use crate::scalar::Ms;
use crate::song::{LANES, Time};

/// PSU and Spark belong here too. Maybe it shouldn't be called Fever then?
/// Eh, that can be dealt with in the future :)
pub(in crate::play) struct Fever<'q, E> {
    lasting: E,
    times: &'q [Time],
    sums: &'q [u32],
    buckets: &'q [u32],
    paid: u64,
    running: bool,
    sum: u32,
    next: Time,
    open: u16,
    carry: u64,
    until: [Time; LANES],
    value: [u32; LANES],
}

impl<E> Fever<'_, E> {
    /// Drops expired windows.
    #[inline(never)]
    #[expect(clippy::indexing_slicing, reason = "`open` is just lane bits")]
    fn lapse(&mut self, now: Time) {
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
        self.next = next;
    }
}

impl<'q, E> Lasting<'q> for Fever<'q, E>
where
    E: Lasting<'q>,
{
    const ANY: bool = E::ANY;
    const TRICKS: bool = E::TRICKS;
    const HEARTS: bool = E::HEARTS;
    const FEVER: bool = true;
    const PSU: bool = E::PSU;
    const PARAM: bool = E::PARAM;

    forward! {
        admit, next, release, converted, locked, lock, trick, occupy, tap, worth,
        heal, overheal_strength,
        psu, expire, psu_tap, psu_paid,
        begin, boosted, rates, sru,
        strength, hearted_strength, param,
    }

    #[inline]
    fn new(last: Time, queue: &'q Queue<'_>, scoring: &Scoring) -> Self {
        Self {
            lasting: E::new(last, queue, scoring),
            times: queue.scored(),
            sums: queue.fever(),
            buckets: queue.buckets(),
            paid: 0,
            running: false,
            sum: 0,
            next: NEVER,
            open: 0,
            carry: 0,
            until: [Time(0); LANES],
            value: [0; LANES],
        }
    }

    #[expect(clippy::indexing_slicing, reason = "is safe")]
    fn fever(&mut self, lane: u32, now: Time, duration: Ms, value: u32, notes: u32) {
        let end = self.lasting.occupy(lane, now, duration);
        if self.running {
            if now >= self.next {
                self.lapse(now);
            }
            let bit = 1_u16.checked_shl(lane).unwrap_or(0);
            self.until[lane as usize] = end;
            self.value[lane as usize] = value;
            self.open |= bit;
            self.sum = self.sum.saturating_add(value);
            self.next = self.next.min(end);
            return;
        }
        let from = (notes as usize).min(self.times.len());
        let bucket = ((end.0 >> BUCKET) as usize).min(self.buckets.len().saturating_sub(1));
        let mut to = self.buckets.get(bucket).map_or(from, |&n| (n as usize).max(from));
        while self.times.get(to).is_some_and(|&t| t < end) {
            to = to.saturating_add(1);
        }
        let worth = self.sums[to].saturating_sub(self.sums[from]);
        self.paid = self.paid.saturating_add(u64::from(value).saturating_mul(u64::from(worth)));
    }

    #[inline(always)]
    #[expect(clippy::integer_division, reason = "game mechanic")]
    fn fevered(&mut self, now: Time, combo: u32) -> u64 {
        if now >= self.next {
            self.lapse(now);
        }
        let due = self
            .carry
            .saturating_add(u64::from(self.sum).saturating_mul(u64::from(fever_multiplier(combo))));
        self.carry = due % FEVER_UNIT;
        due / FEVER_UNIT
    }

    #[inline(always)]
    fn read_running(&mut self) {
        self.running = true;
    }

    #[inline(always)]
    #[expect(clippy::integer_division, reason = "game mechanic.")]
    fn fever_paid(&self) -> u64 {
        self.paid / FEVER_UNIT
    }
}

#[cfg(test)]
mod tests {
    use crate::play::Scoring;
    use crate::play::activation::{AccessorySkill, Triggers};
    use crate::play::effect::tests::{TEAM, clocked, fever, ladder};
    use crate::play::queue::Queue;
    use crate::play::tests::{ALWAYS, arena, at, fifty_one, only, play, sloppy, tap};
    use crate::skill::{SkillEffect, TriggerType};

    /// Fever multipliers of notes 11..=51
    const ELEVEN_ON: u32 = 10 * 110 + 10 * 120 + 10 * 130 + 10 * 140 + 150;

    #[test]
    fn check_combo_fever() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        fever(&mut scoring, 4, TriggerType::Combo, 10, ALWAYS, 45, 10_000);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.fever(), triggers.lasting()), (1 << 4, 1 << 4));

        let out = play(&queue, &scoring);
        assert_eq!(ELEVEN_ON, 5150);
        assert_eq!(out.skill, 2317, "45 * 51.5 = 2317.5");

        let mut matched = scoring;
        matched.lanes = [121; 9];
        assert_eq!(play(&queue, &matched).skill, out.skill, "no lane bonus");
        for seed in 0..8 {
            assert_eq!(
                at(&queue, &scoring, &sloppy(0, seed)).skill,
                out.skill,
                "acc don't matter for cf"
            );
        }

        ladder(&mut scoring, 6);
        let read = play(&queue, &scoring);
        assert_eq!(Triggers::build(&queue, &scoring).score(), 1 << 6);
        assert_eq!(read, out, "same");
    }

    #[test]
    fn check_fever_duration() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        fever(&mut scoring, 8, TriggerType::Time, 3, ALWAYS, 100, 500);

        // [3000, 3500) is notes 11 to 15.
        // [6000, 6500) is notes 41 to 45.
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 2);
        assert_eq!(out.skill, 5 * 110 + 5 * 140);

        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out);
    }

    /// BACK IN MY DAY COMBO FEVER HAD A CAP.
    /// But the skill was shit and apparently klab actually buffed it shortly before eos.
    #[test]
    fn check_fever_has_no_cap() {
        let stream: Vec<_> = (0..230_u32).map(|i| tap(2000 + i * 50, (i % 9) as u8)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        fever(&mut scoring, 2, TriggerType::Time, 1, ALWAYS, 300, 60_000);
        fever(&mut scoring, 6, TriggerType::Time, 1, ALWAYS, 300, 60_000);

        let rows: u32 = [
            100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, 225, 250, 275, 300, 350, 400,
            500, 600, 700, 800, 900,
        ]
        .iter()
        .sum();
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 2);
        assert_eq!(out.skill * 100, u64::from(600 * (10 * rows + 10 * 1000)));
        assert_eq!(out.skill, 477_000);

        ladder(&mut scoring, 4);
        assert_eq!(play(&queue, &scoring), out);
    }

    #[test]
    fn check_fever_lane_blocking() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        fever(&mut scoring, 4, TriggerType::Combo, 5, ALWAYS, 100, 550);

        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 9);
        assert_eq!(out.skill, 5 * 100 + 10 * 110 + 10 * 120 + 10 * 130 + 10 * 140 + 150);

        ladder(&mut scoring, 6);
        assert_eq!(play(&queue, &scoring), out);
    }

    #[test]
    fn check_fever_acc() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        scoring.skills[4] = clocked(TriggerType::Combo, 10, 0, 1);
        scoring.accessories[4] =
            AccessorySkill { value: 45, chance: ALWAYS, effect: SkillEffect::ComboFever };
        scoring.durations[4][1] = 10_000;
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.fever(), triggers.lasting()), (1 << 4, 1 << 4));

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations), (0, 1));
        assert_eq!(out.skill, 2317);
    }

    #[test]
    fn check_fever_contribs_to_score_scorers() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(TEAM);
        fever(&mut scoring, 4, TriggerType::Combo, 10, ALWAYS, 300, 10_000);
        scoring.skills[6] = clocked(TriggerType::Score, 10_000, ALWAYS, 0);

        let out = play(&queue, &scoring);
        assert_eq!(out.skill * 100, u64::from(300 * ELEVEN_ON));
        assert_eq!(out.activations, 1 + 6);
        scoring.skills[4].value = 0;
        assert_eq!(play(&queue, &scoring).activations, 1 + 5);
    }
}
