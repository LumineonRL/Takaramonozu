// So the goal is convert cards.json into an rkyv file.
// But we need to load that in first with serde.

use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawCard {
    pub card_id: u16,
    pub display_name: String,
    pub rarity: String,
    pub attribute: String,
    pub character: String,
    pub is_promo: bool,
    pub is_preidolized_non_promo: bool,
    pub stats: RawForms,
    pub leader_skill: RawLeader,
    pub skill: RawSkill,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawForms {
    pub unidolized: RawStats,
    pub idolized: RawStats,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawStats {
    pub smile: u16,
    pub pure: u16,
    pub cool: u16,
    pub sis_base: u8,
    pub sis_max: u8,
    // Image is a TODO item for the eventual frontend, but has
    // no bearing on the simulator itself.
    pub image: IgnoredAny,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawLeader {
    pub leader_attribute: Option<String>,
    pub leader_secondary_attribute: Option<String>,
    pub leader_value: Option<f64>,
    pub extra: RawLeaderExtra,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawLeaderExtra {
    pub leader_extra_attribute: Option<String>,
    pub leader_extra_target: Option<String>,
    pub leader_extra_value: Option<f64>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawSkill {
    // Always `1..=len`, so it carries nothing the length does not.
    pub level: Vec<u8>,
    // This one is just "" in `cards.json` when empty, not null. `cards.json` is very inconsistent but I don't think
    // it's worth fixing.
    #[serde(rename = "type")]
    pub effect: String,
    pub activation: String,
    pub threshold: Vec<Option<f64>>,
    pub chance: Vec<f64>,
    pub value: Vec<Option<f64>>,
    pub duration: Vec<Option<f64>>,
    pub target: Option<String>,
}

impl RawSkill {
    #[must_use]
    pub fn is_present(&self) -> bool {
        !self.effect.is_empty()
    }

    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.target
            .as_deref()
            .filter(|t| !t.is_empty())
    }
}

/// # Errors
/// Any generic error that could occur from any of the above functions.
pub fn parse(bytes: &[u8]) -> serde_json::Result<Vec<RawCard>> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cards() -> Vec<RawCard> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/cards.json");
        let bytes = std::fs::read(path).expect("cards.json");
        parse(&bytes).expect("schema")
    }

    #[test]
    fn check_card_count() {
        let cards = cards();
        assert_eq!(cards.len(), 3826);
        let mut ids: Vec<u16> = cards.iter().map(|c| c.card_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3826);
    }

    #[test]
    fn check_skill_format() {
        for c in cards() {
            let s = &c.skill;
            let rows = s.level.len();
            assert_eq!(s.is_present(), rows > 0, "card {}", c.card_id);
            assert!(
                s.threshold.len() == rows
                    && s.chance.len() == rows
                    && s.value.len() == rows
                    && s.duration.len() == rows,
                "card {} ragged",
                c.card_id
            );
            let expected: Vec<u8> = (1..=rows as u8).collect();
            assert_eq!(s.level, expected, "card {}", c.card_id);
        }
    }

    /// Makes sure that there's no nulls in the middle of an array that has non-null values.
    #[test]
    fn check_unexpected_nulls() {
        for c in cards() {
            let s = &c.skill;
            for (name, col) in [
                ("threshold", &s.threshold),
                ("value", &s.value),
                ("duration", &s.duration),
            ] {
                let nulls = col.iter().filter(|v| v.is_none()).count();
                assert!(
                    nulls == 0 || nulls == col.len(),
                    "card {} has a mixed {name} column",
                    c.card_id
                );
            }
        }
    }
}