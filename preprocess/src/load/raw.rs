// So the goal is convert cards.json into an rkyv file.
// But we need to load that in first with serde.

use std::collections::HashMap;

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

/// One entry of `unique_leader_skills.json`.
///
/// The same shape as [`RawLeader`] with an id bolted on. Kept separate rather
/// than flattened because `serde(flatten)` and `deny_unknown_fields` do not
/// compose.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct RawUniqueLeader {
    pub leader_skill_id: u16,
    pub leader_attribute: Option<String>,
    pub leader_secondary_attribute: Option<String>,
    pub leader_value: Option<f64>,
    pub extra: RawLeaderExtra,
}

impl RawUniqueLeader {
    /// Clones, which is irrelevant on a 313-entry cross-check that runs offline.
    #[must_use]
    pub fn to_leader(&self) -> RawLeader {
        RawLeader {
            leader_attribute: self.leader_attribute.clone(),
            leader_secondary_attribute: self.leader_secondary_attribute.clone(),
            leader_value: self.leader_value,
            extra: RawLeaderExtra {
                leader_extra_attribute: self.extra.leader_extra_attribute.clone(),
                leader_extra_target: self.extra.leader_extra_target.clone(),
                leader_extra_value: self.extra.leader_extra_value,
            },
        }
    }
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
        self.target.as_deref().filter(|t| !t.is_empty())
    }
}

/// `additional_leader_skill_map.json` maps a `leader_extra_target` name to the
/// character names it corresponds to.
pub type RawTargetMap = HashMap<String, Vec<String>>;

/// # Errors
/// Any deviation from the schema above, including an unknown field.
pub fn parse(bytes: &[u8]) -> serde_json::Result<Vec<RawCard>> {
    serde_json::from_slice(bytes)
}

/// # Errors
/// Anything that is not an object of string arrays.
pub fn parse_target_map(bytes: &[u8]) -> serde_json::Result<RawTargetMap> {
    serde_json::from_slice(bytes)
}

/// # Errors
/// Any deviation from [`RawUniqueLeader`].
pub fn parse_unique_leaders(bytes: &[u8]) -> serde_json::Result<Vec<RawUniqueLeader>> {
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
    fn parses_every_card_with_unique_ids() {
        let cards = cards();
        assert_eq!(cards.len(), 3826);
        let mut ids: Vec<u16> = cards.iter().map(|c| c.card_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3826);
    }

    /// The parallel arrays are a table, and `level` is only ever its row numbers.
    #[test]
    fn skill_arrays_are_parallel_and_level_is_positional() {
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
            #[expect(clippy::cast_possible_truncation, reason = "rows <= 16")]
            let expected: Vec<u8> = (1..=rows as u8).collect();
            assert_eq!(s.level, expected, "card {}", c.card_id);
        }
    }

    #[test]
    fn leader_null_patterns_are_structural() {
        let mut identity = 0;
        for c in cards() {
            let l = &c.leader_skill;
            let e = &l.extra;
            let extras = [
                e.leader_extra_attribute.is_some(),
                e.leader_extra_target.is_some(),
                e.leader_extra_value.is_some(),
            ];
            assert!(
                extras == [true; 3] || extras == [false; 3],
                "card {} has a partial leader extra",
                c.card_id
            );
            if l.leader_attribute.is_none() {
                assert!(
                    l.leader_value.is_none()
                        && l.leader_secondary_attribute.is_none()
                        && extras == [false; 3],
                    "card {} has no leader attribute but carries leader data",
                    c.card_id
                );
                identity += 1;
            } else {
                assert!(l.leader_value.is_some(), "card {} has no leader value", c.card_id);
            }
        }
        assert_eq!(identity, 321, "the identity leader belongs to the N cards alone");
    }

    #[test]
    fn null_columns_are_all_or_nothing() {
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
