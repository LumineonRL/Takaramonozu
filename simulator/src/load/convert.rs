//! Definitions for converting the raw .json data files into .rkyv

use core::fmt;

use crate::load::raw::RawTargetMap;
use crate::scalar::{Attribute, Centi, CharacterSet, Millis, Pct, Rarity, Roster};
use crate::skill::{SkillEffect, SkillTrigger};

#[derive(Clone, PartialEq, Debug)]
pub struct LoadError {
    /// `None` for larger failures that no single card would own.
    pub card_id: Option<u16>,
    pub field: &'static str,
    pub kind: ErrorKind,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ErrorKind {
    Unknown(Box<str>),
    /// Not a whole number once scaled, so shape is wrong.
    NotFixedPoint(f64),
    /// Exact, but past the width the field is stored in.
    OutOfRange(f64),
    /// A field a sibling field made mandatory. The leader `extra` triple is
    /// all-or-nothing, and a `leader_attribute` implies a `leader_value`.
    Missing,
    /// A value that's theoretically possible but violates the rules that exist in cards.json.
    /// Like if an N had an 8 SIS max or something.
    Invariant,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = self.card_id {
            write!(f, "card {id}, ")?;
        }
        write!(f, "field `{}`: ", self.field)?;
        match &self.kind {
            #[expect(clippy::use_debug, reason = "quotes the offending token")]
            ErrorKind::Unknown(s) => write!(f, "unknown value {s:?}"),
            ErrorKind::NotFixedPoint(v) => write!(f, "{v} is not exact in this unit"),
            ErrorKind::OutOfRange(v) => write!(f, "{v} does not fit this field"),
            ErrorKind::Missing => write!(f, "absent, but a sibling field requires it"),
            ErrorKind::Invariant => write!(f, "contradicts a known invariant of the source"),
        }
    }
}

impl core::error::Error for LoadError {}

/// Where a conversion failed.
pub trait At<T> {
    /// # Errors
    /// The one being annotated, if there is one.
    fn at(self, card_id: u16, field: &'static str) -> Result<T, LoadError>;
}

impl<T> At<T> for Result<T, ErrorKind> {
    fn at(self, card_id: u16, field: &'static str) -> Result<T, LoadError> {
        self.map_err(|kind| LoadError { card_id: Some(card_id), field, kind })
    }
}

/// All data is frozen since the game is ded, so the table below is the whole
/// function and an unlisted string is a schema change rather than a value.
macro_rules! closed_set {
    ($name:ident -> $ty:ty { $($text:literal => $variant:expr),+ $(,)? }) => {
        /// # Errors
        /// A string that isn't in the table.
        pub fn $name(s: &str) -> Result<$ty, ErrorKind> {
            Ok(match s {
                $($text => $variant,)+
                _ => return Err(ErrorKind::Unknown(s.into())),
            })
        }
    };
}

closed_set!(rarity -> Rarity {
    "N" => Rarity::N,
    "R" => Rarity::R,
    "SR" => Rarity::SR,
    "SSR" => Rarity::SSR,
    "UR" => Rarity::UR,
});

closed_set!(attribute -> Attribute {
    "Smile" => Attribute::Smile,
    "Pure" => Attribute::Pure,
    "Cool" => Attribute::Cool,
});

// `Total Trick` is card-only and `Spark` accessory-only.
closed_set!(effect -> SkillEffect {
    "Scorer" => SkillEffect::Scorer,
    "Healer" => SkillEffect::Healer,
    "Perfect Lock" => SkillEffect::PerfectLock,
    "Encore" => SkillEffect::Encore,
    "Amplify" => SkillEffect::Amplify,
    "Total Trick" => SkillEffect::TotalTrick,
    "Perfect Score Up" => SkillEffect::PerfectScoreUp,
    "Skill Rate Up" => SkillEffect::SkillRateUp,
    "Appeal Boost" => SkillEffect::AppealBoost,
    "Combo Bonus Up" => SkillEffect::ComboBonusUp,
    "Sync" => SkillEffect::Sync,
    "Spark" => SkillEffect::Spark,
});

closed_set!(trigger -> SkillTrigger {
    "Rhythm Icons" => SkillTrigger::RhythmIcons,
    "Perfects" => SkillTrigger::Perfects,
    "Combo" => SkillTrigger::Combo,
    "Time" => SkillTrigger::Time,
    "Score" => SkillTrigger::Score,
    "Star Notes" => SkillTrigger::StarNotes,
    "Year Group" => SkillTrigger::YearGroup,
});

/// Exact or nothing. Every scaled value in `cards.json` is whole, so a fraction
/// here means the column is not the correct shape in the first place.
///
/// In practice the rounding / truncation issues we might observe from
/// multiplying, say `0.56 * 100` should never be significant enough to
/// actually affect the value being used during the sim.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "range checked immediately above"
)]
#[expect(clippy::map_err_ignore, reason = "`TryFromIntError` already reports this")]
fn scaled<T>(value: f64, factor: f64) -> Result<T, ErrorKind>
where
    T: TryFrom<u32>,
{
    let scaled = value * factor;
    let whole = scaled.round();
    if (scaled - whole).abs() > 1e-6 {
        return Err(ErrorKind::NotFixedPoint(value));
    }
    if !(0.0..=f64::from(u32::MAX)).contains(&whole) {
        return Err(ErrorKind::OutOfRange(value));
    }
    T::try_from(whole as u32).map_err(|_| ErrorKind::OutOfRange(value))
}

