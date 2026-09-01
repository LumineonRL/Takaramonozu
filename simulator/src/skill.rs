//! A skill contains 4 main components that scale with level -
//! value, duration, chance, and threshold (well threshold usually doesn't change. But it can.)
//!
//! The main consideration here is how the `Amplify` skill interacts with these skill
//! components as we potentially will need to look up different skill levels inside of
//! the hot loop. We also don't want to carry more values than we need to because of
//! cache-lines and all that.
//! For this reason, `(threshold, chance)` are armed, resolved once per card at team-compile time
//! from the card's own level and never read again, because Amplify moves the
//! effect and not the trigger. `(value, duration)` is payload: read only on
//! activation, indexed by the effective (potentially amplified) level. The two
//! halves therefore live in different arenas rather than in one row.

use core::num::{NonZeroU8, NonZeroU16};

use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::portable;
use crate::scalar::{Centi, Millis, Pct};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum SkillEffect {
    Scorer,
    Healer,
    PerfectLock,
    Encore,
    Amplify,
    TotalTrick,
    PerfectScoreUp,
    SkillRateUp,
    AppealBoost,
    ComboBonusUp,
    Sync,
    Spark,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum SkillTrigger {
    RhythmIcons,
    Perfects,
    Combo,
    Time,
    Score,
    StarNotes,
    YearGroup,
}

/// One of the 14 distinct skill target strings, set on 100 cards. Resolves to
/// a `CharacterSet` at load. `NonZero` so `Option<TargetId>` is 1 byte.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct TargetId(NonZeroU8);

impl TargetId {
    /// # Panics
    /// Past 255 targets. There are 14.
    #[must_use]
    pub fn from_index(i: usize) -> Self {
        let slot = u8::try_from(i + 1).expect("target table fits u8");
        Self(NonZeroU8::new(slot).expect("slots are 1 based"))
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0.get() as usize - 1
    }
}

/// Index into [`SkillArena::defs`].
///
/// Slot 0 is deliberately left unused, so 0 is not a valid id - which is what
/// `NonZeroU16` says. That lets the compiler spend the now-impossible 0 on
/// representing `None`, so `Option<SkillId>` is the same 2 bytes as a bare
/// `SkillId` and the 321 cards with no skill pay nothing for the field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct SkillId(pub NonZeroU16);

/// What it takes to arm. `threshold` is integral in every row of the source and
/// tops out at 18000 (from a score based scorer); Star Notes stores 1.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
#[repr(C)]
pub struct Arming {
    pub threshold: u16,
    pub chance: Pct,
    /// rkyv will not write a type whose padding is uninitialized.
    _pad: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct SkillDef {
    pub effect: SkillEffect,
    pub trigger: SkillTrigger,
    pub target: Option<TargetId>,
    /// Row count of the payload table, which is the amplify ceiling rather than
    /// the skill cap: an Encore (8 rows) at level 5 under a +7 Amplify resolves
    /// at 8, not 12. Lookups saturate here instead of bounds-checking.
    pub rows: u8,
    /// `[payload, payload + rows)` in the value/duration arenas.
    pub payload: u32,
    /// Base of this skill's 8 arming rows. Always 8: base level is 1..=8 for
    /// every card in the game, and rows past 8 padded.
    pub arming: u32,
}

#[derive(Default, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct SkillArena {
    pub defs: Vec<SkillDef>,
    pub values: Vec<Centi>,
    pub durations: Vec<Millis>,
    pub arming: Vec<Arming>,
}

impl SkillArena {
    /// `level` is effective (post-Amplify) and 1-based; Amplify reaches +16 on
    /// top of a base 8, so requests up to 24 arrive here and clamp.
    #[inline]
    #[must_use]
    pub fn payload(&self, def: &SkillDef, level: u8) -> (Centi, Millis) {
        let row = def.payload as usize + (level.clamp(1, def.rows) as usize - 1);
        (self.values[row], self.durations[row])
    }

    /// `level` is the card's own level, 1..=8.
    #[inline]
    #[must_use]
    pub fn arming(&self, def: &SkillDef, level: u8) -> Arming {
        self.arming[def.arming as usize + (level.clamp(1, 8) as usize - 1)]
    }
}

portable!(SkillEffect, SkillTrigger, TargetId, SkillId, Arming, SkillDef);
