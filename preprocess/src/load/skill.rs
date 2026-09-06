//! Folds the 3505 card skills into a [`SkillArena`] of 2460 distinct ones,
//! plus an unused entry at slot 0 so that a card with no skill costs nothing
//! to store. See [`SkillId`] for why that works.

use std::collections::HashMap;

use simulator::scalar::{Centi, Ms, Roster};
use simulator::skill::{
    SkillArena, SkillEffect, SkillId, TRIGGER_ROWS, TargetId, Targets, Trigger, TriggerType,
};

use crate::load::convert::{self, At, ErrorKind, LoadError};
use crate::load::raw::{RawSkill, RawTargetMap};

/// The number of rows a card's trigger half occupies (aka the first 8).
const ROWS: usize = TRIGGER_ROWS as usize;

#[derive(PartialEq, Eq, Hash)]
struct Key {
    effect: SkillEffect,
    trigger_type: TriggerType,
    target: Option<TargetId>,
    triggers: [Trigger; ROWS],
    payload: Vec<(Centi, Ms)>,
}

#[derive(Default)]
pub struct Table {
    pub arena: SkillArena,
    pub targets: Targets,
    seen: HashMap<Key, SkillId>,
}

impl Table {
    /// # Errors
    /// Any field that does not convert
    /// # Panics
    /// Past 65535 distinct skills or 255 targets, neither of which the frozen
    /// data comes close to.
    pub fn insert(
        &mut self,
        card_id: u16,
        raw: &RawSkill,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<Option<SkillId>, LoadError> {
        if !raw.is_present() {
            return Ok(None);
        }
        let effect = convert::effect(&raw.effect).at(card_id, "type")?;
        let trigger_type = convert::trigger(&raw.activation).at(card_id, "activation")?;
        let target = raw
            .target()
            .map(|t| self.targets.intern(t, || convert::character_set(t, map, roster)))
            .transpose()
            .at(card_id, "target")?;

        let rows = raw.value.len();
        if rows < ROWS
            || raw.threshold.len() != rows
            || raw.chance.len() != rows
            || raw.duration.len() != rows
        {
            return Err(ErrorKind::Invariant).at(card_id, "rows");
        }

        let mut triggers = [Trigger::default(); ROWS];
        for ((out, &threshold), &chance) in triggers.iter_mut().zip(&raw.threshold).zip(&raw.chance)
        {
            // Six skills have no threshold. Star Notes rolls on every star note,
            // and the five Year Group ones fire on a set cover a counter cannot
            // express, so nothing sensible goes here.
            out.threshold = match (threshold, trigger_type) {
                (Some(t), _) => convert::threshold(t).at(card_id, "threshold")?,
                (None, TriggerType::YearGroup) => 0,
                (None, _) => 1,
            };
            out.chance = convert::pct(chance).at(card_id, "chance")?;
        }

        // Perfect Lock, Total Trick, Sync and Encore carry no value.
        // Stores 0 rather than an `Option`
        let mut payload = Vec::with_capacity(rows);
        for (&value, &duration) in raw.value.iter().zip(&raw.duration) {
            payload.push((
                match value {
                    Some(v) => convert::centi(v).at(card_id, "value")?,
                    None => 0,
                },
                match duration {
                    Some(d) => convert::ms(d).at(card_id, "duration")?,
                    None => 0,
                },
            ));
        }

        let key = Key { effect, trigger_type, target, triggers, payload };
        if let Some(&id) = self.seen.get(&key) {
            return Ok(Some(id));
        }
        let id = self.arena.push(effect, trigger_type, target, &key.triggers, &key.payload);
        self.seen.insert(key, id);
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

    use simulator::skill::SkillRows;

    use super::*;
    use crate::load::raw::{self, RawCard};

    fn read(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        std::fs::read(format!("{path}{name}")).expect("data file")
    }

    fn cards() -> Vec<RawCard> {
        raw::parse(&read("cards.json")).expect("schema")
    }

    fn map() -> RawTargetMap {
        raw::parse_target_map(&read("year_group_target.json")).expect("schema")
    }

    fn built() -> (Table, Roster, Vec<(u16, Option<SkillId>)>) {
        let cards = cards();
        let roster = Roster::build(cards.iter().map(|c| c.character.as_str()));
        let map = map();
        let mut table = Table::default();
        let ids = cards
            .iter()
            .map(|c| {
                let id = table.insert(c.card_id, &c.skill, &map, &roster);
                (c.card_id, id.expect("convert"))
            })
            .collect();
        (table, roster, ids)
    }

    #[test]
    fn check_corpus_folds_to_the_expected_tables() {
        let (table, _, ids) = built();
        assert_eq!(ids.iter().filter(|(_, id)| id.is_none()).count(), 321);
        // 2460 distinct skills plus the dead slot 0.
        assert_eq!(table.arena.defs().len(), 2461);
        assert_eq!(table.arena.trigger_rows(), 2460 * ROWS);
        assert_eq!(table.targets.len(), 14);
    }

    /// Every card must read back out of the arena what its own rows said, which
    /// is what dedupe is allowed to preserve and nothing else.
    #[test]
    fn check_every_card_round_trips_through_the_arena() {
        let (table, _, ids) = built();
        for (card, (card_id, id)) in cards().iter().zip(ids) {
            assert_eq!(card.card_id, card_id);
            let Some(id) = id else { continue };
            let def = table.arena.def(id).expect("push handed this id out");
            let raw = &card.skill;
            assert_eq!(def.rows as usize, raw.level.len());

            for row in 0..ROWS {
                #[expect(clippy::cast_possible_truncation, reason = "row < 8")]
                let trigger = table.arena.trigger(def, row as u8 + 1);
                assert_eq!(trigger.chance, convert::pct(raw.chance[row]).expect("chance"));
            }
            for (row, value) in raw.value.iter().enumerate() {
                #[expect(clippy::cast_possible_truncation, reason = "row < 16")]
                let (got, _) = table.arena.payload(def, row as u8 + 1);
                assert_eq!(got, value.map_or(0, |v| convert::centi(v).expect("value")));
            }
        }
    }

    /// The big difference between how skill and leader skills are handled
    ///  is that leader skills can be pairs but skills can be any of groups,
    /// year-groups (which would be a trio), or a sub-unit (which are also only trios in SIF)
    #[test]
    fn check_targets_resolve_to_whole_groups() {
        let (table, roster, _) = built();
        assert_eq!(table.targets.len(), 14);
        for i in 0..table.targets.len() {
            let id = TargetId::from_index(i);
            let name = table.targets.name(id).expect("in range");
            let members = match name {
                "μ's" | "Aqours" => 9,
                _ => 3,
            };
            assert_eq!(table.targets.set(id).expect("in range").0.count_ones(), members, "{name}");
        }

        let names: Vec<&str> = (0..table.targets.len())
            .map(|i| table.targets.name(TargetId::from_index(i)).expect("in range"))
            .collect();
        assert!(!names.contains(&"All"), "no card asks for the All key");

        // The stored mask is the whole group, including its own own bit
        let nico = roster.id("Yazawa Nico").expect("in roster");
        let third = convert::character_set("third-year μ's", &map(), &roster).expect("resolve");
        assert!(third.contains(nico));
    }

    #[test]
    fn check_every_target_round_trips_through_the_table() {
        let (table, _, ids) = built();
        for (card, (card_id, id)) in cards().iter().zip(ids) {
            assert_eq!(card.card_id, card_id);
            let Some(id) = id else { continue };
            let target = table.arena.def(id).expect("push handed this id out").target;
            assert_eq!(target.and_then(|t| table.targets.name(t)), card.skill.target());
        }
    }

    #[test]
    fn check_an_unmapped_target_is_an_error() {
        let mut cards = cards();
        let roster = Roster::build(cards.iter().map(|c| c.character.as_str()));
        let c = cards.iter_mut().find(|c| c.skill.target().is_some()).expect("a targeted skill");
        c.skill.target = Some("A-RISE".into());
        let err = Table::default()
            .insert(c.card_id, &c.skill, &map(), &roster)
            .expect_err("not in year_group_target.json");
        assert_matches!(err, LoadError { field: "target", kind: ErrorKind::Unknown(s), .. }
            if &*s == "A-RISE");
    }

    /// Leaving slot 0 unused is what keeps a skill-less card free: see
    /// [`SkillId`]. If this ever fails, `Option<SkillId>` grew a second field
    #[test]
    fn check_optional_ids_cost_nothing() {
        assert_eq!(size_of::<Option<SkillId>>(), size_of::<SkillId>());
        assert_eq!(size_of::<Option<TargetId>>(), 1);
    }
}
