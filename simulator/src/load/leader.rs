//! Most cards have a leader skill, but there's only a few hundred (312)
//! meaingfully unique ones. The main purpose of this module is to extract
//! the unique (leader, extra) skill tuples.

use std::collections::HashMap;

use crate::card::{LeaderExtra, LeaderId, LeaderSkill, LeaderTargetId};
use crate::load::convert::{self, At, ErrorKind, LoadError, Roster};
use crate::load::raw::{RawLeader, RawTargetMap};
use crate::scalar::{Attribute, CharacterSet};

/// A bit of a hacky way to represent no leader skill by applying
/// a boost of 0%
const IDENTITY: LeaderSkill = LeaderSkill {
    attribute: Attribute::Smile,
    secondary: None,
    value: 0,
    extra: None,
};

/// Encodes `leader_extra_target` strings into bitmasks.
/// Should be 74 unique extra targets.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Targets {
    names: Vec<Box<str>>,
    sets: Vec<CharacterSet>,
}

impl Targets {
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    #[must_use]
    pub fn set(&self, id: LeaderTargetId) -> Option<CharacterSet> {
        self.sets.get(id.0 as usize).copied()
    }

    #[must_use]
    pub fn name(&self, id: LeaderTargetId) -> Option<&str> {
        self.names.get(id.0 as usize).map(Box::as_ref)
    }

