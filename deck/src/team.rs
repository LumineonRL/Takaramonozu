//! A slot for each lane + a guest

use simulator::accessory::AccessoryDb;
use simulator::card::CardDb;
use simulator::sis::SisDb;

use crate::accessory::Accessories;
use crate::card::{Deck, DeckEntry};
use crate::manager::{EntryId, Manager};
use crate::sis::{SisEntry, SisInventory};

pub const LANES: usize = 9;

/// Lane 5
pub const CENTER: usize = 4;

/// The absolute most you could equip on one card using the minimum-cost SIS
/// would be 1+1+1+2+3, so 5 is the most SIS one card can hold.
pub const MAX_SIS_PER_CARD: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    // Not a valid lane.
    NoLane,
    /// Already equipped elsewhere on this team.
    InUse,
    /// A card holds at most one of a given SIS.
    Duplicate,
    /// Out of SIS capacity.
    NoRoom,
    /// The accessory's `card_id`/character restriction.
    Restricted,
    Unknown,
}

#[derive(Clone, Copy)]
pub struct Context<'a> {
    pub deck: &'a Deck,
    pub accessories: &'a Accessories,
    pub inventory: &'a SisInventory,
    pub cards: &'a CardDb,
    pub acc_db: &'a AccessoryDb,
    pub sis_db: &'a SisDb,
}

impl Context<'_> {
    fn equips(self, acc: EntryId, card: EntryId) -> Option<bool> {
        let acc = self.accessories.get(acc)?;
        let meta = self.acc_db.meta.get(usize::from(self.acc_db.index(acc.acc_id())?.0))?;
        let card_id = self.deck.get(card)?.card_id();
        let wearer = self.cards.cards.get(usize::from(self.cards.index(card_id)?.0))?;
        Some(meta.equips(acc.skill_level(), card_id, wearer.character))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TeamSlot {
    card: EntryId,
    accessory: Option<EntryId>,
    sis: [Option<EntryId>; MAX_SIS_PER_CARD],
}

impl TeamSlot {
    const fn new(card: EntryId) -> Self {
        Self { card, accessory: None, sis: [None; MAX_SIS_PER_CARD] }
    }

    // Maybe not disabling the `must_use` lint was a mistake. Most of
    // these are just irrelvant.
    #[must_use]
    pub const fn card(self) -> EntryId {
        self.card
    }

    #[must_use]
    pub const fn accessory(self) -> Option<EntryId> {
        self.accessory
    }

    pub fn sis(self) -> impl Iterator<Item = EntryId> {
        self.sis.into_iter().flatten()
    }

    #[must_use]
    pub fn slots_used(self, ctx: Context) -> u8 {
        self.sis()
            .filter_map(|id| ctx.inventory.sis(id, ctx.sis_db))
            .fold(0_u8, |n, s| n.saturating_add(s.slots))
    }

    #[must_use]
    pub fn capacity(self, ctx: Context) -> u8 {
        ctx.deck.get(self.card).map_or(0, DeckEntry::sis_slots)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Team {
    slots: [Option<TeamSlot>; LANES],
    pub guest: Option<u16>,
}

impl Team {
    #[must_use]
    pub fn slot(&self, lane: usize) -> Option<TeamSlot> {
        self.slots.get(lane).copied().flatten()
    }

    pub fn lanes(&self) -> impl Iterator<Item = (usize, TeamSlot)> {
        self.slots.iter().enumerate().filter_map(|(l, s)| s.map(|slot| (l, slot)))
    }

    fn at(&mut self, lane: usize) -> Result<&mut Option<TeamSlot>, Problem> {
        self.slots.get_mut(lane).ok_or(Problem::NoLane)
    }

    /// # Errors
    /// A card not in the deck, a card already in another lane.
    pub fn equip_card(&mut self, lane: usize, card: EntryId, ctx: Context) -> Result<(), Problem> {
        if ctx.deck.get(card).is_none() {
            return Err(Problem::Unknown);
        }
        if self.lanes().any(|(l, s)| l != lane && s.card == card) {
            return Err(Problem::InUse);
        }
        *self.at(lane)? = Some(TeamSlot::new(card));
        Ok(())
    }

    /// # Errors
    /// Empty lane, unowned accessory, one already worn in another lane,
    /// or invalid on this card.
    pub fn equip_accessory(
        &mut self,
        lane: usize,
        acc: EntryId,
        ctx: Context,
    ) -> Result<(), Problem> {
        let mut slot = self.slot(lane).ok_or(Problem::NoLane)?;
        if self.lanes().any(|(_, s)| s.accessory == Some(acc)) {
            return Err(Problem::InUse);
        }
        if !ctx.equips(acc, slot.card).ok_or(Problem::Unknown)? {
            return Err(Problem::Restricted);
        }
        slot.accessory = Some(acc);
        *self.at(lane)? = Some(slot);
        Ok(())
    }

    /// # Errors
    /// Empty lane, unowned SIS, copy already equipped elsewhere, a
    /// second of the same SIS on one card, or no capacity left.
    pub fn equip_sis(&mut self, lane: usize, sis: EntryId, ctx: Context) -> Result<(), Problem> {
        let mut slot = self.slot(lane).ok_or(Problem::NoLane)?;
        let added = ctx.inventory.sis(sis, ctx.sis_db).ok_or(Problem::Unknown)?;
        if self.lanes().any(|(_, s)| s.sis().any(|held| held == sis)) {
            return Err(Problem::InUse);
        }
        let kind = |e: EntryId| ctx.inventory.get(e).map(SisEntry::sis_id);
        if slot.sis().any(|held| kind(held) == kind(sis)) {
            return Err(Problem::Duplicate);
        }
        if slot.slots_used(ctx).saturating_add(added.slots) > slot.capacity(ctx) {
            return Err(Problem::NoRoom);
        }
        *slot.sis.iter_mut().find(|s| s.is_none()).ok_or(Problem::NoRoom)? = Some(sis);
        *self.at(lane)? = Some(slot);
        Ok(())
    }

    /// # Errors
    /// Empty lane, or a SIS this card is not holding.
    pub fn unequip_sis(&mut self, lane: usize, sis: EntryId) -> Result<(), Problem> {
        let mut slot = self.slot(lane).ok_or(Problem::NoLane)?;
        *slot.sis.iter_mut().find(|s| **s == Some(sis)).ok_or(Problem::Unknown)? = None;
        *self.at(lane)? = Some(slot);
        Ok(())
    }

    /// # Errors
    /// Past lane 9.
    pub fn clear(&mut self, lane: usize) -> Result<(), Problem> {
        *self.at(lane)? = None;
        Ok(())
    }
}
