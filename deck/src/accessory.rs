//! Every accessory a user owns and its skill level.

use serde::{Deserialize, Serialize};

use simulator::accessory::{AccessoryDb, AccessoryIdx, StatRows};
use simulator::scalar::Stats;

use crate::manager::{Entry, EntryId, Manager, Repairs, Slots};
use crate::skill_level;

pub const MAX_ACCESSORIES: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    UnknownAccessory,
    SkillLevel,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AccessoryEntry {
    acc_id: u16,
    #[serde(default = "skill_level::default")]
    skill_level: u8,
    /// Row number of `accessories.rkyv` for this acc so the lookup can jump straight to it
    /// instead of needing to search it based on the id.
    #[serde(skip)]
    at: u16,
}

impl AccessoryEntry {
    pub(crate) const fn new(acc_id: u16, skill_level: u8) -> Self {
        Self { acc_id, skill_level, at: 0 }
    }

    #[must_use]
    pub const fn acc_id(self) -> u16 {
        self.acc_id
    }

    #[must_use]
    pub const fn skill_level(self) -> u8 {
        self.skill_level
    }

    #[must_use]
    pub fn index(self, accessories: &AccessoryDb) -> Option<AccessoryIdx> {
        accessories.index_with_hint(self.acc_id, AccessoryIdx(self.at))
    }
}

impl Entry for AccessoryEntry {
    type Db = AccessoryDb;
    type Problem = Problem;

    const CAPACITY: usize = MAX_ACCESSORIES;
    const FULL: Problem = Problem::Full;

