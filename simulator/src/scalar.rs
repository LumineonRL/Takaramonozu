//! In essence this takes the data from `cards.json` that does not exist as an
//! integer, and defines it as an integer.
//!
//! So it takes the floats and represents them as integers, which we can do
//! since we have a fixed number of decimal places.

use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::portable;

/// Hundredths. Used for Skill `value`s, whose meaning changes depending on the
/// effect.
///
/// A skill's value is actually really annoying to represent because of how its
/// meaning changes depending on the type of skill it's attached to so I'll mark
/// this as a TBD that might change in the future.
///
/// Max is a flat score for a Scorer (`14_824_000` = 148240.00).
pub type Centi = u32;

/// Milliseconds. Used for Skill `duration`s; max observed `43_000`.
pub type Millis = u16;

/// Percent as written on the card: `93` means 93%, i.e. 0.93. Activation
/// chance (max 93) and leader values (max 15).
///
/// Note that this is a storage type for rates that came out of cards.json, all of
/// which are 0..=100. However, It is deliberately too narrow for a rate as the simulator will
/// need to handle because Skill Rate Up + Amplify shenanigans can technicaly get a skill activation chance
/// to exceed 256% (I think... I actually need to check this edge case. Mark it as a TODO).
pub type Pct = u8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Attribute {
    Smile = 0,
    Pure = 1,
    Cool = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Rarity {
    N = 0,
    R = 1,
    SR = 2,
    SSR = 3,
    UR = 4,
}

/// Even pre-idolized cards technically have unidolized stat values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Form {
    Unidolized = 0,
    Idolized = 1,
}

/// One of 99 potential character values including all N girls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct CharacterId(pub u8);

/// The roster fits in a `u128`, so every group, year group and leader-extra
/// target, including ship card targets.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct CharacterSet(pub u128);

impl CharacterSet {
    #[inline]
    #[must_use]
    pub const fn contains(self, c: CharacterId) -> bool {
        ((self.0 >> c.0) & 1) == 1
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct Roster {
    names: Vec<Box<str>>,
}

impl Roster {
    /// # Panics
    /// Past 128 names, which is what a `CharacterSet` bitmask holds.
    /// There are 99 unique names.
    /// But idk what if someone wants to mod / extend the sim with new characters
    /// one day? Or what if custom characters become a thing on private servers
    /// one day?
    #[must_use]
    pub fn build<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut names: Vec<Box<str>> = names.into_iter().map(Box::from).collect();
        names.sort_unstable();
        names.dedup();
        assert!(names.len() <= 128, "{} characters do not fit a CharacterSet", names.len());
        Self { names }
    }

    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "len checked at build")]
    pub fn id(&self, name: &str) -> Option<CharacterId> {
        let i = self.names.binary_search_by(|n| (**n).cmp(name)).ok()?;
        Some(CharacterId(i as u8))
    }

    #[must_use]
    pub fn name(&self, id: CharacterId) -> Option<&str> {
        self.names.get(id.0 as usize).map(Box::as_ref)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Smile/Pure/Cool indexed by [`Attribute`], so selecting the song attribute is
/// an index rather than a branch.
///
/// Max base stat is 8940; the mutable per-trial copy is wider, since runtime
/// boosts with galleries and appeal boost can push well past `u16`.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct Stats(pub [u16; 3]);

impl Stats {
    #[inline]
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`Attribute` has exactly these 3 possibilities")]
    pub const fn get(self, a: Attribute) -> u16 {
        self.0[a as usize]
    }
}

portable!(Attribute, Rarity, CharacterId, CharacterSet, Stats);
