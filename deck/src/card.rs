//! Every card a user owns, their skill levels, their memory gallery,
//! their level cap (if an idolized UR)

use serde::{Deserialize, Serialize};

use simulator::scalar::{Attribute, Form};

/// This essentially determines the search space I really should make this
/// number smaller but it needs to be at least somewhat realistic
pub const MAX_CARDS: usize = 300;

/// Real talk I have no idea what the Gallery actually capped out at
/// by EoS. TODO: What is the max possible gallery.
pub const MAX_GALLERY: u32 = 50_000;

const fn default_skill_level() -> u8 {
    1
}

/// You may be wondering where every non-UR and non-idolized level caps are.
/// Simple - They don't matter :) Only idolized URs can have cap increases.
const fn default_idolized_ur_level_cap() -> u16 {
    100
}

/// If you were not around when this mechanic was added and have no idea what it does,
/// it's just a flat, additive base-stat bonus that gets applied to every card you own.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Gallery([u32; 3]);

impl Gallery {
    #[must_use]
    pub fn new(values: [u32; 3]) -> Option<Self> {
        values.iter().all(|&v| v <= MAX_GALLERY).then_some(Self(values))
    }

    #[inline]
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`There are 3 attributes.")]
    pub const fn get(self, a: Attribute) -> u32 {
        self.0[a as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct EntryId(u16);

impl EntryId {
    /// # Panics
    /// Past 65,536 entries. A `Deck` holds at most `MAX_CARDS`.
    #[must_use]
    #[expect(clippy::expect_used, reason = "capacity bound, documented above")]
    pub fn from_index(i: usize) -> Self {
        Self(u16::try_from(i).expect("deck index fits u16"))
    }

    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeckEntry {
    card_id: u16,
    idolized: bool,
    #[serde(default = "default_skill_level")]
    skill_level: u8,
    sis_slots: u8,
    /// Technically I'm giving *everything* a level of 100 here even like, SRs
    /// that cap at 60/80, but again... the level does not matter, only
    /// base stats + idolization status.
    #[serde(default = "default_idolized_ur_level_cap")]
    cap_level: u16,
}

impl DeckEntry {
    #[cfg_attr(not(test), expect(dead_code, reason = "TODO"))]
    pub(crate) const fn new(
        card_id: u16,
        idolized: bool,
        skill_level: u8,
        sis_slots: u8,
        cap_level: u16,
    ) -> Self {
        Self { card_id, idolized, skill_level, sis_slots, cap_level }
    }

    #[must_use]
    pub const fn card_id(self) -> u16 {
        self.card_id
    }

    #[must_use]
    pub const fn form(self) -> Form {
        if self.idolized {
            Form::Idolized
        } else {
            Form::Unidolized
        }
    }

    #[must_use]
    pub const fn skill_level(self) -> u8 {
        self.skill_level
    }

    #[must_use]
    pub const fn sis_slots(self) -> u8 {
        self.sis_slots
    }

    #[must_use]
    pub const fn cap_level(self) -> u16 {
        self.cap_level
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Deck {
    pub name: String,
    #[serde(default)]
    pub gallery: Gallery,
    entries: Vec<Option<DeckEntry>>,
}

impl Deck {
    #[must_use]
    pub const fn new(name: String, gallery: Gallery) -> Self {
        Self { name, gallery, entries: Vec::new() }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn is_full(&self) -> bool {
        self.len() >= MAX_CARDS
    }

    #[must_use]
    pub fn get(&self, id: EntryId) -> Option<DeckEntry> {
        self.entries.get(id.index()).copied().flatten()
    }

    pub fn insert(&mut self, entry: DeckEntry) -> Option<EntryId> {
        if self.is_full() {
            return None;
        }
        let index = if let Some((i, hole)) =
            self.entries.iter_mut().enumerate().find(|(_, e)| e.is_none())
        {
            *hole = Some(entry);
            i
        } else {
            let i = self.entries.len();
            self.entries.push(Some(entry));
            i
        };
        Some(EntryId::from_index(index))
    }

    pub fn remove(&mut self, id: EntryId) -> Option<DeckEntry> {
        self.entries.get_mut(id.index())?.take()
    }

    pub fn entries(&self) -> impl Iterator<Item = (EntryId, DeckEntry)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.map(|entry| (EntryId::from_index(i), entry)))
    }
}

#[cfg(test)]
mod tests {
    use super::{Deck, DeckEntry, EntryId, Gallery, MAX_CARDS, MAX_GALLERY};
    use simulator::scalar::{Attribute, Form};

    fn entry(card_id: u16) -> DeckEntry {
        DeckEntry::new(card_id, true, 8, 8, 500)
    }

    fn deck() -> Deck {
        Deck::new("test".to_owned(), Gallery::default())
    }

    #[test]
    fn check_the_gallery_capt() {
        assert_eq!(
            Gallery::new([MAX_GALLERY; 3]).map(|g| g.get(Attribute::Cool)),
            Some(MAX_GALLERY)
        );
        assert_eq!(Gallery::new([0, MAX_GALLERY + 1, 0]), None);
    }

    #[test]
    fn check_an_id_is_reused_lowest_hole_first() {
        let mut d = deck();
        let ids: Vec<EntryId> = (0..3).map(|i| d.insert(entry(i)).unwrap()).collect();
        assert_eq!(d.remove(ids[2]), Some(entry(2)));
        assert_eq!(d.remove(ids[0]), Some(entry(0)));
        assert_eq!(d.len(), 1);

        assert_eq!(d.insert(entry(10)), Some(ids[0]));
        assert_eq!(d.insert(entry(11)), Some(ids[2]));
        assert_eq!(d.len(), 3);
        assert_eq!(d.get(ids[0]).map(DeckEntry::card_id), Some(10));
    }

    #[test]
    fn check_a_removed_id_doesnt_shift() {
        let mut d = deck();
        let a = d.insert(entry(1)).unwrap();
        let b = d.insert(entry(2)).unwrap();
        d.remove(a);

        assert_eq!(d.get(a), None);
        assert_eq!(d.get(b).map(DeckEntry::card_id), Some(2));
        assert_eq!(d.remove(a), None);
        assert_eq!(d.entries().count(), 1);
    }

    #[test]
    fn check_the_cap_counts_live_entries() {
        let mut d = deck();
        for i in 0..MAX_CARDS {
            assert!(d.insert(entry(u16::try_from(i).unwrap())).is_some());
        }
        assert!(d.is_full());
        assert_eq!(d.insert(entry(0)), None);

        d.remove(EntryId::from_index(7));
        assert!(!d.is_full());
        assert_eq!(d.insert(entry(999)), Some(EntryId::from_index(7)));
    }

    #[test]
    fn check_a_removal_round_trip() {
        let mut d = deck();
        d.gallery = Gallery::new([1, 2, 3]).unwrap();
        let a = d.insert(entry(1)).unwrap();
        let b = d.insert(entry(2)).unwrap();
        d.remove(a);

        let json = serde_json::to_string(&d).unwrap();
        let back: Deck = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.get(a), None);
        assert_eq!(back.get(b).map(DeckEntry::card_id), Some(2));
    }

    #[test]
    fn check_defaults() {
        let json = r#"{"name":"d","entries":[{"card_id":66,"idolized":false,"sis_slots":2}]}"#;
        let d: Deck = serde_json::from_str(json).unwrap();

        let e = d.get(EntryId::from_index(0)).unwrap();
        assert_eq!(e.skill_level(), 1);
        assert_eq!(e.cap_level(), 100);
        assert_eq!(e.form(), Form::Unidolized);
        assert_eq!(d.gallery, Gallery::default());
    }
}
