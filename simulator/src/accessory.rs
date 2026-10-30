use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::{Kind, portable};
use crate::scalar::{Centi, CharacterId, Ms, Pct, Stats};
use crate::skill::{self, MAX_BASE_LEVEL, SkillEffect, TRIGGER_ROWS, TargetId};

pub const STAT_ROWS: u8 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct AccessoryIdx(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct AccessorySkillId(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct StatsId(pub u16);

/// What team compilation will read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Accessory {
    pub skill: AccessorySkillId,
    pub stats: StatsId,
}

/// The info needed to know if a `Card` can equip an `Accessory`.
#[derive(Clone, PartialEq, Eq, Debug, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct AccessoryMeta {
    pub acc_id: u16,
    pub card_id: Option<u16>,
    pub character: Option<CharacterId>,
    pub name: Box<str>,
}

impl AccessoryMeta {
    /// Did you know that accessories tied to a specific `card_id` can
    /// actually be equipped on any `Card` of that `Character` if the
    /// `Accessory` is max level? Because I sure as hell didn't until
    /// a few hours ago!
    #[must_use]
    pub fn equips(&self, level: u8, card_id: u16, character: CharacterId) -> bool {
        let Some(restricted_to) = self.card_id else {
            return true;
        };
        restricted_to == card_id || (level >= MAX_BASE_LEVEL && self.character == Some(character))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct AccessorySkillDef {
    pub effect: SkillEffect,
    pub target: Option<TargetId>,
    pub threshold: u8,
    pub rows: u8,
    payload: u32,
    trigger: u32,
}

#[derive(Default, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct AccessoryArena {
    defs: Vec<AccessorySkillDef>,
    values: Vec<Centi>,
    durations: Vec<Ms>,
    chances: Vec<Pct>,
}

impl AccessoryArena {
    /// # Panics
    /// Empty, past 255 rows, past 65535 skills.
    #[expect(clippy::expect_used, reason = "capacity bounds")]
    pub fn push(
        &mut self,
        effect: SkillEffect,
        target: Option<TargetId>,
        threshold: u8,
        chances: &[Pct; TRIGGER_ROWS as usize],
        payload: &[(Centi, Ms)],
    ) -> AccessorySkillId {
        assert!(!payload.is_empty(), "a skill with no payload rows would read the next skill's");
        let def = AccessorySkillDef {
            effect,
            target,
            threshold,
            rows: u8::try_from(payload.len()).expect("16 payload rows at most"),
            payload: u32::try_from(self.values.len()).expect("payload arena fits u32"),
            trigger: u32::try_from(self.chances.len()).expect("chance arena fits u32"),
        };
        for &(value, duration) in payload {
            self.values.push(value);
            self.durations.push(duration);
        }
        self.chances.extend_from_slice(chances);

        let id = u16::try_from(self.defs.len()).expect("skill table fits u16");
        self.defs.push(def);
        AccessorySkillId(id)
    }
}

pub trait AccessoryRows {
    fn defs(&self) -> &[AccessorySkillDef];
    fn chances(&self) -> &[Pct];
    fn payload_row(&self, row: usize) -> (Centi, Ms);

    #[must_use]
    fn def(&self, id: AccessorySkillId) -> Option<&AccessorySkillDef> {
        self.defs().get(id.0 as usize)
    }

    #[inline]
    #[must_use]
    #[expect(clippy::arithmetic_side_effects, reason = "u32 + u8; see `archive`'s usize assert")]
    fn payload(&self, def: &AccessorySkillDef, level: u8) -> (Centi, Ms) {
        self.payload_row(def.payload as usize + level.min(def.rows).saturating_sub(1) as usize)
    }

    #[inline]
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated TRIGGER_ROWS at `def.trigger`")]
    #[expect(clippy::arithmetic_side_effects, reason = "u32 + u8; see `archive`'s usize assert")]
    fn chance(&self, def: &AccessorySkillDef, level: u8) -> Pct {
        self.chances()[def.trigger as usize + level.min(TRIGGER_ROWS).saturating_sub(1) as usize]
    }
}

impl AccessoryRows for AccessoryArena {
    fn defs(&self) -> &[AccessorySkillDef] {
        &self.defs
    }

