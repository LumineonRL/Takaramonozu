//! Takes the 3505 unique cards skills and places them into a [`SkillArena`]

use std::collections::HashMap;
use core::num::{NonZeroU8, NonZeroU16};

use crate::load::convert::{self, At, LoadError};
use crate::load::raw::RawSkill;
use crate::scalar::{Centi, Millis};
use crate::skill::{
    Arming, SkillArena, SkillDef, SkillEffect, SkillId, SkillTrigger, TargetId,
};

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

pub struct Interner {
    pub arena: SkillArena,
    /// The 14 distinct `target` strings.
    pub targets: Vec<Box<str>>,
    seen: HashMap<Key, SkillId>,
}

impl Default for Interner {
    fn default() -> Self {
        let mut arena = SkillArena::default();
        // Slot 0 is no skill
        arena.defs.push(SkillDef {
            effect: SkillEffect::Scorer,
            trigger: SkillTrigger::Combo,
            target: None,
            rows: 0,
            payload: 0,
            arming: 0,
        });
        Self { arena, targets: Vec::new(), seen: HashMap::new() }
    }
}

impl Interner {
    /// # Errors
    /// Any field that does not convert
    /// # Panics
    /// Past 65535 distinct skills or 255 targets, neither of which the frozen
    /// data comes close to.
    pub fn skill(&mut self, card_id: u16, raw: &RawSkill) -> Result<Option<SkillId>, LoadError> {
        if !raw.is_present() {
            return Ok(None);
        }
        let effect = convert::effect(&raw.effect).at(card_id, "type")?;
        let trigger = convert::trigger(&raw.activation).at(card_id, "activation")?;
        let target = raw.target().map(|t| self.target(t));

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

    /// 14 strings, so a scan beats a second map.
    fn target(&mut self, name: &str) -> TargetId {
        let i = self.targets.iter().position(|t| **t == *name).unwrap_or_else(|| {
            self.targets.push(name.into());
            self.targets.len() - 1
        });
        let slot = u8::try_from(i + 1).expect("target table fits u8");
        TargetId(NonZeroU8::new(slot).expect("slots are 1 based"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::raw::{self, RawCard};

    fn cards() -> Vec<RawCard> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/cards.json");
        raw::parse(&std::fs::read(path).expect("cards.json")).expect("schema")
    }

    fn interned() -> (Interner, Vec<(u16, Option<SkillId>)>) {
        let mut interner = Interner::default();
        let ids = cards()
            .iter()
            .map(|c| (c.card_id, interner.skill(c.card_id, &c.skill).expect("convert")))
            .collect();
        (interner, ids)
    }

    #[test]
    fn check_corpus_folds_to_the_expected_tables() {
        let (interner, ids) = interned();
        assert_eq!(ids.iter().filter(|(_, id)| id.is_none()).count(), 321);
        // 2460 distinct skills plus the dead slot 0.
        assert_eq!(interner.arena.defs.len(), 2461);
        assert_eq!(interner.arena.arming.len(), 2460 * ARMING_ROWS);
        assert_eq!(interner.arena.values.len(), interner.arena.durations.len());
        assert_eq!(interner.targets.len(), 14);
    }

    /// Every card must read back out of the arena what its own rows said, which
    /// is what dedupe is allowed to preserve and nothing else.
    #[test]
    fn check_every_card_round_trips_through_the_arena() {
        let (interner, ids) = interned();
        for (card, (card_id, id)) in cards().iter().zip(ids) {
            assert_eq!(card.card_id, card_id);
            let Some(id) = id else { continue };
            let def = &interner.arena.defs[id.0.get() as usize];
            let raw = &card.skill;
            assert_eq!(def.rows as usize, raw.level.len());

            for row in 0..ARMING_ROWS {
                #[expect(clippy::cast_possible_truncation, reason = "row < 8")]
                let armed = interner.arena.arming(def, row as u8 + 1);
                assert_eq!(armed.chance, convert::pct(raw.chance[row]).expect("chance"));
            }
            for (row, value) in raw.value.iter().enumerate() {
                #[expect(clippy::cast_possible_truncation, reason = "row < 16")]
                let (got, _) = interner.arena.payload(def, row as u8 + 1);
                assert_eq!(got, value.map_or(0, |v| convert::centi(v).expect("value")));
            }
        }
    }

    /// The dead slot is what keeps `Option<SkillId>` the same size as `SkillId`.
    #[test]
    fn check_skill_id_niche_is_free() {
        assert_eq!(size_of::<Option<SkillId>>(), size_of::<SkillId>());
    }
}
