//! The tap score formula / PPN.

use crate::song::{Event, EventKind};

/// I love magic numbers! PPN is calculated by `team_strength / 100 * 1.25`.
/// But of course float arithmetic is the greatest evil of man-kind,
/// Thankfully we can multiply by the nice rational 1/80 instead because it's the same number.
const BASE: u64 = 80;

/// 1.00x -> 100. Again, floats are cringe.
pub(super) const UNIT: u16 = 100;

/// Note multipliers can have an extra decimal. A swing hold gets a 0.625x multiplier. The scrape dropped the
/// Update: swing holds are very much real, it was the data that was wrong.
/// This is pretty big actually because affects burrito's sim too.
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

/// `[locked][head][tail]`.
pub(super) const JUDGEMENT: [[[u16; 2]; 2]; 2] = [
    [[GREAT_GREAT, GREAT], [GREAT, PERFECT]],
    [[PERFECT, PERFECT], [PERFECT, LOCKED_PERFECT]],
];

/// For total trick. Converts everything to a perfect without a true perfect bonus.
const CONVERTED: [[u16; 2]; 2] = [[PERFECT; 2]; 2];

const DIVISOR: u64 = BASE * 100 * 100 * 1000 * JUDGEMENT_UNIT as u64;

/// Base 1.00x overheal multiplier.
pub(super) const HEART_UNIT: u16 = 10_000;

/// The overheal bonus stops at +200%
pub(super) const HEART_CAP: u16 = 20_000;

/// Some performance hacking here because overheal was tanking performance by 20% even when no
/// healers were on the team. So the fun part is rewiring already implemented mechanics to help the
/// compiler out. All of these decisions were made by manually inspecting and comparing the compiled assembly.
/// So since every combo multiplier is a multiple of 5 and every note multiplier of 125,
/// the heart's extra factor fits a `u64` once those come out of both sides.
const COMBO_STEP: u16 = 5;
const NOTE_STEP: u16 = 125;
#[expect(clippy::integer_division, reason = "Exact. See below.")]
const OVERHEAL_DIVISOR: u64 = DIVISOR * HEART_UNIT as u64 / (COMBO_STEP as u64 * NOTE_STEP as u64);
#[expect(clippy::indexing_slicing, reason = "bounded by the loop")]
const _: () = {
    let mut i = 0;
    while i < COMBO_BONUS.len() {
        assert!(COMBO_BONUS[i].1.is_multiple_of(COMBO_STEP), "combo multiplier of overheal stage.");
        i += 1;
    }
    assert!(UNIT.is_multiple_of(COMBO_STEP), "lane's multiplier");
    assert!(NOTE_UNIT.is_multiple_of(NOTE_STEP), "note's multiplier");
    assert!(
        OVERHEAL_DIVISOR * (COMBO_STEP as u64 * NOTE_STEP as u64) == DIVISOR * HEART_UNIT as u64,
        "no remainder"
    );
};

/// For teams with total trick. the ppn times every judgements, so converted * locked * head * tail.
#[derive(Default)]
pub(super) struct Worth([[[[u64; 2]; 2]; 2]; 2]);

impl Worth {
    pub(super) fn new(team_strength: u32, trick: u32) -> Self {
        let strengths = [team_strength, team_strength.saturating_add(trick)].map(u64::from);
        let tables = [[JUDGEMENT[0], JUDGEMENT[1]], [CONVERTED, JUDGEMENT[1]]];
        let mut out = Self::default();
        for ((dst, rows), strength) in out.0.iter_mut().zip(tables).zip(strengths) {
            *dst = rows.map(|by_head| {
                by_head.map(|by_tail| by_tail.map(|j| strength.saturating_mul(u64::from(j))))
            });
        }
        out
    }

    /// `[head][tail]`.
    #[inline]
    #[expect(clippy::indexing_slicing, reason = "by bool")]
    pub(super) fn row(&self, converted: bool, locked: bool) -> &[[u64; 2]; 2] {
        &self.0[usize::from(converted)][usize::from(locked)]
    }
}

