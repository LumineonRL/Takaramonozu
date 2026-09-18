//! A slot for each lane + a guest

use simulator::accessory::AccessoryDb;
use simulator::card::{Card, CardDb, LeaderId};
use simulator::sis::{Sis, SisDb, SisEffect};

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
    /// Past lane 9.
    NoLane,
    /// A lane with no card
    Empty,
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

impl<'a> Context<'a> {
    fn equipped_by(self, entry: EntryId) -> Option<(u16, &'a Card)> {
        let card_id = self.deck.get(entry)?.card_id();
        let card = self.cards.cards.get(usize::from(self.cards.index(card_id)?.0))?;
        Some((card_id, card))
    }

    fn accessory_equips(self, acc: EntryId, card_id: u16, equipped_by: &Card) -> Option<bool> {
        let owned = self.accessories.get(acc)?;
        let meta = self.acc_db.meta.get(usize::from(self.acc_db.index(owned.acc_id())?.0))?;
        Some(meta.equips(owned.skill_level(), card_id, equipped_by.character))
    }

    /// # Errors
    /// Equipping a SIS that it shouldn't be able to do so I guess
    /// it would need to be something outside of `sis.rkyv`.
    fn sis_equips(self, sis: Sis, equipped_by: &Card) -> Result<bool, Problem> {
        if matches!(sis.effect, SisEffect::Charm | SisEffect::Heal) {
            return Ok(sis.attribute == equipped_by.attribute);
        }
        match sis.restriction {
            None => Ok(true),
            Some(t) => {
                let set = self.sis_db.targets.set(t).ok_or(Problem::Unknown)?;
                Ok(set.contains(equipped_by.character))
            }
        }
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

    #[must_use]
    pub fn room(self, ctx: Context) -> u8 {
        self.capacity(ctx).saturating_sub(self.slots_used(ctx))
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

    fn occupied(&self, lane: usize) -> Result<TeamSlot, Problem> {
        self.slots.get(lane).copied().ok_or(Problem::NoLane)?.ok_or(Problem::Empty)
    }

    /// # Errors
    /// Not a valid lane, a card not in the deck, or already in another lane.
    pub fn can_equip_card(&self, lane: usize, card: EntryId, ctx: Context) -> Result<(), Problem> {
        self.slots.get(lane).ok_or(Problem::NoLane)?;
        if ctx.deck.get(card).is_none() {
            return Err(Problem::Unknown);
        }
        if self.lanes().any(|(l, s)| l != lane && s.card == card) {
            return Err(Problem::InUse);
        }
        Ok(())
    }

    /// # Errors
    /// Not a lane, an empty one, an unowned accessory, one already worn in
    ///*another lane, you get the point.
    pub fn can_equip_accessory(
        &self,
        lane: usize,
        acc: EntryId,
        ctx: Context,
    ) -> Result<(), Problem> {
        let slot = self.occupied(lane)?;
        let equipped_by = ctx.equipped_by(slot.card).ok_or(Problem::Unknown)?;
        self.accessory_fits(lane, acc, ctx, equipped_by)
    }

    fn accessory_fits(
        &self,
        lane: usize,
        acc: EntryId,
        ctx: Context,
        equipped_by: (u16, &Card),
    ) -> Result<(), Problem> {
        if self.lanes().any(|(l, s)| l != lane && s.accessory == Some(acc)) {
            return Err(Problem::InUse);
        }
        if ctx
            .accessory_equips(acc, equipped_by.0, equipped_by.1)
            .ok_or(Problem::Unknown)?
        {
            Ok(())
        } else {
            Err(Problem::Restricted)
        }
    }

    /// # Errors
    /// Not a lane, an empty one, an unowned SIS, a copy already equipped
    /// elsewhere, a second of the same SIS on one card, one this card may not
    /// wear, or no capacity left. Yeah SIS has a lot of rules.
    pub fn can_equip_sis(&self, lane: usize, sis: EntryId, ctx: Context) -> Result<(), Problem> {
        let slot = self.occupied(lane)?;
        let (_, equipped_by) = ctx.equipped_by(slot.card).ok_or(Problem::Unknown)?;
        self.sis_fits(slot, sis, ctx, equipped_by, slot.room(ctx))
    }

    fn sis_fits(
        &self,
        slot: TeamSlot,
        sis: EntryId,
        ctx: Context,
        equipped_by: &Card,
        room: u8,
    ) -> Result<(), Problem> {
        let added = ctx.inventory.sis(sis, ctx.sis_db).ok_or(Problem::Unknown)?;
        if self.lanes().any(|(_, s)| s.sis().any(|held| held == sis)) {
            return Err(Problem::InUse);
        }
        let kind = ctx.inventory.get(sis).map(SisEntry::sis_id);
        if slot.sis().any(|held| ctx.inventory.get(held).map(SisEntry::sis_id) == kind) {
            return Err(Problem::Duplicate);
        }
        if !ctx.sis_equips(added, equipped_by)? {
            return Err(Problem::Restricted);
        }
        if added.slots > room {
            return Err(Problem::NoRoom);
        }
        Ok(())
    }

    /// # Errors
    /// As [`Team::can_equip_card`].
    pub fn equip_card(&mut self, lane: usize, card: EntryId, ctx: Context) -> Result<(), Problem> {
        self.can_equip_card(lane, card, ctx)?;
        *self.at(lane)? = Some(TeamSlot::new(card));
        Ok(())
    }

    /// # Errors
    /// As [`Team::can_equip_accessory`].
    pub fn equip_accessory(
        &mut self,
        lane: usize,
        acc: EntryId,
        ctx: Context,
    ) -> Result<(), Problem> {
        self.can_equip_accessory(lane, acc, ctx)?;
        let mut slot = self.occupied(lane)?;
        slot.accessory = Some(acc);
        *self.at(lane)? = Some(slot);
        Ok(())
    }

    /// # Errors
    /// As [`Team::can_equip_sis`].
    pub fn equip_sis(&mut self, lane: usize, sis: EntryId, ctx: Context) -> Result<(), Problem> {
        self.can_equip_sis(lane, sis, ctx)?;
        let mut slot = self.occupied(lane)?;
        *slot.sis.iter_mut().find(|s| s.is_none()).ok_or(Problem::NoRoom)? = Some(sis);
        *self.at(lane)? = Some(slot);
        Ok(())
    }

    pub fn card_choices<'t>(
        &'t self,
        lane: usize,
        ctx: Context<'t>,
    ) -> impl Iterator<Item = EntryId> + 't {
        ctx.deck
            .entries()
            .filter(move |&(id, _)| self.can_equip_card(lane, id, ctx).is_ok())
            .map(|(id, _)| id)
    }

