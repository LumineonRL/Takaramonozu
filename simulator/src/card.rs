use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::{Kind, portable};
use crate::scalar::{Attribute, CharacterId, CharacterSet, Form, Pct, Rarity, Roster, Stats};
use crate::skill::{self, SkillArena, SkillId};

/// card index needed for team building to pair accessory to a card
/// Not used in hot loop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CardIdx(pub u16);

/// Index into the leader-skill table, of which there are 313.
/// Made Slot 0 is the identity leader since N cards have no leader skill
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct LeaderId(pub u16);

/// One of the 74 `leader_extra_target` values, which could be groups, sub groups,
/// year groups, or character pair groups like `"Eli, Umi"`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct LeaderTargetId(u8);

impl LeaderTargetId {
    /// # Panics
    /// Past 256 targets. There are 74.
    #[must_use]
    #[expect(clippy::expect_used, reason = "capacity bound, documented above")]
    pub fn from_index(i: usize) -> Self {
        Self(u8::try_from(i).expect("leader target table fits u8"))
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Card {
    /// Unidolized and Idolized
    pub stats: [Stats; 2],
    pub skill: Option<SkillId>,
    pub leader: LeaderId,
    pub character: CharacterId,
    pub attribute: Attribute,
}

impl Card {
    #[inline]
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "You're either idolized or you're not.")]
    pub const fn stats(&self, form: Form) -> Stats {
        self.stats[form as usize]
    }
}

/// Information about a card that matters for users adding cards to their Deck
/// but that does NOT matter for team building / the simulation itself.
#[derive(Clone, PartialEq, Eq, Debug, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct CardMeta {
    pub card_id: u16,
    pub rarity: Rarity,
    /// u8 because max is 8. length 2 for unidolized and idolized.
    pub sis_base: [u8; 2],
    pub sis_max: [u8; 2],
    pub hp: [u8; 2],
    /// Cosmetic
    pub display_name: Box<str>,
    /// `is_promo` indicates what schoido.lu called a promo
    /// `is_preidolized_non_promo` was my manual edit to differentiate actual promos
    /// from cards that function as normal cards that just so happened to come pre-idolized.
    pub is_promo: bool,
    pub is_preidolized_non_promo: bool,
}

/// A `Guest` is exactly this with no card attached.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C)]
pub struct LeaderSkill {
    /// The stat that receives the boost.
    pub attribute: Attribute,
    /// For effects like "boost Smile by 12% of Pure", this would store the "Pure" part.
    pub secondary: Option<Attribute>,
    pub value: Pct,
    pub extra: Option<LeaderExtra>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C)]
pub struct LeaderExtra {
    pub attribute: Attribute,
    pub target: LeaderTargetId,
    pub value: Pct,
}

/// Encodes `leader_extra_target` strings into bitmasks.
/// Should be 74 unique extra targets.
#[derive(Clone, PartialEq, Eq, Default, Debug, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct LeaderTargets {
    names: Vec<Box<str>>,
    sets: Vec<CharacterSet>,
}

impl LeaderTargets {
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    #[must_use]
    pub fn set(&self, id: LeaderTargetId) -> Option<CharacterSet> {
        self.sets.get(id.index()).copied()
    }

    #[must_use]
    pub fn name(&self, id: LeaderTargetId) -> Option<&str> {
        self.names.get(id.index()).map(Box::as_ref)
    }

    /// # Errors
    /// Whatever `resolve` returns for a name that is not already interned.
    ///
    /// # Panics
    /// Past 256 targets, which is what a [`LeaderTargetId`] holds. There are 74.
    pub fn intern<E>(
        &mut self,
        name: &str,
        resolve: impl FnOnce() -> Result<CharacterSet, E>,
    ) -> Result<LeaderTargetId, E> {
        if let Some(i) = self.names.iter().position(|n| **n == *name) {
            return Ok(LeaderTargetId::from_index(i));
        }
        let set = resolve()?;
        let i = self.names.len();
        self.names.push(name.into());
        self.sets.push(set);
        Ok(LeaderTargetId::from_index(i))
    }
}

/// Every card table the simulator reads, and the unit the archive is written
/// in. Assembling one is the preprocessor's job: see `load::card::build`.
#[derive(Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct CardDb {
    pub cards: Vec<Card>,
    pub meta: Vec<CardMeta>,
    pub skills: SkillArena,
    pub skill_targets: skill::Targets,
    pub leaders: Vec<LeaderSkill>,
    pub leader_targets: LeaderTargets,
    pub roster: Roster,
}

impl Kind for CardDb {
    const MAGIC: [u8; 8] = *b"LLSIFCDB";
    const VERSION: u32 = 2;
}

impl CardDb {
    /// `card_id` is what a saved deck stores, since [`CardIdx`] is a position in
    /// this table and moves whenever the corpus does.
    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "len checked at build")]
    pub fn index(&self, card_id: u16) -> Option<CardIdx> {
        let i = self.meta.binary_search_by_key(&card_id, |m| m.card_id).ok()?;
        Some(CardIdx(i as u16))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }
}

portable!(LeaderId, LeaderTargetId, LeaderExtra, LeaderSkill, Card);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_the_hot_half_stays_small() {
        assert_eq!(size_of::<Card>(), 18);
        assert_eq!(size_of::<rkyv::Archived<Card>>(), 18);
    }
}