    fn id(
        &mut self,
        name: &str,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<LeaderTargetId, ErrorKind> {
        if let Some(i) = self.names.iter().position(|n| **n == *name) {
            return Ok(LeaderTargetId(index(i)));
        }
        let set = resolve(name, map, roster)?;
        self.names.push(name.into());
        self.sets.push(set);
        Ok(LeaderTargetId(index(self.names.len() - 1)))
    }
}

/// # Panics
/// Past 255 targets. There are only 74 in cards.json
#[expect(clippy::cast_possible_truncation, reason = "asserted immediately above")]
fn index(i: usize) -> u8 {
    assert!(i < 256, "{i} targets do not fit a LeaderTargetId");
    i as u8
}

/// Target maps to a key in the roster bitmask.
fn resolve(name: &str, map: &RawTargetMap, roster: &Roster) -> Result<CharacterSet, ErrorKind> {
    let members = map.get(name).ok_or_else(|| ErrorKind::Unknown(name.into()))?;
    let mut bits = 0u128;
    for member in members {
        let id = roster.id(member).ok_or_else(|| ErrorKind::Unknown(member.as_str().into()))?;
        bits |= 1 << id.0;
    }
    Ok(CharacterSet(bits))
}
pub struct Table {
    /// Slot 0 is [`IDENTITY`], which is `leader_skill_id` 1 upstream.
    pub by_id: Vec<LeaderSkill>,
    pub targets: Targets,
    seen: HashMap<LeaderSkill, LeaderId>,
}

impl Default for Table {
    fn default() -> Self {
        Self {
            by_id: vec![IDENTITY],
            targets: Targets::default(),
            seen: HashMap::from([(IDENTITY, LeaderId(0))]),
        }
    }
}

impl Table {
    /// # Errors
    /// 1) An attribute or target string outside of its set
    /// 2) a non-exact percentage value
    /// 3) An incomplete / half-filled in leader skill
    ///
    /// # Panics
    /// More than 65535 distinct leader skills. There are only 313 :).
    pub fn insert(
        &mut self,
        card_id: u16,
        raw: &RawLeader,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<LeaderId, LoadError> {
        let Some(attribute) = raw.leader_attribute.as_deref() else {
            return Ok(LeaderId(0));
        };
        let attribute = convert::attribute(attribute).at(card_id, "leader_attribute")?;
        let secondary = raw
            .leader_secondary_attribute
            .as_deref()
            .map(convert::attribute)
            .transpose()
            .at(card_id, "leader_secondary_attribute")?;
        let value = raw.leader_value.ok_or(ErrorKind::Missing).at(card_id, "leader_value")?;
        let value = convert::pct(value).at(card_id, "leader_value")?;

        let extra = self.extra(card_id, raw, map, roster)?;
        let skill = LeaderSkill { attribute, secondary, value, extra };

        if let Some(&id) = self.seen.get(&skill) {
            return Ok(id);
        }
        let slot = u16::try_from(self.by_id.len()).expect("leader table fits u16");
        let id = LeaderId(slot);
        self.by_id.push(skill);
        self.seen.insert(skill, id);
        Ok(id)
    }

    /// Extra is all-or-nothing
    fn extra(
        &mut self,
        card_id: u16,
        raw: &RawLeader,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<Option<LeaderExtra>, LoadError> {
        let e = &raw.extra;
        let (Some(attribute), Some(target), Some(value)) = (
            e.leader_extra_attribute.as_deref(),
            e.leader_extra_target.as_deref(),
            e.leader_extra_value,
        ) else {
            if e.leader_extra_attribute.is_some()
                || e.leader_extra_target.is_some()
                || e.leader_extra_value.is_some()
            {
                return Err(ErrorKind::Missing).at(card_id, "extra");
            }
            return Ok(None);
        };
        Ok(Some(LeaderExtra {
            attribute: convert::attribute(attribute).at(card_id, "leader_extra_attribute")?,
            target: self.targets.id(target, map, roster).at(card_id, "leader_extra_target")?,
            value: convert::pct(value).at(card_id, "leader_extra_value")?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::raw::{self, RawCard, RawUniqueLeader};

    fn read(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        std::fs::read(format!("{path}{name}")).expect("data file")
    }

    fn cards() -> Vec<RawCard> {
        raw::parse(&read("cards.json")).expect("schema")
    }

    fn map() -> RawTargetMap {
        raw::parse_target_map(&read("additional_leader_skill_map.json")).expect("schema")
    }

    fn unique() -> Vec<RawUniqueLeader> {
        raw::parse_unique_leaders(&read("unique_leader_skills.json")).expect("schema")
    }

    fn built() -> (Table, Roster, Vec<(u16, LeaderId)>) {
        let cards = cards();
        let roster = Roster::build(cards.iter().map(|c| c.character.as_str()));
        let map = map();
        let mut table = Table::default();
        let ids = cards
            .iter()
            .map(|c| {
                let id = table.insert(c.card_id, &c.leader_skill, &map, &roster);
                (c.card_id, id.expect("convert"))
            })
            .collect();
        (table, roster, ids)
    }

    #[test]
    fn check_corpus_folds_to_the_expected_tables() {
        let (table, _, ids) = built();
        assert_eq!(table.by_id.len(), 313);
        assert_eq!(table.targets.len(), 74);
        assert_eq!(ids.iter().filter(|(_, id)| *id == LeaderId(0)).count(), 321);
    }

    #[test]
    fn check_identity_occupies_slot_zero() {
        let (table, _, _) = built();
        assert_eq!(table.by_id[0], IDENTITY);
        assert!(
            table.by_id[1..].iter().all(|l| l.value > 0),
            "a real leader skill collided with the identity"
        );
    }

    /// I might need to remove this one later if we want custom cards in the future
    /// but for a snapshot of where SIF was when it shut down, I'm putting this to
    /// make sure that file stays static.
    #[test]
    fn check_unique_leader_skills_json_adds_nothing() {
        let (mut table, roster, _) = built();
        let map = map();
        let file = unique();
        assert_eq!(file.len(), 313);
        for u in &file {
            table
                .insert(u.leader_skill_id, &u.to_leader(), &map, &roster)
                .expect("convert");
        }
        assert_eq!(table.by_id.len(), 313, "the corpus and the reference file disagree");
    }

    /// Dedupe can merge identical leader skills and nothing else.
    #[test]
    fn check_every_card_round_trips_through_the_table() {
        let (table, _, ids) = built();
        for (card, (card_id, id)) in cards().iter().zip(ids) {
            assert_eq!(card.card_id, card_id);
            let got = table.by_id[id.0 as usize];
            let raw = &card.leader_skill;
            let Some(attribute) = raw.leader_attribute.as_deref() else {
                assert_eq!(got, IDENTITY);
                continue;
            };
            assert_eq!(got.attribute, convert::attribute(attribute).expect("attribute"));
            assert_eq!(got.value, convert::pct(raw.leader_value.expect("value")).expect("value"));
            assert_eq!(got.secondary.is_some(), raw.leader_secondary_attribute.is_some());
            assert_eq!(got.extra.is_some(), raw.extra.leader_extra_target.is_some());
            if let Some(extra) = got.extra {
                assert_eq!(
                    table.targets.name(extra.target),
                    raw.extra.leader_extra_target.as_deref()
                );
                assert_eq!(
                    extra.value,
                    convert::pct(raw.extra.leader_extra_value.expect("extra value"))
                        .expect("extra value")
                );
            }
        }
    }

    /// Looks for the cards that boost an attribute by a percentage of a different attribute.
    #[test]
    fn check_cross_attribute_leaders_survive() {
        let (table, _, ids) = built();
        let cross = ids
            .iter()
            .filter(|(_, id)| table.by_id[id.0 as usize].secondary.is_some())
            .count();
        assert_eq!(cross, 87);
    }

    /// Checks that say, "Eli, Umi" and "Aqours" don't resolve to the same
    /// values.
    #[test]
    fn check_targets_resolve_to_distinct_sets() {
        let (table, roster, _) = built();
        let mut sets: Vec<u128> = (0..table.targets.len())
            .map(|i| table.targets.set(LeaderTargetId(index(i))).expect("in range").0)
            .collect();
        assert!(sets.iter().all(|s| *s != 0), "a target resolved to nobody");
        sets.sort_unstable();
        sets.dedup();
        assert_eq!(sets.len(), 74);

        let map = map();
        for (name, members) in [("Chika, Hanamaru", 2), ("Aqours", 9), ("first-year", 19)] {
            let set = resolve(name, &map, &roster).expect("resolve");
            assert_eq!(set.0.count_ones(), members, "{name}");
        }
        let chika = roster.id("Takami Chika").expect("in roster");
        assert!(resolve("Chika, Hanamaru", &map, &roster).expect("resolve").contains(chika));
    }

    /// `Attribute` only ever holds 3 of the 256 values a byte can take, so the
    /// compiler has spare ones to represent `None` with and the optional extra
    /// costs no space at all. pog.
    #[test]
    fn check_optional_extra_costs_nothing() {
        assert_eq!(size_of::<Option<LeaderExtra>>(), size_of::<LeaderExtra>());
        assert_eq!(size_of::<LeaderSkill>(), 6);
    }

    #[test]
    fn check_unknown_target_is_an_error() {
        let (_, roster, _) = built();
        let err = resolve("Nobody, Nohow", &map(), &roster).expect_err("not a target");
        assert_eq!(err, ErrorKind::Unknown("Nobody, Nohow".into()));
    }
}
