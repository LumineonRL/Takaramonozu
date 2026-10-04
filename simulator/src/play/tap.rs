//! The tap score formula / PPN.

use crate::song::{Event, EventKind};

/// I love magic numbers! PPN is calculated by `team_strength / 100 * 1.25`.
/// But of course float arithmetic is the greatest evil of man-kind,
/// Thankfully we can multiply by the nice rational 1/80 instead because it's the same number.
const BASE: u64 = 80;

/// 1.00x -> 100. Again, floats are cringe.
pub(super) const UNIT: u16 = 100;

/// Note multiplierse can have an extra decimal. A swing hold gets a 0.625x multiplier. Wait a minute SIF1 doesn't
/// even have swing holds when I checked `songs.json` it has to though or else why would we have the multipliers for it.
/// My script searching `songs.json` must've been wrong. Surely. Maybe.
const NOTE_UNIT: u16 = 1000;

/// Previously this was 1.00, but apparently getting a great+great on a hold notes gives you a
/// 0.88*0.88 = 0.7744 multiplier, not 0.88. The problem is we can't express 0.7744 with integer division with
/// 100 as the base. But fear not, 625 is the smallest available number that allows us to express all the multipliers
/// we need! Thank goodness because using 10000 would've looked ugly.
const JUDGEMENT_UNIT: u16 = 625;

/// Perfects are a 1.00x multiplier.
const PERFECT: u16 = JUDGEMENT_UNIT;

/// Greats are a 0.88x multiplier 500/625 = 0.88
const GREAT: u16 = 550;

/// A hold GREAT at both ends: 484/625 = 0.7744
const GREAT_GREAT: u16 = 484;

/// A true Perfect under a Perfect Lock. I can't believe this works with the 625 denominator too.
/// 675/625 = 1.08.
const LOCKED_PERFECT: u16 = 675;

pub(super) const JUDGEMENT: [[[u16; 2]; 2]; 2] = [
    [[GREAT_GREAT, GREAT], [GREAT, PERFECT]],
    [[PERFECT, PERFECT], [PERFECT, LOCKED_PERFECT]],
];

const DIVISOR: u64 = BASE * 100 * 100 * 1000 * JUDGEMENT_UNIT as u64;

/// Transcription of `combo_bonuses.json`.
///
/// TODO: might move to a table module once there are several of these.
const COMBO_BONUS: [(u32, u16); 7] = [
    (0, 100),
    (51, 110),
    (101, 115),
    (201, 120),
    (401, 125),
    (601, 130),
    (801, 135),
];

/// I thought .`find` would be pretty slow but apparently the compiler makes it
/// not slow.
#[must_use]
pub fn combo_bonus(combo: u32) -> u16 {
    COMBO_BONUS
        .iter()
        .rev()
        .find(|&&(at, _)| combo >= at)
        .map_or(100, |&(_, mult)| mult)
}

