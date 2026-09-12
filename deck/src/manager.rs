use core::fmt;
use core::marker::PhantomData;
use core::mem;
use std::io;
use std::path::Path;

use serde::de::{DeserializeOwned, Deserializer, Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

const MAX_ROWS: usize = u16::MAX as usize;

pub trait Flag: Copy {
    fn bit(self) -> u8;
}

/// I hate this name too. This really just like, defaults if the
/// user enters an invalid value. Like, if a user tries to add an
/// unidolized promo card, it "repairs" it by making it idolized.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct Repairs(u8);

impl Repairs {
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub fn contains<F>(self, repair: F) -> bool
    where
        F: Flag,
    {
        self.0 & repair.bit() != 0
    }

    pub fn insert<F>(&mut self, repair: F)
    where
        F: Flag,
    {
        self.0 |= repair.bit();
    }
}

pub trait Entry: Copy + Eq + fmt::Debug {
    type Db: ?Sized;
    type Problem: Copy + Eq + fmt::Debug;
    const CAPACITY: usize;
    const FULL: Self::Problem;

    /// # Errors
    /// Any and every rule the entry breaks.
    fn vet(self, db: &Self::Db) -> Result<(Self, Repairs), Vec<Self::Problem>>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct EntryId(u16);

impl EntryId {
    /// # Panics
    /// Past 65,536 entries, which `MAX_ROWS` is what stops.
    #[must_use]
    #[expect(clippy::expect_used, reason = "capacity bound, documented above")]
    pub fn from_index(i: usize) -> Self {
        Self(u16::try_from(i).expect("entry index fits u16"))
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rejected<E: Entry> {
    pub row: usize,
    pub entry: E,
    pub problems: Vec<E::Problem>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Repaired {
    pub row: usize,
    pub id: EntryId,
    pub repairs: Repairs,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loaded<M: Manager> {
    pub value: M,
    pub repaired: Vec<Repaired>,
    pub rejected: Vec<Rejected<M::Entry>>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Slots<E: Entry>(Vec<Option<E>>);

impl<E: Entry> Default for Slots<E> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<E: Entry> Slots<E> {
    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.iter().flatten().count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() >= E::CAPACITY
    }

    #[must_use]
    pub fn get(&self, id: EntryId) -> Option<E> {
        self.0.get(id.index()).copied().flatten()
    }

    pub fn remove(&mut self, id: EntryId) -> Option<E> {
        self.0.get_mut(id.index())?.take()
    }

    pub fn entries(&self) -> impl Iterator<Item = (EntryId, E)> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.map(|entry| (EntryId::from_index(i), entry)))
    }

    pub(crate) fn insert(&mut self, entry: E) -> Option<EntryId> {
        if self.is_full() {
            return None;
        }
        let index =
            if let Some((i, hole)) = self.0.iter_mut().enumerate().find(|(_, e)| e.is_none()) {
                *hole = Some(entry);
                i
            } else {
                let i = self.0.len();
                self.0.push(Some(entry));
                i
            };
        Some(EntryId::from_index(index))
    }

    pub(crate) fn add(
        &mut self,
        db: &E::Db,
        entry: E,
    ) -> Result<(EntryId, Repairs), Vec<E::Problem>> {
        if self.is_full() {
            return Err(vec![E::FULL]);
        }
        let (entry, repairs) = entry.vet(db)?;
        let id = self.insert(entry).ok_or_else(|| vec![E::FULL])?;
        Ok((id, repairs))
    }

    fn vet(self, db: &E::Db) -> (Self, Vec<Repaired>, Vec<Rejected<E>>) {
        let mut out = Self::default();
        let (mut repaired, mut rejected) = (Vec::new(), Vec::new());

        for (row, entry) in self.0.into_iter().enumerate() {
            let Some(entry) = entry else { continue };
            match entry.vet(db) {
                Ok((entry, repairs)) => match out.insert(entry) {
                    Some(id) if !repairs.is_empty() => {
                        repaired.push(Repaired { row, id, repairs });
                    }
                    Some(_) => {}
                    None => {
                        rejected.push(Rejected { row, entry, problems: vec![E::FULL] });
                    }
                },
                Err(problems) => rejected.push(Rejected { row, entry, problems }),
            }
        }
        (out, repaired, rejected)
    }
}

impl<E: Entry + Serialize> Serialize for Slots<E> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(self.0.iter().flatten())
    }
}

impl<'de, E> Deserialize<'de> for Slots<E>
where
    E: Entry + Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Rows<E>(PhantomData<E>);

        impl<'de, E> Visitor<'de> for Rows<E>
        where
            E: Entry + Deserialize<'de>,
        {
            type Value = Slots<E>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "at most {MAX_ROWS} entries")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(E::CAPACITY));
                while let Some(entry) = seq.next_element()? {
                    if out.len() == MAX_ROWS {
                        return Err(A::Error::custom(format!("past {MAX_ROWS} entries")));
                    }
                    out.push(entry);
                }
                Ok(Slots(out))
            }
        }

        deserializer.deserialize_seq(Rows(PhantomData))
    }
}

pub trait Manager: Serialize + DeserializeOwned + Sized {
    type Entry: Entry;

    fn slots(&self) -> &Slots<Self::Entry>;
    fn slots_mut(&mut self) -> &mut Slots<Self::Entry>;

    #[must_use]
    fn len(&self) -> usize {
        self.slots().len()
    }

    #[must_use]
    fn is_empty(&self) -> bool {
        self.slots().is_empty()
    }

    #[must_use]
    fn is_full(&self) -> bool {
        self.slots().is_full()
    }

    #[must_use]
    fn get(&self, id: EntryId) -> Option<Self::Entry> {
        self.slots().get(id)
    }

    fn remove(&mut self, id: EntryId) -> Option<Self::Entry> {
        self.slots_mut().remove(id)
    }

    fn entries(&self) -> impl Iterator<Item = (EntryId, Self::Entry)> {
        self.slots().entries()
    }

    #[must_use]
    fn vet(mut self, db: &<Self::Entry as Entry>::Db) -> Loaded<Self> {
        let (slots, repaired, rejected) = mem::take(self.slots_mut()).vet(db);
        *self.slots_mut() = slots;
        Loaded { value: self, repaired, rejected }
    }

    /// # Errors
    /// An I/O issue. Not a `Manager`. A file that *is* a managre but
    /// carries but has bad rows.
    fn load(path: &Path, db: &<Self::Entry as Entry>::Db) -> io::Result<Loaded<Self>> {
        Self::parse(&std::fs::read(path)?, db)
    }

    /// # Errors
    /// As [`Manager::load`], minus the I/O.
    fn parse(bytes: &[u8], db: &<Self::Entry as Entry>::Db) -> io::Result<Loaded<Self>> {
        let manager: Self = serde_json::from_slice(bytes)?;
        Ok(manager.vet(db))
    }

    /// # Errors
    /// I/O skill issue.
    fn save(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_json()?)
    }

    /// # Errors
    /// Serialization, which one of these shouldn't actually fail in the first place.
    fn to_json(&self) -> io::Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
}
