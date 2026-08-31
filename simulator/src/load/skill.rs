//! Folds the 3505 card skills into a [`SkillArena`] of 2460 distinct ones,
//! plus an unused entry at slot 0 so that a card with no skill costs nothing
//! to store. See [`SkillId`] for why that works.

use core::num::NonZeroU16;
use std::collections::HashMap;

use crate::load::convert::{self, At, ErrorKind, LoadError, Roster};
use crate::load::raw::{RawSkill, RawTargetMap};
use crate::scalar::{Centi, CharacterSet, Millis};
use crate::skill::{Arming, SkillArena, SkillDef, SkillEffect, SkillId, SkillTrigger, TargetId};

/// Base level is 1..=8 for every card in the game and the shortest skill table
/// is 8 rows (the URs), so arming is always exactly this many rows.
const ARMING_ROWS: usize = 8;

#[derive(PartialEq, Eq, Hash)]
struct Key {
    effect: SkillEffect,
    trigger: SkillTrigger,
    target: Option<TargetId>,
    arming: [Arming; ARMING_ROWS],
    values: Vec<Centi>,
    durations: Vec<Millis>,
}

/// Encodes the 14 distinct skill target strings into bitmasks, out of
/// `year_group_target.json`.
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
    pub fn set(&self, id: TargetId) -> Option<CharacterSet> {
        self.sets.get(id.index()).copied()
    }

    #[must_use]
    pub fn name(&self, id: TargetId) -> Option<&str> {
        self.names.get(id.index()).map(Box::as_ref)
    }

    fn id(
        &mut self,
        name: &str,
        map: &RawTargetMap,
        roster: &Roster,
    ) -> Result<TargetId, ErrorKind> {
        if let Some(i) = self.names.iter().position(|n| **n == *name) {
            return Ok(TargetId::from_index(i));
        }
        let set = convert::character_set(name, map, roster)?;
        self.names.push(name.into());
        self.sets.push(set);
        Ok(TargetId::from_index(self.names.len() - 1))
    }
}

pub struct Table {
    pub arena: SkillArena,
    pub targets: Targets,
    seen: HashMap<Key, SkillId>,
}

impl Default for Table {
    fn default() -> Self {
        let mut arena = SkillArena::default();
        // Slot 0 is never handed out. Its contents are never read; it is here
        // only so that no real skill has id 0.
        arena.defs.push(SkillDef {
            effect: SkillEffect::Scorer,
            trigger: SkillTrigger::Combo,
            target: None,
            rows: 0,
            payload: 0,
            arming: 0,
        });
        Self { arena, targets: Targets::default(), seen: HashMap::new() }
    }
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
        let trigger = convert::trigger(&raw.activation).at(card_id, "activation")?;
        let target = raw
            .target()
            .map(|t| self.targets.id(t, map, roster))
            .transpose()
            .at(card_id, "target")?;

        let mut arming = [Arming { threshold: 0, chance: 0 }; ARMING_ROWS];
        for (row, out) in arming.iter_mut().enumerate() {
            // Six skills have no threshold. Star Notes rolls on every star note,
            // and the five Year Group ones arm on a set cover a counter cannot
            // express, so nothing sensible goes here.
            out.threshold = match (raw.threshold[row], trigger) {
                (Some(t), _) => convert::threshold(t).at(card_id, "threshold")?,
                (None, SkillTrigger::YearGroup) => 0,
                (None, _) => 1,
            };
            out.chance = convert::pct(raw.chance[row]).at(card_id, "chance")?;
        }

        // Perfect Lock, Total Trick, Sync and Encore carry no value.
        // Stores 0 rather than an `Option`
        let mut values = Vec::with_capacity(raw.value.len());
        let mut durations = Vec::with_capacity(raw.duration.len());
        for row in 0..raw.value.len() {
            values.push(match raw.value[row] {
                Some(v) => convert::centi(v).at(card_id, "value")?,
                None => 0,
            });
            durations.push(match raw.duration[row] {
                Some(d) => convert::millis(d).at(card_id, "duration")?,
                None => 0,
            });
        }

        let key = Key { effect, trigger, target, arming, values, durations };
        if let Some(&id) = self.seen.get(&key) {
            return Ok(Some(id));
        }
        Ok(Some(self.push(key)))
    }

    fn push(&mut self, key: Key) -> SkillId {
        let def = SkillDef {
            effect: key.effect,
            trigger: key.trigger,
            target: key.target,
            rows: u8::try_from(key.values.len()).expect("16 rows at most"),
            payload: u32::try_from(self.arena.values.len()).expect("payload arena fits u32"),
            arming: u32::try_from(self.arena.arming.len()).expect("arming arena fits u32"),
        };
        self.arena.values.extend_from_slice(&key.values);
        self.arena.durations.extend_from_slice(&key.durations);
        self.arena.arming.extend_from_slice(&key.arming);

        let slot = u16::try_from(self.arena.defs.len()).expect("skill table fits u16");
        let id = SkillId(NonZeroU16::new(slot).expect("slot 0 is the dead entry"));
        self.arena.defs.push(def);
        self.seen.insert(key, id);
        id
    }
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

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
        assert_eq!(table.arena.defs.len(), 2461);
        assert_eq!(table.arena.arming.len(), 2460 * ARMING_ROWS);
        assert_eq!(table.arena.values.len(), table.arena.durations.len());
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
            let def = &table.arena.defs[id.0.get() as usize];
            let raw = &card.skill;
            assert_eq!(def.rows as usize, raw.level.len());

            for row in 0..ARMING_ROWS {
                #[expect(clippy::cast_possible_truncation, reason = "row < 8")]
                let armed = table.arena.arming(def, row as u8 + 1);
                assert_eq!(armed.chance, convert::pct(raw.chance[row]).expect("chance"));
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
            let target = table.arena.defs[id.0.get() as usize].target;
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
