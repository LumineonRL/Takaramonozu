//! Definitions for converting the raw .json data files into .rkyv

use core::fmt;

use simulator::scalar::{Attribute, Centi, CharacterSet, Milli, Ms, Pct, Rarity, Roster};
use simulator::sis::SisEffect;
use simulator::skill::{SkillEffect, TriggerType};
use simulator::song::{Difficulty, Time};

use crate::load::raw::RawTargetMap;

#[derive(Clone, PartialEq, Debug)]
pub struct LoadError {
    pub culprit: Culprit,
    pub field: &'static str,
    pub kind: ErrorKind,
}

/// Which record is to blame. Used for Error messages.
/// card/acc/sis are just ids, song charts are the composite (title, diff)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Culprit {
    Card(u16),
    Accessory(u16),
    Sis(u16),
    Chart(Box<str>),
}

impl From<&str> for Culprit {
    fn from(key: &str) -> Self {
        Self::Chart(key.into())
    }
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
        match &self.culprit {
            Culprit::Card(id) => write!(f, "card {id}, ")?,
            Culprit::Accessory(id) => write!(f, "accessory {id}, ")?,
            Culprit::Sis(id) => write!(f, "SIS {id}, ")?,
            Culprit::Chart(key) => write!(f, "chart {key}, ")?,
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
    fn at(self, culprit: impl Into<Culprit>, field: &'static str) -> Result<T, LoadError>;
}

impl<T> At<T> for Result<T, ErrorKind> {
    fn at(self, culprit: impl Into<Culprit>, field: &'static str) -> Result<T, LoadError> {
        self.map_err(|kind| LoadError { culprit: culprit.into(), field, kind })
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

closed_set!(sis_effect -> SisEffect {
    "self flat boost" => SisEffect::SelfFlat,
    "self percent boost" => SisEffect::SelfPercent,
    "all percent boost" => SisEffect::AllPercent,
    "charm" => SisEffect::Charm,
    "heal" => SisEffect::Heal,
    "trick" => SisEffect::Trick,
});

closed_set!(difficulty -> Difficulty {
    "Easy" => Difficulty::Easy,
    "Normal" => Difficulty::Normal,
    "Hard" => Difficulty::Hard,
    "Expert" => Difficulty::Expert,
    "Master" => Difficulty::Master,
});

closed_set!(trigger -> TriggerType {
    "Rhythm Icons" => TriggerType::RhythmIcons,
    "Perfects" => TriggerType::Perfects,
    "Combo" => TriggerType::Combo,
    "Time" => TriggerType::Time,
    "Score" => TriggerType::Score,
    "Star Notes" => TriggerType::StarNotes,
    "Year Group" => TriggerType::YearGroup,
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
#[expect(clippy::float_arithmetic, reason = "the only float arithmetic in the crate")]
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
pub fn ms(value: f64) -> Result<Ms, ErrorKind> {
    scaled(value, 1000.0)
}

/// Thousandths, for the SIS `value` column: `0.015 -> 15` or `1400.0 -> 1400000`.
/// Basically it's the same factor as [`ms`] onto a different width,
/// which is why the two are named for their types rather than the scale they share.
///
/// # Errors
/// More than three decimal places, or past `Milli`.
pub fn milli(value: f64) -> Result<Milli, ErrorKind> {
    scaled(value, 1000.0)
}

/// # Errors
/// More than three decimal places, or past whatever the max f64 is I don't feel
/// like looking it up.
pub fn time(value: f64) -> Result<Time, ErrorKind> {
    scaled::<u32>(value, 1000.0).map(Time)
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

/// # Errors
/// A name that is neither `"All"`, a key of the map, nor a valid member name.
pub fn accessory_target(
    name: &str,
    map: &RawTargetMap,
    roster: &Roster,
) -> Result<CharacterSet, ErrorKind> {
    if name == "All" {
        return Ok(roster.all());
    }
    if map.contains_key(name) {
        return character_set(name, map, roster);
    }
    singleton(name, roster)
}

/// One character as a one-bit set.
///
/// # Errors
/// Invalid character name.
pub fn singleton(name: &str, roster: &Roster) -> Result<CharacterSet, ErrorKind> {
    let id = roster.id(name).ok_or_else(|| ErrorKind::Unknown(name.into()))?;
    Ok(CharacterSet(1 << id.0))
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
                ms(*v).expect("duration");
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
        assert_eq!(ms(0.05), Ok(50));
        assert_eq!(ms(0.0005), Err(ErrorKind::NotFixedPoint(0.0005)));
        assert_eq!(milli(0.015), Ok(15)); // the third decimal `Centi` cannot hold
        assert_eq!(milli(1400.0), Ok(1_400_000));
        assert_eq!(centi(148_240.0), Ok(14_824_000));
        assert_eq!(pct(3.0), Err(ErrorKind::OutOfRange(3.0))); // 300 past `Pct`
        assert_eq!(ms(99.0), Err(ErrorKind::OutOfRange(99.0))); // 99 s past `Ms`

        // Longest song in the game
        assert_eq!(time(174.718), Ok(Time(174_718)));
        assert_eq!(ms(174.718), Err(ErrorKind::OutOfRange(174.718)));
        assert_eq!(centi(-1.0), Err(ErrorKind::OutOfRange(-1.0)));
        assert_eq!(centi(1e12), Err(ErrorKind::OutOfRange(1e12))); // past the u32 gate

        assert_matches!(rarity("SSSR"), Err(ErrorKind::Unknown(s)) if &*s == "SSSR");
    }

    /// Tests all targets and regular character name targets
    #[test]
    fn check_accessory_targetss() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        let read = |n: &str| std::fs::read(format!("{path}{n}")).expect("data file");
        let accs = raw::parse_accessories(&read("accessories.json")).expect("schema");
        let map = raw::parse_target_map(&read("year_group_target.json")).expect("schema");
        let roster = Roster::build(cards().iter().map(|c| c.character.as_str()));

        let all = accessory_target("All", &map, &roster).expect("All");
        assert_eq!(all.0.count_ones() as usize, roster.len());
        assert_eq!(all, roster.all());
        let file = character_set("All", &map, &roster).expect("the map's own key");
        assert_eq!(file.0.count_ones(), 39, "the file's key is what we are overriding");
        assert_ne!(all, file);

        let mut singletons = 0;
        for a in accs.values() {
            let Some(t) = a.skill.target() else { continue };
            let set = accessory_target(t, &map, &roster).expect("accessory target");
            if t != "All" {
                // Character target should always name its own character.
                assert_eq!(set.0.count_ones(), 1, "{t}");
                assert!(set.contains(roster.id(t).expect("in roster")));
                assert_matches!(character_set(t, &map, &roster), Err(ErrorKind::Unknown(_)));
                singletons += 1;
            }
        }
        assert_eq!(singletons, 30);
        assert_matches!(accessory_target("A-RISE", &map, &roster), Err(ErrorKind::Unknown(s))
            if &*s == "A-RISE");
    }

    #[test]
    fn check_errors_carry_the_card_and_field() {
        let err = pct(0.935).at(Culprit::Card(66), "chance").expect_err("not fixed point");
        assert_matches!(err, LoadError { culprit: Culprit::Card(66), field: "chance", .. });
        assert_eq!(err.to_string(), "card 66, field `chance`: 0.935 is not exact in this unit");

        let wide = pct(3.0).at(Culprit::Card(66), "chance").expect_err("out of range");
        assert_eq!(wide.to_string(), "card 66, field `chance`: 3 does not fit this field");
    }
}