    pub fn accessory_choices<'t>(
        &'t self,
        lane: usize,
        ctx: Context<'t>,
    ) -> impl Iterator<Item = EntryId> + 't {
        let equipped_by = self.occupied(lane).ok().and_then(|s| ctx.equipped_by(s.card));
        ctx.accessories.entries().filter_map(move |(id, _)| {
            self.accessory_fits(lane, id, ctx, equipped_by?).ok().map(|()| id)
        })
    }

    pub fn sis_choices<'t>(
        &'t self,
        lane: usize,
        ctx: Context<'t>,
    ) -> impl Iterator<Item = EntryId> + 't {
        let lane = self.occupied(lane).ok().and_then(|s| {
            let (_, equipped_by) = ctx.equipped_by(s.card)?;
            Some((s, equipped_by, s.room(ctx)))
        });
        ctx.inventory.entries().filter_map(move |(id, _)| {
            let (slot, equipped_by, room) = lane?;
            self.sis_fits(slot, id, ctx, equipped_by, room).ok().map(|()| id)
        })
    }

    /// # Errors
    /// Not a lane, an empty one, or a SIS this card doesn't have equipped..
    pub fn unequip_sis(&mut self, lane: usize, sis: EntryId) -> Result<(), Problem> {
        let mut slot = self.occupied(lane)?;
        *slot.sis.iter_mut().find(|s| **s == Some(sis)).ok_or(Problem::Unknown)? = None;
        *self.at(lane)? = Some(slot);
        Ok(())
    }

    /// # Errors
    /// Not a lane, an empty one, or a card without an acc.
    pub fn unequip_accessory(&mut self, lane: usize) -> Result<(), Problem> {
        let mut slot = self.occupied(lane)?;
        slot.accessory.take().ok_or(Problem::Unknown)?;
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
    use super::{Context, LANES, Problem, Team};
    use crate::accessory::{Accessories, tests::accessories};
    use crate::card::{Deck, Gallery, tests::cards};
    use crate::manager::{EntryId, Manager};
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

        // Re-equipping what this lane already wears is a no-op, not `InUse`.
        team.equip_accessory(0, loose, f.ctx()).unwrap();
        assert_eq!(team.slot(0).unwrap().accessory(), Some(loose));
    }

    #[test]
    fn check_unequip_accessory() {
        let mut f = fixture();
        let (a, b) = (f.card(true, 8), f.card(true, 8));
        let (loose, kiss) = (f.acc(7), f.owns(7));
        let mut team = Team::default();
        team.equip_card(0, a, f.ctx()).unwrap();
        team.equip_card(1, b, f.ctx()).unwrap();
        team.equip_accessory(0, loose, f.ctx()).unwrap();
        team.equip_sis(0, kiss, f.ctx()).unwrap();

        team.unequip_accessory(0).unwrap();
        let slot = team.slot(0).unwrap();
        assert_eq!(slot.accessory(), None);
        assert_eq!(slot.sis().count(), 1, "card and its SIS stay");

        assert_eq!(team.unequip_accessory(0), Err(Problem::Unknown), "nothing equipped");
        team.equip_accessory(1, loose, f.ctx()).unwrap();
    }

    #[test]
    fn check_sis_equip_restriction() {
        let mut f = fixture();
        let card = f.card(true, 8);
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();

        let (mine, theirs) = (f.owns(11), f.owns(13));
        team.equip_sis(0, mine, f.ctx()).unwrap();
        assert_eq!(team.equip_sis(0, theirs, f.ctx()), Err(Problem::Restricted));

        // Charm/Heal's restriction is an attribute. Like everything else is a character set.
        let (smile, cool) = (f.owns(15), f.owns(17));
        assert_eq!(team.equip_sis(0, cool, f.ctx()), Err(Problem::Restricted));
        team.equip_sis(0, smile, f.ctx()).unwrap();
        assert_eq!(team.slot(0).unwrap().sis().count(), 2);
    }

    #[test]
    fn check_equipment_character_mask() {
        let mut f = fixture();
        let card = f.card(true, 8);
        let owned: Vec<EntryId> = [7, 9, 11, 13, 15, 17].map(|id| f.owns(id)).into();
        let mut team = Team::default();

        // Nothing can be equipped in a lane with no card,
        assert_eq!(team.sis_choices(0, f.ctx()).count(), 0);
        assert_eq!(team.accessory_choices(0, f.ctx()).count(), 0);

        team.equip_card(0, card, f.ctx()).unwrap();
        let legal: Vec<EntryId> = team.sis_choices(0, f.ctx()).collect();
        assert_eq!(legal.len(), 4, "13 is another char's, 17 another attribute's");
        assert!(!legal.contains(&owned[3]));
        assert!(!legal.contains(&owned[5]));

        for (id, _) in f.inventory.entries() {
            assert_eq!(team.can_equip_sis(0, id, f.ctx()).is_ok(), legal.contains(&id));
        }
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
