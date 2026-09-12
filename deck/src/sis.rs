//! This is nice and simple since it's basically the exact same as
//! accessory.rs, but even simpler, since there's no skill levels for SIS.
//! Just add the ids of the SIS that you have and BAM nothing else needed.

use serde::{Deserialize, Serialize};

use simulator::sis::{Sis, SisDb};

use crate::manager::{Entry, EntryId, Manager, Repairs, Slots};

pub const MAX_SIS: usize = 300;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    UnknownSis,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct SisEntry {
    sis_id: u16,
}

impl SisEntry {
    pub(crate) const fn new(sis_id: u16) -> Self {
        Self { sis_id }
    }

    #[must_use]
    pub const fn sis_id(self) -> u16 {
        self.sis_id
    }
}

impl Entry for SisEntry {
    type Db = SisDb;
    type Problem = Problem;

    const CAPACITY: usize = MAX_SIS;
    const FULL: Problem = Problem::Full;

    fn vet(self, db: &SisDb) -> Result<(Self, Repairs), Vec<Problem>> {
        if db.index(self.sis_id).is_none() {
            return Err(vec![Problem::UnknownSis]);
        }
        Ok((self, Repairs::default()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SisInventory {
    pub name: String,
    entries: Slots<SisEntry>,
}

impl Manager for SisInventory {
    type Entry = SisEntry;

    fn slots(&self) -> &Slots<SisEntry> {
        &self.entries
    }

    fn slots_mut(&mut self) -> &mut Slots<SisEntry> {
        &mut self.entries
    }
}

impl SisInventory {
    #[must_use]
    pub const fn new(name: String) -> Self {
        Self { name, entries: Slots::empty() }
    }

    /// # Errors
    /// An unknown id, or an already full inventory.
    pub fn add(&mut self, sis: &SisDb, sis_id: u16) -> Result<(EntryId, Repairs), Vec<Problem>> {
        self.entries.add(sis, SisEntry::new(sis_id))
    }

    #[must_use]
    pub fn sis(&self, id: EntryId, sis: &SisDb) -> Option<Sis> {
        let i = sis.index(self.get(id)?.sis_id())?.0 as usize;
        sis.sis.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_SIS, Problem, SisEntry, SisInventory};
    use crate::manager::{EntryId, Manager};
    use simulator::scalar::Attribute;
    use simulator::sis::{Sis, SisDb, SisEffect, SisMeta};
    use simulator::skill;

    /// Ids 7 and 9, so 8 is a hole to miss on, and two effects so a lookup can
    /// tell them apart.
    fn sis() -> SisDb {
        SisDb {
            sis: vec![
                Sis::new(SisEffect::SelfFlat, Attribute::Smile, 1, 200_000, None, None),
                Sis::new(SisEffect::SelfPercent, Attribute::Cool, 4, 290, None, None),
            ],
            meta: vec![
                SisMeta { sis_id: 7, name: "Smile Kiss".into() },
                SisMeta { sis_id: 9, name: "Cool Charm".into() },
            ],
            targets: skill::Targets::default(),
        }
    }

    fn box_of(name: &str) -> SisInventory {
        SisInventory::new(name.to_owned())
    }

    #[test]
    fn check_unknown_sis_id() {
        let (db, mut s) = (sis(), box_of("test"));
        assert_eq!(s.add(&db, 8), Err(vec![Problem::UnknownSis]));
        assert!(s.is_empty());
    }

    #[test]
    fn check_an_id_is_reused_lowest_hole_first() {
        let (db, mut s) = (sis(), box_of("test"));
        let ids: Vec<EntryId> = [7, 7, 7].into_iter().map(|id| s.add(&db, id).unwrap().0).collect();
        assert_eq!(s.remove(ids[2]).map(SisEntry::sis_id), Some(7));
        assert_eq!(s.remove(ids[0]).map(SisEntry::sis_id), Some(7));
        assert_eq!(s.len(), 1);

        assert_eq!(s.add(&db, 9).unwrap().0, ids[0]);
        assert_eq!(s.add(&db, 9).unwrap().0, ids[2]);
        assert_eq!(s.get(ids[0]).map(SisEntry::sis_id), Some(9));
        assert_eq!(s.entries().count(), 3);
    }

    #[test]
    fn check_a_full_inventory_says_so() {
        let (db, mut s) = (sis(), box_of("test"));
        for _ in 0..MAX_SIS {
            s.add(&db, 7).unwrap();
        }
        assert!(s.is_full());
        assert_eq!(s.add(&db, 7), Err(vec![Problem::Full]));
        assert_eq!(s.len(), MAX_SIS);
    }

    #[test]
    fn check_a_removal_round_trip() {
        let (db, mut s) = (sis(), box_of("mine"));
        let first = s.add(&db, 7).unwrap().0;
        let second = s.add(&db, 9).unwrap().0;
        s.remove(first);

        let json = s.to_json().unwrap();
        let text = String::from_utf8(json.clone()).unwrap();
        assert!(!text.contains("null"));
        assert!(!text.contains("sis_id"), "id");

        let back = SisInventory::parse(&json, &db).unwrap();
        assert_eq!(back.value.name, "mine");
        assert_eq!(back.value.len(), 1);
        assert_eq!(back.value.get(first).map(SisEntry::sis_id), Some(9));
        assert_eq!(back.value.get(second), None);
        assert!(back.rejected.is_empty() && back.repaired.is_empty());
    }
}