    fn chances(&self) -> &[Pct] {
        &self.chances
    }

    #[inline]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated `def.rows` at `def.payload`")]
    fn payload_row(&self, row: usize) -> (Centi, Ms) {
        (self.values[row], self.durations[row])
    }
}

impl AccessoryRows for ArchivedAccessoryArena {
    fn defs(&self) -> &[AccessorySkillDef] {
        &self.defs
    }

    fn chances(&self) -> &[Pct] {
        &self.chances
    }

    #[inline]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated `def.rows` at `def.payload`")]
    fn payload_row(&self, row: usize) -> (Centi, Ms) {
        (self.values[row].to_native(), self.durations[row].to_native())
    }
}

#[derive(Default, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct StatArena {
    rows: Vec<Stats>,
}

impl StatArena {
    /// # Panics
    /// Past `u16::MAX` rows. The current file is nowhere close.
    #[expect(clippy::expect_used, reason = "capacity bound, documented above")]
    pub fn push(&mut self, table: &[Stats; STAT_ROWS as usize]) -> StatsId {
        let id = u16::try_from(self.rows.len()).expect("stat arena fits u16");
        self.rows.extend_from_slice(table);
        StatsId(id)
    }
}

pub trait StatRows {
    fn rows(&self) -> &[Stats];

    #[inline]
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated STAT_ROWS at this offset")]
    #[expect(clippy::arithmetic_side_effects, reason = "u16 + u8; see `archive`'s usize assert")]
    fn get(&self, id: StatsId, level: u8) -> Stats {
        self.rows()[id.0 as usize + level.min(STAT_ROWS).saturating_sub(1) as usize]
    }
}

impl StatRows for StatArena {
    fn rows(&self) -> &[Stats] {
        &self.rows
    }
}

impl StatRows for ArchivedStatArena {
    fn rows(&self) -> &[Stats] {
        &self.rows
    }
}

// hot accessory fields.
#[derive(Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct AccessoryDb {
    pub accessories: Vec<Accessory>,
    pub meta: Vec<AccessoryMeta>,
    pub skills: AccessoryArena,
    pub stats: StatArena,
    pub targets: skill::Targets,
}

impl Kind for AccessoryDb {
    const MAGIC: [u8; 8] = *b"LLSIFADB";
    const VERSION: u32 = 3;
}

impl AccessoryDb {
    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "len checked at build")]
    pub fn index(&self, acc_id: u16) -> Option<AccessoryIdx> {
        let i = self.meta.binary_search_by_key(&acc_id, |m| m.acc_id).ok()?;
        Some(AccessoryIdx(i as u16))
    }

    /// Tries to jump to directly to the row number if available (and it always should be)
    /// and if it's not it defaults to doing a binary search on the id to find the right
    /// location.
    #[must_use]
    pub fn index_with_hint(&self, acc_id: u16, hint: AccessoryIdx) -> Option<AccessoryIdx> {
        match self.meta.get(usize::from(hint.0)) {
            Some(meta) if meta.acc_id == acc_id => Some(hint),
            _ => self.index(acc_id),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.accessories.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.accessories.is_empty()
    }
}

