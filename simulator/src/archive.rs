#![expect(clippy::little_endian_bytes, reason = "the header is explicitly LE, see `header`")]

//! Framing and mmap access for the rkyv archives.
//!
//! There's some custom logic here that adds a header before the body rkyv wrote.
//! The header serves as a guard against files left over from a build whose
//! layout has changed.

// Huge thanks to The Rustonomicon's section on PhantomData for this module.
// I never would have pieced this + rkyv's documentation together otherwise.
// https://doc.rust-lang.org/nomicon/phantom-data.html

use core::marker::PhantomData;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use memmap2::Mmap;
use rkyv::api::high::HighSerializer;
use rkyv::rancor; // only using rancor::Error but don't want to conflict io::Error.
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::{Archive, Portable, Serialize};

/// Also the alignment of the body, which is what `AlignedVec` gives.
const HEADER: usize = 16;

/// One "kind" of rkyv format. For example, Cards and songs get their own file, their own magic and
/// their own version, since their layouts are independent.
pub trait Kind {
    const MAGIC: [u8; 8];
    /// TODO: Kind of brittle but this reads a manually-bumped version number.
    /// Used for `card.json`, `accessory.json`, `sis.json`, `songs.json`.
    /// Each one has an corresponding database and the VERSION would be
    /// incremented when (if) the data itself changes.
    const VERSION: u32;
}

/// The conversion to little endian (big endian also would've worked) was done to keep the header size the
/// same regardless of body size.
fn header<T>(body: u32) -> [u8; HEADER]
where
    T: Kind,
{
    let mut h = [0; HEADER];
    h[..8].copy_from_slice(&T::MAGIC);
    h[8..12].copy_from_slice(&T::VERSION.to_le_bytes());
    h[12..].copy_from_slice(&body.to_le_bytes());
    h
}

/// Serializes `value` into `path` after the `header`.
///
/// # Errors
/// Can't write file for whatever reason.
/// Archive doesn't fit u32.
pub fn write<T>(path: &Path, value: &T) -> io::Result<()>
where
    T: Kind + for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, rancor::Error>>,
{
    let body = rkyv::to_bytes::<rancor::Error>(value).map_err(io::Error::other)?;
    let len = u32::try_from(body.len()).map_err(io::Error::other)?;
    let mut file = File::create(path)?;
    file.write_all(&header::<T>(len))?;
    file.write_all(&body)
}

pub struct Mapped<T> {
    map: Mmap,
    kind: PhantomData<fn() -> T>,
}

impl<T: Archive + Kind> Mapped<T>
where
    T::Archived: Portable,
{
    const ALIGNED: () =
        assert!(align_of::<T::Archived>() <= HEADER, "the body is only aligned to HEADER");

    /// # Errors
    /// Can't open file for whatever reason
    /// Header is wrong / denied
    ///
    /// # Safety
    /// The file must not be modified while it's mapped.
    /// The file must be an archive produced by `write`.
    pub unsafe fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        // SAFETY: the caller guarantees the file is not modified while mapped.
        let map = unsafe { Mmap::map(&file) }?;
        let ok = map.split_at_checked(HEADER).is_some_and(|(head, body)| {
            u32::try_from(body.len()).is_ok_and(|len| head == header::<T>(len))
        });
        if !ok {
            let why = "not an archive of this kind and version";
            return Err(io::Error::new(io::ErrorKind::InvalidData, why));
        }
        Ok(Self { map, kind: PhantomData })
    }

    #[must_use]
    pub fn root(&self) -> &T::Archived {
        let () = Self::ALIGNED;
        #[expect(clippy::indexing_slicing, reason = "`open` rejects a map shorter than HEADER")]
        let body = &self.map[HEADER..];
        // SAFETY: HEADER prevents stale layout.
        // a corrupt body is UB.
        unsafe { rkyv::access_unchecked(body) }
    }
}

/// Basically says "this type's bytes in the archive are exactly its bytes in memory".
/// Note that types must be `Copy` or else boom.
macro_rules! portable {
    ($($t:ty),+ $(,)?) => {$(
        // SAFETY: repr(C) or repr(transparent) over integers and repr(u8)
        // enums, with no padding and no pointer.
        unsafe impl rkyv::Portable for $t {}
        // SAFETY: same as above
        unsafe impl rkyv::traits::NoUndef for $t {}

        impl rkyv::Archive for $t {
            type Archived = Self;
            type Resolver = ();

            fn resolve(&self, (): (), out: rkyv::Place<Self>) {
                out.write(*self);
            }
        }

        impl<S: rkyv::rancor::Fallible + ?Sized> rkyv::Serialize<S> for $t {
            fn serialize(&self, _: &mut S) -> Result<(), S::Error> {
                Ok(())
            }
        }

        impl<D: rkyv::rancor::Fallible + ?Sized> rkyv::Deserialize<$t, D> for $t {
            fn deserialize(&self, _: &mut D) -> Result<Self, D::Error> {
                Ok(*self)
            }
        }
    )+};
}

const _: () = assert!(cfg!(target_endian = "little"), "the archive is host-endian");
const _: () = assert!(size_of::<usize>() >= 8, "arena offsets assume a 64-bit usize");

