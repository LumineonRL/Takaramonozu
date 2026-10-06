//! A skill contains 4 main components that scale with level -
//! value, duration, chance, and threshold (well threshold usually doesn't change. But it can.)
//!
//! The main consideration here is how the `Amplify` skill interacts with these skill
//! components as we potentially will need to look up different skill levels inside of
//! the hot loop. We also don't want to carry more values than we need to because of
//! cache-lines and all that.
//! For this reason, `(threshold, chance)` is the trigger, resolved once per card at
//! team-compile time from the card's own level and never read again, because Amplify
//! moves the effect and not the trigger. `(value, duration)` is payload: read only on
//! activation, indexed by the effective (potentially amplified) level. The two
//! halves therefore live in different arenas rather than in one row.

use core::num::{NonZeroU8, NonZeroU16};

use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::portable;
use crate::scalar::{Centi, CharacterSet, Ms, Pct};

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
    /// Schoolido.lu, which I grabbed much of my data from, called this "Combo Bonus Up".
    /// The community refers to it as Combo Fever though.
    /// I stopped playing EN so no idea what the EN name was in-game.
    ComboFever,
    Sync,
    Spark,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum TriggerType {
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
    #[expect(clippy::expect_used, reason = "capacity bound, documented above")]
    pub fn from_index(i: usize) -> Self {
        let slot = u8::try_from(i)
            .ok()
            .and_then(|n| n.checked_add(1))
            .and_then(NonZeroU8::new)
            .expect("target table fits u8");
        Self(slot)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0.get().saturating_sub(1) as usize
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

/// What it takes to fire. `threshold` is integral in every row of the source and
/// tops out at 18000 (from a score based scorer); Star Notes stores 1.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
#[repr(C)]
pub struct Trigger {
    pub threshold: u16,
    pub chance: Pct,
    /// rkyv will not write a type whose padding is uninitialized.
    _pad: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct SkillDef {
    pub effect: SkillEffect,
    pub trigger_type: TriggerType,
    pub target: Option<TargetId>,
    /// Row count of the payload table, which is the amplify ceiling rather than
    /// the skill cap: an Encore (8 rows) at level 5 under a +7 Amplify resolves
    /// at 8, not 12. Lookups saturate here instead of bounds-checking.
    pub rows: u8,
    payload: u32,
    trigger: u32,
}

/// Encodes the 14 distinct skill target strings into bitmasks, out of
/// `year_group_target.json`.
#[derive(Clone, PartialEq, Eq, Default, Debug, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
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

    /// # Errors
    /// Whatever `resolve` returns for a name that is not already interned.
    ///
    /// # Panics
    /// Past 255 targets, which is what a [`TargetId`] holds. There are 14.
    pub fn intern<E>(
        &mut self,
        name: &str,
        resolve: impl FnOnce() -> Result<CharacterSet, E>,
    ) -> Result<TargetId, E> {
        if let Some(i) = self.names.iter().position(|n| **n == *name) {
            return Ok(TargetId::from_index(i));
        }
        let set = resolve()?;
        let i = self.names.len();
        self.names.push(name.into());
        self.sets.push(set);
        Ok(TargetId::from_index(i))
    }
}

pub const MAX_BASE_LEVEL: u8 = 8;

/// Amp doesn't modify threshold/chance, only value/duration.
/// See: `Payload` for the value/duration.
pub const TRIGGER_ROWS: u8 = MAX_BASE_LEVEL;

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct SkillArena {
    defs: Vec<SkillDef>,
    values: Vec<Centi>,
    durations: Vec<Ms>,
    triggers: Vec<Trigger>,
}

impl Default for SkillArena {
    fn default() -> Self {
        Self {
            defs: vec![SkillDef {
                effect: SkillEffect::Scorer,
                trigger_type: TriggerType::Combo,
                target: None,
                rows: 0,
                payload: 0,
                trigger: 0,
            }],
            values: Vec::new(),
            durations: Vec::new(),
            triggers: Vec::new(),
        }
    }
}

impl SkillArena {
    /// # Panics
    /// Empty payload, past 255 rows, past 65535 skills.
    #[expect(clippy::expect_used, reason = "capacity bounds, documented above")]
    pub fn push(
        &mut self,
        effect: SkillEffect,
        trigger_type: TriggerType,
        target: Option<TargetId>,
        triggers: &[Trigger; TRIGGER_ROWS as usize],
        payload: &[(Centi, Ms)],
    ) -> SkillId {
        assert!(!payload.is_empty(), "a skill with no payload rows would read the next skill's");
        let def = SkillDef {
            effect,
            trigger_type,
            target,
            rows: u8::try_from(payload.len()).expect("16 payload rows at most"),
            payload: u32::try_from(self.values.len()).expect("payload arena fits u32"),
            trigger: u32::try_from(self.triggers.len()).expect("trigger arena fits u32"),
        };
        for &(value, duration) in payload {
            self.values.push(value);
            self.durations.push(duration);
        }
        self.triggers.extend_from_slice(triggers);

        let slot = u16::try_from(self.defs.len()).expect("skill table fits u16");
        self.defs.push(def);
        SkillId(NonZeroU16::new(slot).expect("`default` seeded the dead slot 0"))
    }

    #[must_use]
    pub fn trigger_rows(&self) -> usize {
        self.triggers.len()
    }
}

pub trait SkillRows {
    fn defs(&self) -> &[SkillDef];
    fn triggers(&self) -> &[Trigger];
    fn payload_row(&self, row: usize) -> (Centi, Ms);

    #[must_use]
    fn def(&self, id: SkillId) -> Option<&SkillDef> {
        self.defs().get(id.0.get() as usize)
    }

    /// `level` is effective (post-Amplify) and 1-based; Amplify reaches +16 on
    /// top of a base 8, so requests up to 24 arrive here and clamp.
    #[inline]
    #[must_use]
    #[expect(clippy::arithmetic_side_effects, reason = "u32 + u8; see `archive`'s usize assert")]
    fn payload(&self, def: &SkillDef, level: u8) -> (Centi, Ms) {
        self.payload_row(def.payload as usize + level.min(def.rows).saturating_sub(1) as usize)
    }

    /// `level` is the card's own level, 1..=8.
    #[inline]
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated TRIGGER_ROWS at `def.trigger`")]
    #[expect(clippy::arithmetic_side_effects, reason = "u32 + u8; see `archive`'s usize assert")]
    fn trigger(&self, def: &SkillDef, level: u8) -> Trigger {
        self.triggers()[def.trigger as usize + level.min(TRIGGER_ROWS).saturating_sub(1) as usize]
    }
}

impl SkillRows for SkillArena {
    fn defs(&self) -> &[SkillDef] {
        &self.defs
    }

    fn triggers(&self) -> &[Trigger] {
        &self.triggers
    }

    #[inline]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated `def.rows` at `def.payload`")]
    fn payload_row(&self, row: usize) -> (Centi, Ms) {
        (self.values[row], self.durations[row])
    }
}

impl SkillRows for ArchivedSkillArena {
    fn defs(&self) -> &[SkillDef] {
        &self.defs
    }

    fn triggers(&self) -> &[Trigger] {
        &self.triggers
    }

    #[inline]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated `def.rows` at `def.payload`")]
    fn payload_row(&self, row: usize) -> (Centi, Ms) {
        (self.values[row].to_native(), self.durations[row].to_native())
    }
}

portable!(SkillEffect, TriggerType, TargetId, SkillId, Trigger, SkillDef);