portable!(AccessoryIdx, AccessorySkillId, StatsId, Accessory, AccessorySkillDef);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{Mapped, write};

    /// Acc 217 `UR Guren Type Special`, row 1: threshold 5, chance 11.
    fn spark(arena: &mut AccessoryArena) -> AccessorySkillId {
        const PAYLOAD: [(Centi, Ms); 8] = [
            (1000, 500),
            (2000, 1000),
            (3000, 1500),
            (4000, 2000),
            (5000, 2500),
            (6000, 3000),
            (7000, 3500),
            (8000, 4000),
        ];
        arena.push(SkillEffect::Spark, None, 5, &[11, 14, 17, 20, 24, 28, 31, 34], &PAYLOAD)
    }

    #[test]
    fn check_the_definition_tier_stays_small() {
        assert_eq!(size_of::<Accessory>(), 4);
        assert_eq!(size_of::<rkyv::Archived<Accessory>>(), 4);
        assert_eq!(size_of::<AccessorySkillDef>(), 12);
        assert_eq!(size_of::<rkyv::Archived<AccessorySkillDef>>(), 12);
    }

    #[test]
    fn check_character_equip() {
        let (honoka, eli) = (CharacterId(3), CharacterId(4));
        let restricted = AccessoryMeta {
            acc_id: 1,
            card_id: Some(165),
            character: Some(honoka),
            name: "UR Pink Ice Cream Diary".into(),
        };
        let free = AccessoryMeta {
            acc_id: 19,
            card_id: None,
            character: None,
            name: "UR Amethyst".into(),
        };

        for level in 1..=MAX_BASE_LEVEL {
            assert!(restricted.equips(level, 165, honoka), "its own card, level {level}");
            assert!(free.equips(level, 165, honoka), "unrestricted, level {level}");
            assert!(free.equips(level, 900, eli), "unrestricted, level {level}");
        }
        for level in 1..MAX_BASE_LEVEL {
            assert!(!restricted.equips(level, 900, honoka), "her other card, level {level}");
        }
        assert!(restricted.equips(MAX_BASE_LEVEL, 900, honoka), "her other card, maxed");
        assert!(!restricted.equips(MAX_BASE_LEVEL, 900, eli), "never off-character");
    }

    #[test]
    fn check_table_saturates() {
        // Checking that a +16 Amplify on top of level 8 must land in and not past the acc's bounds.
        let mut arena = AccessoryArena::default();
        let encore = arena.push(SkillEffect::Encore, None, 0, &[3; 8], &[(0, 0)]);
        let spark = spark(&mut arena);

        let trimmed = arena.def(encore).expect("just pushed");
        assert_eq!(arena.payload(trimmed, 24), (0, 0));
        assert_eq!(arena.payload(trimmed, 1), (0, 0));
        assert_eq!(arena.chance(trimmed, 1), 3);
        // Amplify does not reach the trigger, but a level of 0 must not wrap.
        assert_eq!(arena.chance(trimmed, 0), 3);

        let full = arena.def(spark).expect("just pushed");
        assert_eq!(arena.payload(full, 1), (1000, 500));
        assert_eq!(arena.payload(full, 8), (8000, 4000));
        assert_eq!(arena.payload(full, 24), (8000, 4000));
        assert_eq!(arena.chance(full, 8), 34);
        assert_eq!(arena.def(AccessorySkillId(2)), None);
    }

    #[test]
    fn check_the_accessory_db_survives_the_archive() {
        let mut skills = AccessoryArena::default();
        let skill = spark(&mut skills);
        let mut stats = StatArena::default();
        let stats_id = stats.push(&[Stats([1, 2, 3]); STAT_ROWS as usize]);
        let db = AccessoryDb {
            accessories: vec![Accessory { skill, stats: stats_id }],
            meta: vec![AccessoryMeta {
                acc_id: 217,
                card_id: Some(3859),
                character: Some(CharacterId(7)),
                name: "UR Guren Type Special".into(),
            }],
            skills,
            stats,
            targets: skill::Targets::default(),
        };
        assert_eq!(db.index(217), Some(AccessoryIdx(0)));
        assert_eq!(db.index(216), None);

        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("llsif-accessory-{pid}.rkyv"));
        write(&path, &db).expect("write");
        // SAFETY: `write` just produced this file and nothing else touches it.
        let mapped: Mapped<AccessoryDb> = unsafe { Mapped::open(&path) }.expect("open");
        // `AccessoryDb` has no `Debug`, so this cannot be `assert_eq!`.
        assert!(mapped.root() == &db, "the archive is not the database it was built from");

        let root = mapped.root();
        let def = root.skills.def(skill).expect("in range");
        assert_eq!(root.skills.payload(def, 8), (8000, 4000));
        assert_eq!(root.skills.chance(def, 8), 34);
        assert_eq!(root.stats.get(stats_id, 8), Stats([1, 2, 3]));

        drop(mapped);
        std::fs::remove_file(&path).expect("cleanup");
    }
}