/// `note_score` with the strength and judgement already multiplied.
#[expect(clippy::integer_division, reason = "Game mechanic.")]
pub(super) fn worth_score(worth: u64, combo: u16, lane: u16, note: u16) -> u64 {
    let factor =
        u64::from(u32::from(combo).saturating_mul(u32::from(lane))).saturating_mul(u64::from(note));
    worth.saturating_mul(factor) / DIVISOR
}

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

/// Transcription of `combo_fever_map.json`,
const FEVER: [u16; 23] = [
    100, 110, 120, 130, 140, 150, 160, 170, 180, 190, 200, // 1..=110
    225, 250, 275, 300, 350, 400, 500, 600, 700, 800, 900,  // 111..=220
    1000, // 221..
];
const LAST_FEVER: u32 = 22;
const _: () = assert!(FEVER.len() == LAST_FEVER as usize + 1, "last row");
pub(super) const FEVER_UNIT: u64 = UNIT as u64;

/// Combo Fever's multiplier on the comboth note. Inclusive.
#[must_use]
#[expect(clippy::indexing_slicing, reason = "clamped")]
#[expect(clippy::integer_division, reason = "dividing by 10")]
pub fn fever_multiplier(combo: u32) -> u16 {
    FEVER[(combo.saturating_sub(1) / 10).min(LAST_FEVER) as usize]
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

/// This is part of the aforementioned performance hacking. `note_multiplier` to eighths
/// to be used for `overheal_score`.
#[expect(clippy::integer_division, reason = "exact")]
pub(super) const fn note_eighths(event: Event) -> u16 {
    match (matches!(event.kind(), EventKind::HoldRelease), event.is_swing()) {
        (true, true) => 625 / NOTE_STEP,
        (true, false) => 1250 / NOTE_STEP,
        (false, true) => 500 / NOTE_STEP,
        (false, false) => NOTE_UNIT / NOTE_STEP,
    }
}

/// Max lane bonus is 1.1x * 1.1x = 1.21x with a matching attribute and group
const MAX_LANE: u64 = 121;

/// Largest possible team strength if an absolutely nutty, infeasible team to prove that even in the absolute
/// worst case scenario, this number will not overflow.
/// It takes the max combo bonus, max matching multiplier, the 1.08 true PL multiplier, and a hold note multiplier.
/// There's a test that puts all of those multipliers on the most souped up team you've ever seen and guarantees
/// that it's still below overflowing.
#[expect(clippy::integer_division, reason = "for an upper bound so we can eliminate a bounds check")]
const WIDEST: u64 = u64::MAX / (MAX_COMBO * MAX_LANE * 1250 * LOCKED_PERFECT as u64);
#[expect(clippy::cast_possible_truncation, reason = "see assertion")]
pub(super) const MAX_STRENGTH: u32 = WIDEST as u32;
const _: () = assert!(MAX_STRENGTH as u64 == WIDEST, "fits a u32");

/// Just grabs the max combo tier for the above check.
const MAX_COMBO: u64 = match COMBO_BONUS.last() {
    Some(&(_, bonus)) => bonus as u64,
    None => 0,
};

/// `floor(team_strength * combo * lane * note * judgement / 500_000_000_000)`.
/// This was originally a `saturating_mul` rather than a wrapping_mul, but profiling showed the this was
/// one of the hottest functions in the sim and looking through the assembly showed that the bounds check
/// was not being yeeted by the compiler.
/// Swapping in a `wrapping_mul` instead bumped this from 2.53 million trials/second across 24 threads, up to 2.61 million
/// so not massive but actually kind of massive?
/// I diff'd the assembly and it looks like the saturating version had an extra `mul` and `jo` on every note.
#[expect(clippy::integer_division, reason = "Game mechanic.")]
pub(super) fn note_score(
    team_strength: u32,
    combo: u16,
    lane: u16,
    note: u16,
    judgement: u16,
) -> u64 {
    let factor = u64::from(u32::from(note).wrapping_mul(u32::from(judgement)));
    debug_assert!(team_strength <= MAX_STRENGTH, "team strength {team_strength}");
    debug_assert!(u64::from(lane) <= MAX_LANE, "lane bonus {lane}");
    u64::from(team_strength)
        .wrapping_mul(u64::from(combo))
        .wrapping_mul(u64::from(lane).wrapping_mul(factor))
        / DIVISOR
}

/// `note_score` * `overheal_multiplier`.
/// `strength` = `strength` * `HEART_UNIT` multiplier.
/// `note` = `note_eighths`.
/// This is fine so long as the team strength stays below 27 million which should not be a problem. Hopefully.
#[inline]
#[expect(clippy::integer_division, reason = "exact`")]
pub(super) fn overheal_score(
    strength: u64,
    combo: u16,
    lane: u16,
    note: u16,
    judgement: u16,
) -> u64 {
    debug_assert!(combo.is_multiple_of(COMBO_STEP), "{combo}");
    let factor = u64::from(u32::from(combo / COMBO_STEP).saturating_mul(u32::from(lane)))
        .saturating_mul(u64::from(u32::from(note).saturating_mul(u32::from(judgement))));
    strength.saturating_mul(factor) / OVERHEAL_DIVISOR
}

#[inline]
#[expect(clippy::integer_division, reason = "exact")]
pub(super) fn overheal_score_val(worth: u64, combo: u16, lane: u16, note: u16, heart: u16) -> u64 {
    debug_assert!(combo.is_multiple_of(COMBO_STEP), "{combo}");
    let factor = u64::from(u32::from(combo / COMBO_STEP).saturating_mul(u32::from(lane)))
        .saturating_mul(u64::from(u32::from(note).saturating_mul(u32::from(heart))));
    worth.saturating_mul(factor) / OVERHEAL_DIVISOR
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::integer_division,
        reason = "Kind of need to verify that the things we're doing to avoid division still result in the same number."
    )]

    use super::{combo_bonus, fever_multiplier, points_per_note};
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
    fn check_fever_table() {
        assert_eq!(fever_multiplier(1), 100);
        assert_eq!(fever_multiplier(10), 100);
        assert_eq!(fever_multiplier(11), 110);
        assert_eq!(fever_multiplier(110), 200);
        assert_eq!(fever_multiplier(111), 225);
        assert_eq!(fever_multiplier(220), 900);
        assert_eq!(fever_multiplier(221), 1000);
        assert_eq!(fever_multiplier(u32::MAX), 1000);

        // From `getCFMultiplier`, `combo <= bound` from burrito's sim
        let reference = [
            (10, 100),
            (20, 110),
            (30, 120),
            (40, 130),
            (50, 140),
            (60, 150),
            (70, 160),
            (80, 170),
            (90, 180),
            (100, 190),
            (110, 200),
            (120, 225),
            (130, 250),
            (140, 275),
            (150, 300),
            (160, 350),
            (170, 400),
            (180, 500),
            (190, 600),
            (200, 700),
            (210, 800),
            (220, 900),
        ];
        for combo in 1..=400 {
            let want =
                reference.iter().find(|&&(bound, _)| combo <= bound).map_or(1000, |&(_, m)| m);
            assert_eq!(fever_multiplier(combo), want, "combo {combo}");
        }
    }

    #[test]
    fn check_ppn() {
        assert_eq!(points_per_note(800), 10);
        assert_eq!(points_per_note(0), 0);
        assert_eq!(points_per_note(79), 0);
        assert_eq!(points_per_note(100_000), 1250);

        // Checks that the int and float versions give the same result.
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
            super::CONVERTED,
            [[super::PERFECT; 2]; 2],
            "Total Trick converts greats into perfects with no additional multiplier."
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
                "the widest possible product here would not saturate"
            );
        }
    }

    /// Safety proof that swapping `saturating_mul` for `wrapping_mul` will not have any negative
    /// ramifications in the release code. Constructs an impossible team, gives it every possible boost
    /// imaginable, and shows that it still comes out well under `u64::MAX`.
    #[test]
    fn check_no_team_reaches_max_strength() {
        let ceil = |x: u64, milli: u64| (x * milli).div_ceil(1000);
        // Card with the highest main attribute stat +, the non-promo level-500 cap bonus + `MAX_GALLERY`.
        let base = 8940 + 12_450 + 50_000;
        // Accessory with the highest stat.
        let worn = base + 8442;
        // Every SIS boost all at once. This is one of the impossible parts since you can't actually
        // equip all of these SIS.
        // It's a 74% self boost, 2050 flat boost, and 8.4% all-boost on all 9 cards.
        let sis = ceil(worn, 1740) + 2050 + ceil(base, 9 * 84);
        // Center and guest both with main 15% and extra 12% each
        let led = sis + 2 * ceil(sis, 150) + 2 * ceil(sis, 120);
        // This is the fun part! Again, this is impossible because the card chosen for this would not have
        // different skills on each card, but here an Amp banks a +16, taken by a param boost. The highest
        // param boost payout is 385%. Then syncs copy that lane and all share that 385% boost.
        // Then apply perfect lock tap bonus (1.08x) and Trick SIS bonus (1.33x to param)
        let lane = led + 2 * ceil(led, 3850) + ceil(led, 330);
        let team = 9 * lane;
        assert_eq!(team, 24_396_705); // add it all up and this is what it comes out to
        assert!(team * 50 < u64::from(super::MAX_STRENGTH), "{}", super::MAX_STRENGTH); // and it's still waaaay under the max.

        // The above didn't account for overheal hearts, but it's still safe with those included as well.
        let hearts = 30_000 * 27 * 121 * 10 * 675;
        assert!(team < u64::MAX / hearts, "{}", u64::MAX / hearts);

        // QED.
    }

    #[test]
    fn check_ppn_calc() {
        let judgements = [super::GREAT_GREAT, super::PERFECT, super::LOCKED_PERFECT];
        for team_strength in [0, 1, 80_400, 16_000_000, super::MAX_STRENGTH] {
            for (note, judgement) in [625, 1000, 1250].into_iter().zip(judgements) {
                let whole = u128::from(team_strength)
                    * 135
                    * 121
                    * u128::from(note)
                    * u128::from(judgement);
                let want = u64::try_from(whole).expect("exact") / super::DIVISOR;
                assert_eq!(super::note_score(team_strength, 135, 121, note, judgement), want);
            }
        }
        let widest = |strength: u32| u128::from(strength) * 135 * 121 * 1250 * 675;
        assert!(widest(super::MAX_STRENGTH) <= u128::from(u64::MAX), "exact at the limit");
        assert!(widest(super::MAX_STRENGTH + 1) > u128::from(u64::MAX), "and the limit is tight");
    }

    #[test]
    fn check_total_trick_ppn() {
        let rows = [
            (false, false, super::JUDGEMENT[0]),
            (true, false, super::CONVERTED),
            (true, true, super::JUDGEMENT[1]),
        ];
        for (team_strength, trick) in [
            (0, 0),
            (1, 7),
            (80_400, 26_532),
            (16_000_000, 1),
            (super::MAX_STRENGTH - 9, 9),
        ] {
            let worth = super::Worth::new(team_strength, trick);
            for (converted, locked, judgements) in rows {
                let strength = if converted {
                    team_strength.saturating_add(trick)
                } else {
                    team_strength
                };
                for (head, tail) in [(false, false), (false, true), (true, false), (true, true)] {
                    let judgement = judgements[usize::from(head)][usize::from(tail)];
                    for (combo, lane, note) in [(100, 100, 1000), (135, 121, 1250), (110, 110, 500)]
                    {
                        assert_eq!(
                            super::worth_score(
                                worth.row(converted, locked)[usize::from(head)][usize::from(tail)],
                                combo,
                                lane,
                                note
                            ),
                            super::note_score(strength, combo, lane, note, judgement),
                            "{team_strength} {converted} {locked} {head} {tail} {combo} {note}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn check_overheal_score() {
        assert_eq!(super::OVERHEAL_DIVISOR, 8_000_000_000_000);
        let judgements = [
            super::GREAT_GREAT,
            super::GREAT,
            super::PERFECT,
            super::LOCKED_PERFECT,
        ];
        let multipliers = [super::HEART_UNIT, 10_026, 13_380, 29_942, 30_000];
        let with_hearts = |strength: u32, heart: u16| u64::from(strength) * u64::from(heart);
        for team_strength in [0, 1, 999, 80_400, 2_700_000, 16_000_000, 27_000_000] {
            for (combo, lane, note) in [(100, 100, 1000), (135, 121, 1250), (110, 110, 500)] {
                let eighths = note / super::NOTE_STEP;
                for judgement in judgements {
                    let plain = super::note_score(team_strength, combo, lane, note, judgement);
                    let unit = with_hearts(team_strength, super::HEART_UNIT);
                    assert_eq!(
                        super::overheal_score(unit, combo, lane, eighths, judgement),
                        plain,
                        "{team_strength} {combo} {note} {judgement}"
                    );
                    for heart in multipliers {
                        let whole = u128::from(team_strength)
                            * u128::from(combo)
                            * u128::from(lane)
                            * u128::from(note)
                            * u128::from(judgement)
                            * u128::from(heart);
                        let want = whole / u128::from(super::DIVISOR * 10_000);
                        let strength = with_hearts(team_strength, heart);
                        assert_eq!(
                            u128::from(super::overheal_score(
                                strength, combo, lane, eighths, judgement
                            )),
                            want,
                            "{team_strength} {combo} {lane} {note} {judgement} {heart}"
                        );
                    }
                }
            }
        }
        let widest = super::overheal_score(with_hearts(u32::MAX, 30_000), 135, 121, 10, 675);
        assert_eq!(widest, u64::MAX / super::OVERHEAL_DIVISOR, "saturates, never wraps");
        assert!(
            widest >= super::overheal_score(with_hearts(27_000_000, 30_000), 135, 121, 10, 675)
        );
    }

    #[test]
    fn check_overheal_score_val() {
        let rows = [
            (false, false, super::JUDGEMENT[0]),
            (true, false, super::CONVERTED),
            (true, true, super::JUDGEMENT[1]),
        ];
        for (team_strength, trick) in [(0, 0), (1, 7), (80_400, 26_532), (16_000_000, 1)] {
            let worth = super::Worth::new(team_strength, trick);
            for (converted, locked, judgements) in rows {
                let strength = if converted {
                    team_strength.saturating_add(trick)
                } else {
                    team_strength
                };
                for (head, tail) in [(false, false), (false, true), (true, false), (true, true)] {
                    let judgement = judgements[usize::from(head)][usize::from(tail)];
                    let row = worth.row(converted, locked)[usize::from(head)][usize::from(tail)];
                    for heart in [super::HEART_UNIT, 13_380, 30_000] {
                        let with_hearts = u64::from(strength) * u64::from(heart);
                        for (combo, lane, note) in [(100, 100, 1000), (135, 121, 1250)] {
                            let eighths = note / super::NOTE_STEP;
                            assert_eq!(
                                super::overheal_score_val(row, combo, lane, eighths, heart),
                                super::overheal_score(with_hearts, combo, lane, eighths, judgement),
                                "{team_strength} {converted} {locked} {head} {tail} {heart}"
                            );
                        }
                    }
                }
            }
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
        for event in [regular, star, swing, start, release, both] {
            assert_eq!(super::note_eighths(event) * 125, super::note_multiplier(event));
        }
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
