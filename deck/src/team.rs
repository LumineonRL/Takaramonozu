//! A slot for each lane + a guest

use simulator::accessory::AccessoryDb;
use simulator::card::{CardDb, LeaderId};
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
    guest: Option<LeaderId>,
}

impl Team {
    #[must_use]
    pub fn slot(&self, lane: usize) -> Option<TeamSlot> {
        self.slots.get(lane).copied().flatten()
    }

    #[must_use]
    pub const fn guest(&self) -> Option<LeaderId> {
        self.guest
    }

    /// # Errors
    /// An id past the leader table.
    pub fn set_guest(&mut self, guest: LeaderId, ctx: Context) -> Result<(), Problem> {
        if ctx.cards.leaders.get(usize::from(guest.0)).is_none() {
            return Err(Problem::Unknown);
        }
        self.guest = Some(guest);
        Ok(())
    }

    pub const fn clear_guest(&mut self) {
        self.guest = None;
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

#[cfg(test)]
mod tests {
    use super::{Context, Problem, Team};
    use crate::accessory::{Accessories, tests::accessories};
    use crate::card::{Deck, Gallery, tests::cards};
    use crate::manager::EntryId;
    use crate::sis::{SisInventory, tests::sis};
    use simulator::accessory::AccessoryDb;
    use simulator::card::{CardDb, LeaderId};
    use simulator::sis::SisDb;

    struct Fixture {
        cards: CardDb,
        acc_db: AccessoryDb,
        sis_db: SisDb,
        deck: Deck,
        accessories: Accessories,
        inventory: SisInventory,
    }

    fn fixture() -> Fixture {
        Fixture {
            cards: cards(),
            acc_db: accessories(),
            sis_db: sis(),
            deck: Deck::new("d".to_owned(), Gallery::default()),
            accessories: Accessories::new("a".to_owned()),
            inventory: SisInventory::new("s".to_owned()),
        }
    }

    impl Fixture {
        fn ctx(&self) -> Context<'_> {
            Context {
                deck: &self.deck,
                accessories: &self.accessories,
                inventory: &self.inventory,
                cards: &self.cards,
                acc_db: &self.acc_db,
                sis_db: &self.sis_db,
            }
        }

        fn card(&mut self, idolized: bool, slots: u8) -> EntryId {
            self.deck.add(&self.cards, 1, idolized, 1, slots, 100).unwrap().0
        }

        fn owns(&mut self, sis_id: u16) -> EntryId {
            self.inventory.add(&self.sis_db, sis_id).unwrap().0
        }

        fn acc(&mut self, acc_id: u16) -> EntryId {
            self.accessories.add(&self.acc_db, acc_id, 1).unwrap().0
        }
    }

    #[test]
    fn check_add_card_to_lanes() {
        let mut f = fixture();
        let (first, second) = (f.card(true, 8), f.card(true, 8));
        let mut team = Team::default();

        team.equip_card(0, first, f.ctx()).unwrap();
        assert_eq!(team.equip_card(1, first, f.ctx()), Err(Problem::InUse));
        team.equip_card(1, second, f.ctx()).unwrap();
        assert_eq!(team.lanes().count(), 2);
    }

    #[test]
    fn check_a_card_to_lanes_uniqueness() {
        let mut f = fixture();
        let (a, b) = (f.card(true, 8), f.card(true, 8));
        let (first, second) = (f.owns(7), f.owns(7));
        let mut team = Team::default();
        team.equip_card(0, a, f.ctx()).unwrap();
        team.equip_card(1, b, f.ctx()).unwrap();

        team.equip_sis(0, first, f.ctx()).unwrap();
        assert_eq!(team.equip_sis(1, first, f.ctx()), Err(Problem::InUse));
        assert_eq!(team.equip_sis(0, second, f.ctx()), Err(Problem::Duplicate));
        team.equip_sis(1, second, f.ctx()).unwrap();
    }

    #[test]
    fn check_capacity() {
        let mut f = fixture();
        let small = f.card(false, 4);
        let (big, kiss) = (f.owns(9), f.owns(7));
        let mut team = Team::default();
        team.equip_card(0, small, f.ctx()).unwrap();

        team.equip_sis(0, big, f.ctx()).unwrap();
        assert_eq!(team.slot(0).unwrap().slots_used(f.ctx()), 4);
        assert_eq!(team.equip_sis(0, kiss, f.ctx()), Err(Problem::NoRoom));

        team.unequip_sis(0, big).unwrap();
        team.equip_sis(0, kiss, f.ctx()).unwrap();
        assert_eq!(team.slot(0).unwrap().slots_used(f.ctx()), 1);
    }

    #[test]
    fn check_accessory() {
        let mut f = fixture();
        let (a, b) = (f.card(true, 8), f.card(true, 8));
        let (loose, locked) = (f.acc(7), f.acc(9));
        let mut team = Team::default();
        team.equip_card(0, a, f.ctx()).unwrap();
        team.equip_card(1, b, f.ctx()).unwrap();

        team.equip_accessory(0, loose, f.ctx()).unwrap();
        assert_eq!(team.equip_accessory(1, loose, f.ctx()), Err(Problem::InUse));
        assert_eq!(team.equip_accessory(1, locked, f.ctx()), Err(Problem::Restricted));
    }

    #[test]
    fn check_guest_is_a_leader_skill() {
        let f = fixture();
        let mut team = Team::default();
        assert_eq!(team.guest(), None);

        let last = LeaderId(u16::try_from(f.cards.leaders.len() - 1).unwrap());
        team.set_guest(last, f.ctx()).unwrap();
        assert_eq!(team.guest(), Some(last));

        let past = LeaderId(u16::try_from(f.cards.leaders.len()).unwrap());
        assert_eq!(team.set_guest(past, f.ctx()), Err(Problem::Unknown));
        assert_eq!(team.guest(), Some(last), "rejected guest");

        team.clear_guest();
        assert_eq!(team.guest(), None);
    }

    #[test]
    fn check_no_leader_skill_guest() {
        let f = fixture();
        let mut team = Team::default();
        team.set_guest(LeaderId(0), f.ctx()).unwrap();
        assert_eq!(team.guest(), Some(LeaderId(0)));
    }

    #[test]
    fn check_cards_goes_to_lane() {
        let mut f = fixture();
        let (a, b) = (f.card(true, 8), f.card(true, 8));
        let (acc, kiss) = (f.acc(7), f.owns(7));
        let mut team = Team::default();
        team.equip_card(0, a, f.ctx()).unwrap();
        team.equip_accessory(0, acc, f.ctx()).unwrap();
        team.equip_sis(0, kiss, f.ctx()).unwrap();

        team.equip_card(0, b, f.ctx()).unwrap();
        let slot = team.slot(0).unwrap();
        assert_eq!(slot.accessory(), None);
        assert_eq!(slot.sis().count(), 0);

        team.equip_accessory(0, acc, f.ctx()).unwrap();
        team.equip_sis(0, kiss, f.ctx()).unwrap();
    }
}