pub(crate) use portable;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::accessory::{AccessoryArena, AccessoryDb, StatArena};
    use crate::card::{Card, CardDb, CardMeta, LeaderSkill, LeaderTargets};
    use crate::scalar::{Attribute, CharacterId, Roster, Stats};
    use crate::sis::SisDb;
    use crate::skill::{SkillArena, Targets};
    use crate::song::{ChartArena, SongDb};

    fn built() -> CardDb {
        CardDb {
            cards: vec![Card {
                stats: [Stats([1, 2, 3]), Stats([4, 5, 6])],
                skill: None,
                leader: crate::card::LeaderId(0),
                character: CharacterId(0),
                attribute: Attribute::Smile,
            }],
            meta: vec![CardMeta {
                card_id: 66,
                rarity: crate::scalar::Rarity::UR,
                sis_base: [4, 4],
                sis_max: [4, 8],
                display_name: "Initial Honoka".into(),
                is_promo: false,
                is_preidolized_non_promo: false,
            }],
            skills: SkillArena::default(),
            skill_targets: Targets::default(),
            leaders: vec![LeaderSkill {
                attribute: Attribute::Smile,
                secondary: None,
                value: 0,
                extra: None,
            }],
            leader_targets: LeaderTargets::default(),
            roster: Roster::build(["Kousaka Honoka"]),
        }
    }

    fn accessories() -> AccessoryDb {
        AccessoryDb {
            accessories: vec![],
            meta: vec![],
            skills: AccessoryArena::default(),
            stats: StatArena::default(),
            targets: Targets::default(),
        }
    }

    fn sis() -> SisDb {
        SisDb { sis: vec![], meta: vec![], targets: Targets::default() }
    }

    fn songs() -> SongDb {
        SongDb {
            songs: vec![],
            charts: ChartArena::default(),
            groups: Targets::default(),
        }
    }

    /// Magic is what keeps the different kinds off each other's files
    /// pid is what keeps the two concurrent `cargo test` runs off them.
    fn path<T>(name: &str) -> PathBuf
    where
        T: Kind,
    {
        let kind = str::from_utf8(&T::MAGIC).expect("magics are ASCII");
        let pid = std::process::id();
        std::env::temp_dir().join(format!("llsif-{name}-{kind}-{pid}.rkyv"))
    }

    /// Writes a good archive of one kind, corrupts it, and expects `open` to refuse.
    fn refuses<T>(name: &str, value: &T, edit: impl FnOnce(&mut Vec<u8>))
    where
        T: Kind
            + Archive
            + for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, rancor::Error>>,
        T::Archived: Portable,
    {
        let path = path::<T>(name);
        write(&path, value).expect("write");
        let mut bytes = std::fs::read(&path).expect("read back");
        edit(&mut bytes);
        std::fs::write(&path, &bytes).expect("rewrite");

        let kind = str::from_utf8(&T::MAGIC).expect("magics are ASCII");
        // SAFETY: Test owns path.
        match unsafe { Mapped::<T>::open(&path) } {
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{kind} {name}"),
            Ok(_) => panic!("`open` accepted a {name} {kind} archive"),
        }
        std::fs::remove_file(&path).expect("cleanup");
    }

    fn rejected(name: &str, edit: impl Fn(&mut Vec<u8>)) {
        refuses(name, &built(), &edit);
        refuses(name, &accessories(), &edit);
        refuses(name, &sis(), &edit);
        refuses(name, &songs(), &edit);
    }

    #[test]
    fn check_the_card_db_survives_the_archive() {
        let db = built();
        let path = path::<CardDb>("round-trip");
        write(&path, &db).expect("write");

        // SAFETY: `write` just produced this file and nothing else touches it.
        let mapped: Mapped<CardDb> = unsafe { Mapped::open(&path) }.expect("open");
        let body = mapped.map.as_ptr() as usize + HEADER;
        assert_eq!(body % 16, 0, "the header misaligned the body");
        // `CardDb` has no `Debug`, so this cannot be `assert_eq!`.
        assert!(mapped.root() == &db, "the archive is not the database it was built from");

        drop(mapped);
        std::fs::remove_file(&path).expect("cleanup");
    }

    #[test]
    fn check_a_bad_version_is_rejected() {
        rejected("wrong-version", |b| b[8] ^= 1); // what a layout change moves
    }

    #[test]
    fn check_a_bad_magic_is_rejected() {
        rejected("foreign-kind", |b| b[0] ^= 1);
    }

    #[test]
    fn check_a_file_shorter_than_the_header_is_rejected() {
        rejected("stub", |b| b.truncate(8));
    }

    #[test]
    fn check_a_headerless_body_is_rejected() {
        rejected("no-body", |b| b.truncate(HEADER));
    }

    #[test]
    fn check_the_kinds_do_not_share_a_magic() {
        let mut magics = [CardDb::MAGIC, AccessoryDb::MAGIC, SisDb::MAGIC, SongDb::MAGIC];
        magics.sort_unstable();
        assert!(magics.windows(2).all(|w| w[0] != w[1]), "two kinds share a magic");
    }

    #[test]
    fn check_one_kind_does_not_open_as_another() {
        let path = path::<SisDb>("cross-kind");
        write(&path, &sis()).expect("write");

        // SAFETY: Test owns path.
        match unsafe { Mapped::<AccessoryDb>::open(&path) } {
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidData),
            Ok(_) => panic!("`open` accepted a SIS archive as an accessory one"),
        }
        std::fs::remove_file(&path).expect("cleanup");
    }
}