/// `floor(team_strength * 0.0125)`
#[must_use]
#[expect(clippy::integer_division, reason = "Game mechanic.")]
pub fn points_per_note(team_strength: u32) -> u32 {
    let scaled = u64::from(team_strength) / BASE;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

pub(super) const fn note_multiplier(event: Event) -> u16 {
    match (matches!(event.kind(), EventKind::HoldRelease), event.is_swing()) {
        (true, true) => 625,
        (true, false) => 1250,
        (false, true) => 500,
        (false, false) => NOTE_UNIT,
    }
}

/// `floor(team_strength * combo * lane * note * judgement / 500_000_000_000)`.
#[expect(clippy::integer_division, reason = "Game mechanic.")]
pub(super) fn note_score(
    team_strength: u32,
    combo: u16,
    lane: u16,
    note: u16,
    judgement: u16,
) -> u64 {
    u64::from(team_strength)
        .saturating_mul(u64::from(combo))
        .saturating_mul(u64::from(lane))
        .saturating_mul(u64::from(note))
        .saturating_mul(u64::from(judgement))
        / DIVISOR
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::integer_division,
        reason = "Kind of need to verify that the things we're doing to avoid division still result in the same number."
    )]

    use super::{combo_bonus, points_per_note};
    use crate::play::queue::Queue;
    use crate::play::tests::{arena, only, play, tap};
    use crate::play::{Outcome, Scoring};
    use crate::song::{Event, EventKind, Time};

    #[test]
    fn check_combo_table() {
        assert_eq!(combo_bonus(0), 100, "first 50 notes");
        assert_eq!(combo_bonus(50), 100);
        assert_eq!(combo_bonus(51), 110);
        assert_eq!(combo_bonus(100), 110);
        assert_eq!(combo_bonus(101), 115);
        assert_eq!(combo_bonus(200), 115);
        assert_eq!(combo_bonus(201), 120);
        assert_eq!(combo_bonus(401), 125);
        assert_eq!(combo_bonus(601), 130);
        assert_eq!(combo_bonus(800), 130);
        assert_eq!(combo_bonus(801), 135);
        assert_eq!(combo_bonus(u32::MAX), 135);
    }

    #[test]
    fn check_ppn() {
        assert_eq!(points_per_note(800), 10);
        assert_eq!(points_per_note(0), 0);
        assert_eq!(points_per_note(79), 0);
        assert_eq!(points_per_note(100_000), 1250);

        // Checks that the int and float versions give the same resolve.
        for team_strength in [1_u32, 999, 12_345, 250_000, 1_000_000, 2_700_000] {
            let float = (f64::from(team_strength) * 0.0125).floor();
            assert_eq!(f64::from(points_per_note(team_strength)), float, "at {team_strength}");
        }
    }

    #[test]
    fn check_accuracy_multipliers() {
        const TABLE_PERFECT: u64 = 125;
        const TABLE_GREAT: u64 = 110;

        assert_eq!(100 * 100, super::BASE * TABLE_PERFECT, "0.01 * 1.25 = 1/80");
        assert_eq!(
            TABLE_GREAT * u64::from(super::JUDGEMENT_UNIT),
            u64::from(super::GREAT) * TABLE_PERFECT
        );
        assert_eq!(super::PERFECT, super::JUDGEMENT_UNIT, "assumed PERFECT");
        assert_eq!(
            u32::from(super::GREAT_GREAT) * u32::from(super::JUDGEMENT_UNIT),
            u32::from(super::GREAT) * u32::from(super::GREAT),
            "multiply both greats of the hold, 0.88 * 0.88 = 0.7744"
        );
        assert_eq!(
            super::JUDGEMENT[0],
            [[super::GREAT_GREAT, super::GREAT], [super::GREAT, super::PERFECT]]
        );
        assert_eq!(
            u32::from(super::LOCKED_PERFECT) * 100,
            u32::from(super::JUDGEMENT_UNIT) * 108,
            "1.08x"
        );
        assert_eq!(
            super::JUDGEMENT[1],
            [
                [super::PERFECT, super::PERFECT],
                [super::PERFECT, super::LOCKED_PERFECT]
            ],
            "PL converts into perfects, gives multiplier when true perfect."
        );

        assert_eq!(11 * 100 * 100 * 100, 800 * TABLE_PERFECT * 110);

        assert_eq!(super::DIVISOR, 800_000_000 * u64::from(super::JUDGEMENT_UNIT));
        for team_strength in [1_u32, 999, 26_540, 80_400, 2_700_000, 16_000_000] {
            assert_eq!(
                super::note_score(team_strength, 135, 121, 1250, super::PERFECT),
                u64::from(team_strength) * 135 * 121 * 1250 / 800_000_000,
            );
            assert_eq!(
                super::note_score(team_strength, 135, 121, 1250, super::LOCKED_PERFECT),
                u64::from(team_strength) * 135 * 121 * 1250 * 108 / 80_000_000_000,
                "the widest possible product jere would not saturate"
            );
        }
    }

    #[test]
    fn check_score_floor() {
        let stream = [tap(2000, 0), tap(2500, 1), tap(3000, 2)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // 110_000 team_strength = 1375 ppn
        let out = play(&queue, &Scoring::flat(110_000));
        assert_eq!(
            out,
            Outcome {
                tap: 1375 * 3,
                notes: 3,
                icons: 3,
                perfects: 3,
                ..Outcome::default()
            }
        );

        assert_eq!(super::note_score(110_000, 110, 100, 1000, super::PERFECT), 1512);

        assert_eq!(points_per_note(26_540), 331);
        assert_eq!(super::note_score(26_540, 130, 110, 1000, super::PERFECT), 474);
        assert_eq!(331 * 130 * 110 / 10_000, 473, "pre-flooring the base is a no-no apparently.");
    }

    #[test]
    fn check_note_type_multipliers() {
        let stream = [
            tap(2000, 0),
            (Time(2100), Event::new(1, EventKind::Tap, true, false)),
            (Time(2200), Event::new(2, EventKind::Tap, false, true)),
            (Time(2300), Event::new(3, EventKind::HoldStart, false, false)),
            (Time(2400), Event::new(3, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let out = play(&queue, &Scoring::flat(800_000));
        assert_eq!(out.notes, 4, "hold note = 1 combo");
        assert_eq!(out.icons, 4, "and 1 rhythm icon, at the head");
        assert_eq!(out.score(), 10_000 + 10_000 + 5_000 + 12_500);

        let regular = Event::new(0, EventKind::Tap, false, false);
        let star = Event::new(0, EventKind::Tap, true, false);
        let swing = Event::new(0, EventKind::Tap, false, true);
        let start = Event::new(0, EventKind::HoldStart, false, false);
        let release = Event::new(0, EventKind::HoldRelease, false, false);
        assert_eq!(super::note_multiplier(regular), 1000);
        assert_eq!(super::note_multiplier(star), 1000, "a star is worth 1.0");
        assert_eq!(super::note_multiplier(swing), 500);
        assert_eq!(super::note_multiplier(release), 1250);
        assert_eq!(super::note_multiplier(start), 1000, "and it never scores anyway");

        let both = Event::new(0, EventKind::HoldRelease, false, true);
        assert_eq!(super::note_multiplier(both), 625, "1.25 * 0.5");
    }

    #[test]
    fn check_score_formula() {
        // One note per lane
        let stream: Vec<_> = (0_u8..9).map(|i| tap(2000 + u32::from(i) * 100, i)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // Under 51 notes, so combo is 1.00x.
        // 80,400 team strength is 1005 ppn
        let mut scoring = Scoring::flat(80_400);
        scoring.lanes = [100, 110, 121, 100, 110, 121, 100, 110, 121];
        let out = play(&queue, &scoring);
        assert_eq!(out.score(), 3 * (1005 + 1105 + 1216));

        assert_eq!(super::note_score(80_400, 100, 121, 1000, super::PERFECT), 1216);
        assert_eq!(super::note_score(80_400, 100, 110, 1000, super::PERFECT), 1105);
        assert_eq!(super::note_score(88_400, 100, 110, 1000, super::PERFECT), 1215);

        assert_eq!(
            super::note_score(80_400, 110, 110, 1000, super::PERFECT),
            1216,
            "1005 * 1.21 the long way"
        );
    }

    #[test]
    fn check_combo_multiplier() {
        // 51 notes. Last note is the first to have 1.10x.
        let stream: Vec<_> = (0..51).map(|i| tap(2000 + i * 100, (i % 9) as u8)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        assert_eq!(queue.combo()[49], 100, "50th note");
        assert_eq!(queue.combo()[50], 110, "51st");

        // 80k team_strength is 1000ppn
        let out = play(&queue, &Scoring::flat(80_000));
        assert_eq!(out.score(), 1000 * 50 + 1100);
    }
}
