use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::{Kind, portable};
use crate::scalar::{Attribute, Milli};
use crate::skill::{self, TargetId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct SisIdx(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum SisEffect {
    SelfFlat,
    SelfPercent,
    AllPercent,
    Charm,
    Heal,
    Trick,
}

/// What team compilation reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Sis {
    /// Thousandths. Aura/Veil SIS have one extra decimal place in their value
    /// field that anything in the cards/accessories jsons.
    pub value: Milli,
    pub restriction: Option<TargetId>,
    pub group: Option<TargetId>,
    pub effect: SisEffect,
    pub attribute: Attribute,
    pub slots: u8,
    /// So `portable!` implements `NoUndef`, whose contract is that the type has no
    /// padding at all. So tjat ,eams the three bytes a `u32` and five `u8`s round up to
    /// need to be declared here because the compiler doesn't like it otherwise.
    _pad: [u8; 3],
}

impl Sis {
    #[must_use]
    pub const fn new(
        effect: SisEffect,
        attribute: Attribute,
        slots: u8,
        value: Milli,
        restriction: Option<TargetId>,
        group: Option<TargetId>,
    ) -> Self {
        Self {
            value,
            restriction,
            group,
            effect,
            attribute,
            slots,
            _pad: [0; 3],
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct SisMeta {
    pub sis_id: u16,
    pub name: Box<str>,
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct SisDb {
    pub sis: Vec<Sis>,
    pub meta: Vec<SisMeta>,
    pub targets: skill::Targets,
}

impl Kind for SisDb {
    const MAGIC: [u8; 8] = *b"LLSIFSDB";
    const VERSION: u32 = 1;
}

impl SisDb {
    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "len checked at build")]
    pub fn index(&self, sis_id: u16) -> Option<SisIdx> {
        let i = self.meta.binary_search_by_key(&sis_id, |m| m.sis_id).ok()?;
        Some(SisIdx(i as u16))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.sis.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sis.is_empty()
    }
}

portable!(SisIdx, SisEffect, Sis);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{Mapped, write};

    const KISS: Sis = Sis::new(SisEffect::SelfFlat, Attribute::Smile, 1, 200_000, None, None);

    fn built() -> SisDb {
        SisDb {
            sis: vec![KISS],
            meta: vec![SisMeta { sis_id: 1, name: "Smile Kiss".into() }],
            targets: skill::Targets::default(),
        }
    }

    #[test]
    fn check_the_record_has_no_padding_to_leave_uninitialized() {
        assert_eq!(size_of::<Sis>(), 12);
        assert_eq!(size_of::<rkyv::Archived<Sis>>(), 12);
        assert_eq!(size_of::<Option<TargetId>>(), 1);
    }

    #[test]
    fn check_the_sis_db_survives_the_archive() {
        let db = built();
        assert_eq!(db.index(1), Some(SisIdx(0)));
        assert_eq!(db.index(2), None);

        let path = std::env::temp_dir().join(format!("llsif-sis-{}.rkyv", std::process::id()));
        write(&path, &db).expect("write");
        // SAFETY: `write` just produced this file and nothing else touches it.
        let mapped: Mapped<SisDb> = unsafe { Mapped::open(&path) }.expect("open");
        assert!(mapped.root() == &db, "the archive is not the database it was built from");

        drop(mapped);
        std::fs::remove_file(&path).expect("cleanup");
    }
}
