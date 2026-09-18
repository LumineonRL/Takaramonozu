//! Most cards have a leader skill, but there's only a few hundred (312)
//! meaingfully unique ones. The main purpose of this module is to extract a
//! card's leader skill to one of them.

use std::collections::HashMap;

use simulator::card::{LeaderExtra, LeaderId, LeaderSkill, LeaderTargets};
use simulator::scalar::{Attribute, Roster};

use crate::load::convert::{self, At, Culprit, ErrorKind, LoadError};
use crate::load::raw::{RawLeader, RawTargetMap, RawUniqueLeader};

/// A bit of a hacky way to represent no leader skill by applying
/// a boost of 0%
const IDENTITY: LeaderSkill = LeaderSkill {
    attribute: Attribute::Smile,
    secondary: None,
    value: 0,
    extra: None,
};

pub struct Table {
    /// Slot 0 is [`IDENTITY`], which is `leader_skill_id` 1 upstream.
    pub by_id: Vec<LeaderSkill>,
    pub targets: LeaderTargets,
    seen: HashMap<LeaderSkill, LeaderId>,
}

impl Table {
    /// # Errors
    /// I'm so tired of writing error docs pedantic clippy
    /// was a mistake. It errors if the id is bad, in
    /// the exact same way that every other Trait in this
    /// codebase does!
    pub fn seeded(
        unique: &[RawUniqueLeader],
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<Self, LoadError> {
        let mut table = Self {
            by_id: Vec::with_capacity(unique.len()),
            targets: LeaderTargets::default(),
            seen: HashMap::with_capacity(unique.len()),
        };
        for (i, u) in unique.iter().enumerate() {
            let who = Culprit::Leader(u.leader_skill_id);
            let id = table.next_id(&who)?;
            if usize::from(u.leader_skill_id) != i.saturating_add(1) {
                return Err(ErrorKind::Invariant).at(&who, "leader_skill_id");
            }
            let skill = table.convert(&who, &u.to_leader(), map, roster)?;
            if (id == LeaderId(0)) != (skill == IDENTITY) {
                return Err(ErrorKind::Invariant).at(&who, "leader_attribute");
            }
            if table.seen.insert(skill, id).is_some() {
                return Err(ErrorKind::Invariant).at(&who, "extra");
            }
            table.by_id.push(skill);
        }
        Ok(table)
    }

    /// # Errors
    /// Past 65535 leader skills, which is what a [`LeaderId`] holds. There are
    /// 313.
    fn next_id(&self, who: &Culprit) -> Result<LeaderId, LoadError> {
        u16::try_from(self.by_id.len())
            .map(LeaderId)
            .map_err(|_ignored| ErrorKind::Invariant)
            .at(who, "leader_skill_id")
    }

    /// The card's leader skill, as a row of the seeded table.
    ///
    /// # Errors
    /// 1) An attribute or target string outside of its set
    /// 2) a non-exact percentage value
    /// 3) An incomplete / half-filled in leader skill
    /// 4) a skill that `unique_leader_skills.json` does not carry
    pub fn resolve(
        &mut self,
        card_id: u16,
        raw: &RawLeader,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<LeaderId, LoadError> {
        if raw.leader_attribute.is_none() {
            return Ok(LeaderId(0));
        }
        let who = Culprit::Card(card_id);
        let skill = self.convert(&who, raw, map, roster)?;
        self.seen
            .get(&skill)
            .copied()
            .ok_or(ErrorKind::Invariant)
            .at(&who, "leader_skill")
    }

    fn convert(
        &mut self,
        who: &Culprit,
        raw: &RawLeader,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<LeaderSkill, LoadError> {
        let Some(attribute) = raw.leader_attribute.as_deref() else {
            return Ok(IDENTITY);
        };
        let attribute = convert::attribute(attribute).at(who, "leader_attribute")?;
        let secondary = raw
            .leader_secondary_attribute
            .as_deref()
            .map(convert::attribute)
            .transpose()
            .at(who, "leader_secondary_attribute")?;
        let value = raw.leader_value.ok_or(ErrorKind::Missing).at(who, "leader_value")?;
        let value = convert::pct(value).at(who, "leader_value")?;

        let extra = self.extra(who, raw, map, roster)?;
        Ok(LeaderSkill { attribute, secondary, value, extra })
    }

    /// Extra is all-or-nothing
    fn extra(
        &mut self,
        who: &Culprit,
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
                return Err(ErrorKind::Missing).at(who, "extra");
            }
            return Ok(None);
        };
        Ok(Some(LeaderExtra {
            attribute: convert::attribute(attribute).at(who, "leader_extra_attribute")?,
            target: self
                .targets
                .intern(target, || convert::character_set(target, map, roster))
                .at(who, "leader_extra_target")?,
            value: convert::pct(value).at(who, "leader_extra_value")?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simulator::card::LeaderTargetId;

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
        let mut table = Table::seeded(&unique(), &map, &roster).expect("seed");
        let ids = cards
            .iter()
            .map(|c| {
                let id = table.resolve(c.card_id, &c.leader_skill, &map, &roster);
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

    #[test]
    fn check_leader_ids_are_the_upstream_ids() {
        let (table, _, ids) = built();
        let file = unique();
        assert_eq!(table.by_id.len(), file.len());
        for (i, u) in file.iter().enumerate() {
            assert_eq!(usize::from(u.leader_skill_id), i + 1, "row {i} is not dense");
            let got = table.by_id[i];
            let Some(attribute) = u.leader_attribute.as_deref() else {
                assert_eq!(got, IDENTITY, "row {i}");
                continue;
            };
            assert_eq!(got.attribute, convert::attribute(attribute).expect("attribute"), "row {i}");
            assert_eq!(
                got.value,
                convert::pct(u.leader_value.expect("value")).expect("value"),
                "row {i}"
            );
            assert_eq!(
                got.extra.map(|e| e.value),
                u.extra.leader_extra_value.map(|v| convert::pct(v).expect("extra value")),
                "row {i}"
            );
        }
        assert!(
            ids.iter().all(|(_, id)| usize::from(id.0) < file.len()),
            "a card resolved past the file"
        );
    }

    /// I might need to remove this one later if we want custom cards in the future
    /// but for a snapshot of where SIF was when it shut down, this is what keeps
    /// that file static: the corpus may not hold a leader skill it does not list.
    #[test]
    fn check_a_leader_skill_outside_the_file_is_rejected() {
        let roster = Roster::build(cards().iter().map(|c| c.character.as_str()));
        let map = map();
        let mut file = unique();
        let dropped = file.pop().expect("313 rows").to_leader();
        let mut table = Table::seeded(&file, &map, &roster).expect("seed");

        let err = table.resolve(66, &dropped, &map, &roster).expect_err("not in the file");
        assert_eq!(err.culprit, Culprit::Card(66));
        assert_eq!(err.field, "leader_skill");
        assert_eq!(err.kind, ErrorKind::Invariant);
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
            .map(|i| table.targets.set(LeaderTargetId::from_index(i)).expect("in range").0)
            .collect();
        assert!(sets.iter().all(|s| *s != 0), "a target resolved to nobody");
        sets.sort_unstable();
        sets.dedup();
        assert_eq!(sets.len(), 74);

        let map = map();
        for (name, members) in [("Chika, Hanamaru", 2), ("Aqours", 9), ("first-year", 19)] {
            let set = convert::character_set(name, &map, &roster).expect("resolve");
            assert_eq!(set.0.count_ones(), members, "{name}");
        }
        let chika = roster.id("Takami Chika").expect("in roster");
        let pair = convert::character_set("Chika, Hanamaru", &map, &roster).expect("resolve");
        assert!(pair.contains(chika));
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
        let err =
            convert::character_set("Nobody, Nohow", &map(), &roster).expect_err("not a target");
        assert_eq!(err, ErrorKind::Unknown("Nobody, Nohow".into()));
    }
}
