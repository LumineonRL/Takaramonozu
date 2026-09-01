use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::portable;
use crate::scalar::{Attribute, CharacterId, Pct, Rarity, Stats};
use crate::skill::SkillId;

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

portable!(LeaderId, LeaderTargetId, LeaderExtra, LeaderSkill, Card);