    fn vet(mut self, db: &AccessoryDb) -> Result<(Self, Repairs), Vec<Problem>> {
        let Some(at) = db.index(self.acc_id) else {
            return Err(vec![Problem::UnknownAccessory]);
        };
        self.at = at.0;
        if !skill_level::in_range(self.skill_level) {
            return Err(vec![Problem::SkillLevel]);
        }
        Ok((self, Repairs::default()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Accessories {
    pub name: String,
    entries: Slots<AccessoryEntry>,
}

impl Manager for Accessories {
    type Entry = AccessoryEntry;

    fn slots(&self) -> &Slots<AccessoryEntry> {
        &self.entries
    }

    fn slots_mut(&mut self) -> &mut Slots<AccessoryEntry> {
        &mut self.entries
    }
}

impl Accessories {
    #[must_use]
    pub const fn new(name: String) -> Self {
        Self { name, entries: Slots::empty() }
    }

    /// # Errors
    /// A rule break or an already full inventory.
    pub fn add(
        &mut self,
        accessories: &AccessoryDb,
        acc_id: u16,
        skill_level: u8,
    ) -> Result<(EntryId, Repairs), Vec<Problem>> {
        self.entries.add(accessories, AccessoryEntry::new(acc_id, skill_level))
    }

    #[must_use]
    pub fn stats(&self, id: EntryId, accessories: &AccessoryDb) -> Option<Stats> {
        let entry = self.get(id)?;
        let i = usize::from(entry.index(accessories)?.0);
        let accessory = accessories.accessories.get(i)?;
        Some(accessories.stats.get(accessory.stats, entry.skill_level()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Accessories, AccessoryEntry, MAX_ACCESSORIES, Problem};
    use crate::manager::{EntryId, Manager};
    use crate::skill_level;
    use simulator::accessory::{
        Accessory, AccessoryArena, AccessoryDb, AccessoryMeta, STAT_ROWS, StatArena,
    };
    use simulator::scalar::{CharacterId, Stats};
    use simulator::skill::{self, SkillEffect};

    pub(crate) fn accessories() -> AccessoryDb {
        let mut skills = AccessoryArena::default();
        let skill = skills.push(SkillEffect::Encore, None, 0, &[3; 8], &[(0, 0)]);

        // 100_000 `Centi` a level, so level 8 is 800_000 and 8000 points.
        let payload: [(u32, u16); 8] = core::array::from_fn(|i| {
            (u32::try_from(i).unwrap().saturating_add(1).saturating_mul(100_000), 0)
        });
        let scorer = skills.push(SkillEffect::Scorer, None, 0, &[31; 8], &payload);
        let spark = skills.push(SkillEffect::Spark, None, 5, &[11; 8], &payload);
        // 1 HP a level.
        let hp: [(u32, u16); 8] = core::array::from_fn(|i| {
            (u32::try_from(i).unwrap().saturating_add(1).saturating_mul(100), 0)
        });
        let healer = skills.push(SkillEffect::Healer, None, 0, &[30; 8], &hp);
        let mut stats = StatArena::default();

        let ladder = |base: u16| {
            let mut table = [Stats([0; 3]); STAT_ROWS as usize];
            for (i, row) in table.iter_mut().enumerate() {
                let step = base.saturating_add(u16::try_from(i).unwrap());
                *row = Stats([step, step.saturating_add(100), step.saturating_add(200)]);
            }
            table
        };
        let first = stats.push(&ladder(10));
        let second = stats.push(&ladder(50));

        AccessoryDb {
            accessories: vec![
                Accessory { skill, stats: first },
                Accessory { skill, stats: second },
                Accessory { skill: scorer, stats: second },
                Accessory { skill: spark, stats: second },
                Accessory { skill: healer, stats: first },
            ],
            // Just a random selection of accessories to use for testing skill level lookups.
            meta: vec![
                AccessoryMeta {
                    acc_id: 7,
                    card_id: None,
                    character: None,
                    name: "N Jam".into(),
                },
                AccessoryMeta {
                    acc_id: 9,
                    card_id: Some(3859),
                    character: Some(CharacterId(7)),
                    name: "UR Guren".into(),
                },
                AccessoryMeta {
                    acc_id: 11,
                    card_id: None,
                    character: None,
                    name: "UR Pin".into(),
                },
                AccessoryMeta {
                    acc_id: 13,
                    card_id: None,
                    character: None,
                    name: "UR Medal".into(),
                },
                // this one's to test healing.
                AccessoryMeta {
                    acc_id: 15,
                    card_id: None,
                    character: None,
                    name: "UR Clef".into(),
                },
            ],
            skills,
            stats,
            targets: skill::Targets::default(),
        }
    }

    fn box_of(name: &str) -> Accessories {
        Accessories::new(name.to_owned())
    }

    #[test]
    fn check_acc_entry_index() {
        let (db, mut a) = (accessories(), box_of("test"));
        let (id, _) = a.add(&db, 9, 1).unwrap();
        let entry = a.get(id).unwrap();
        let right = db.index(9);
        assert_ne!(entry.at, 0, "vet stored it");
        assert_eq!(entry.index(&db), right);
        for at in [0, u16::MAX] {
            assert_eq!(AccessoryEntry { at, ..entry }.index(&db), right, "{at}");
        }
        assert_eq!(AccessoryEntry { acc_id: 8, ..entry }.index(&db), None);
    }

    #[test]
    fn check_unknown_acc_id() {
        let (db, mut a) = (accessories(), box_of("test"));
        assert_eq!(a.add(&db, 8, 1), Err(vec![Problem::UnknownAccessory]));
        assert!(a.is_empty());
    }

    #[test]
    fn check_the_skill_level_bounds() {
        let (db, mut a) = (accessories(), box_of("test"));
        assert_eq!(a.add(&db, 7, skill_level::MAX_LEVEL + 1), Err(vec![Problem::SkillLevel]));
        assert_eq!(a.add(&db, 7, 0), Err(vec![Problem::SkillLevel]));
        assert!(a.is_empty());

        let (id, repairs) = a.add(&db, 7, skill_level::MAX_LEVEL).unwrap();
        assert!(repairs.is_empty(), "an accessory has nothing to repair");
        assert_eq!(a.get(id).map(AccessoryEntry::skill_level), Some(skill_level::MAX_LEVEL));
    }

    #[test]
    fn check_an_id_is_reused_lowest_hole_first() {
        let (db, mut a) = (accessories(), box_of("test"));
        let ids: Vec<EntryId> =
            [1, 4, 8].into_iter().map(|level| a.add(&db, 7, level).unwrap().0).collect();
        assert_eq!(a.remove(ids[2]).map(AccessoryEntry::acc_id), Some(7));
        assert_eq!(a.remove(ids[0]).map(AccessoryEntry::acc_id), Some(7));
        assert_eq!(a.len(), 1);

        assert_eq!(a.add(&db, 9, 1).unwrap().0, ids[0]);
        assert_eq!(a.add(&db, 9, 1).unwrap().0, ids[2]);
        assert_eq!(a.get(ids[0]).map(AccessoryEntry::acc_id), Some(9));
        assert_eq!(a.entries().count(), 3);
    }

    #[test]
    fn check_a_full_inventory_says_so() {
        let (db, mut a) = (accessories(), box_of("test"));
        for _ in 0..MAX_ACCESSORIES {
            a.add(&db, 7, 1).unwrap();
        }
        assert!(a.is_full());
        assert_eq!(a.add(&db, 7, 1), Err(vec![Problem::Full]));
        assert_eq!(a.len(), MAX_ACCESSORIES);
    }

    #[test]
    fn check_a_removal_round_trip() {
        let (db, mut a) = (accessories(), box_of("mine"));
        let first = a.add(&db, 7, 3).unwrap().0;
        let second = a.add(&db, 9, 8).unwrap().0;
        a.remove(first);

        let json = a.to_json().unwrap();
        assert!(!String::from_utf8(json.clone()).unwrap().contains("null"));

        let back = Accessories::parse(&json, &db).unwrap();
        assert_eq!(back.value.name, "mine");
        assert_eq!(back.value.len(), 1);
        assert_eq!(back.value.get(first).map(AccessoryEntry::acc_id), Some(9));
        assert_eq!(back.value.get(second), None);
        assert!(back.rejected.is_empty() && back.repaired.is_empty());
    }
}