/// Hundredths, for skills like `value`: `3.85 -> 385` or `400.0 -> 40000`.
/// # Errors
/// More than two decimal places, or past `Centi`.
pub fn centi(value: f64) -> Result<Centi, ErrorKind> {
    scaled(value, 100.0)
}

/// A fraction as percent, for activation chance and leader values: `0.93 -> 93`.
/// # Errors
/// More than two decimal places, or past 255%.
pub fn pct(value: f64) -> Result<Pct, ErrorKind> {
    scaled(value, 100.0)
}

/// Seconds with one decimal, for durations: `4.5 -> 4500`.
/// # Errors
/// More than three decimal places, or longer than 65.535 s.
pub fn millis(value: f64) -> Result<Millis, ErrorKind> {
    scaled(value, 1000.0)
}

/// # Errors
/// A fractional threshold, or one past 65535.
pub fn threshold(value: f64) -> Result<u16, ErrorKind> {
    scaled(value, 1.0)
}

/// A target name over the roster bitmask it covers.
///
/// Used for leader extras from `additional_leader_skill_map.json`
/// and skill targets from `year_group_target.json`.
///
/// # Errors
/// A name the map does not carry, or a member outside the roster.
pub fn character_set(
    name: &str,
    map: &RawTargetMap,
    roster: &Roster,
) -> Result<CharacterSet, ErrorKind> {
    let members = map.get(name).ok_or_else(|| ErrorKind::Unknown(name.into()))?;
    let mut bits = 0_u128;
    for member in members {
        let id = roster.id(member).ok_or_else(|| ErrorKind::Unknown(member.as_str().into()))?;
        bits |= 1 << id.0;
    }
    Ok(CharacterSet(bits))
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

    use super::*;
    use crate::load::raw::{self, RawCard};

    fn cards() -> Vec<RawCard> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/cards.json");
        raw::parse(&std::fs::read(path).expect("cards.json")).expect("schema")
    }

    #[test]
    fn check_every_card_string_is_in_its_set() {
        for c in &cards() {
            rarity(&c.rarity).expect("rarity");
            attribute(&c.attribute).expect("attribute");
            let attributes = [
                &c.leader_skill.leader_attribute,
                &c.leader_skill.leader_secondary_attribute,
                &c.leader_skill.extra.leader_extra_attribute,
            ];
            for s in attributes.into_iter().flatten() {
                attribute(s).expect("leader attribute");
            }
            if c.skill.is_present() {
                effect(&c.skill.effect).expect("effect");
                trigger(&c.skill.activation).expect("activation");
            }
        }
    }

    #[test]
    fn check_every_float_is_exact_fixed_point() {
        for c in &cards() {
            let s = &c.skill;
            let leader = [c.leader_skill.leader_value, c.leader_skill.extra.leader_extra_value];
            for v in s.chance.iter().copied().chain(leader.into_iter().flatten()) {
                pct(v).expect("chance or leader value");
            }
            for v in s.value.iter().flatten() {
                centi(*v).expect("value");
            }
            for v in s.duration.iter().flatten() {
                millis(*v).expect("duration");
            }
            for v in s.threshold.iter().flatten() {
                threshold(*v).expect("threshold");
            }
        }
    }

    #[test]
    fn check_roster_is_sorted_correctly() {
        let cards = cards();
        let roster = Roster::build(cards.iter().map(|c| c.character.as_str()));
        assert_eq!(roster.len(), 99);
        for c in &cards {
            let id = roster.id(&c.character).expect("character present");
            assert_eq!(roster.name(id), Some(c.character.as_str()));
        }
        assert_eq!(roster.id("Nobody"), None);
    }

    #[test]
    fn check_reject_impossible_values() {
        assert_eq!(pct(0.56), Ok(56)); // the float that is not 56 exactly
        assert_eq!(pct(0.935), Err(ErrorKind::NotFixedPoint(0.935)));
        assert_eq!(millis(0.05), Ok(50));
        assert_eq!(millis(0.0005), Err(ErrorKind::NotFixedPoint(0.0005)));
        assert_eq!(centi(148_240.0), Ok(14_824_000));
        assert_eq!(pct(3.0), Err(ErrorKind::OutOfRange(3.0))); // 300 past `Pct`
        assert_eq!(millis(99.0), Err(ErrorKind::OutOfRange(99.0))); // 99 s past `Millis`
        assert_eq!(centi(-1.0), Err(ErrorKind::OutOfRange(-1.0)));
        assert_eq!(centi(1e12), Err(ErrorKind::OutOfRange(1e12))); // past the u32 gate

        assert_matches!(rarity("SSSR"), Err(ErrorKind::Unknown(s)) if &*s == "SSSR");
    }

    #[test]
    fn check_errors_carry_the_card_and_field() {
        let err = pct(0.935).at(66, "chance").expect_err("not fixed point");
        assert_matches!(err, LoadError { card_id: Some(66), field: "chance", .. });
        assert_eq!(err.to_string(), "card 66, field `chance`: 0.935 is not exact in this unit");

        let wide = pct(3.0).at(66, "chance").expect_err("out of range");
        assert_eq!(wide.to_string(), "card 66, field `chance`: 3 does not fit this field");
    }
}
