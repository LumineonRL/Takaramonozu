//! Every card a user owns, their skill levels, their memory gallery,
//! their level cap (if an idolized UR)

use core::fmt;
use std::io;
use std::path::Path;

use serde::de::{Deserializer, Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Serialize, Serializer};

use simulator::card::CardDb;
use simulator::scalar::{Attribute, Form, Rarity};

use crate::level_cap::{self, LevelCaps};

/// This essentially determines the search space I really should make this
/// number smaller but it needs to be at least somewhat realistic
pub const MAX_CARDS: usize = 300;
const MAX_ROWS: usize = u16::MAX as usize;

/// Real talk I have no idea what the Gallery actually capped out at
/// by EoS. TODO: What is the max possible gallery.
#[expect(
    clippy::doc_markdown,
    reason = "It was about 5x more effort to write
this exception than to just put backticks around EoS like clippy wanted, but
I need to stand my ground. EoS does not need backticks. It is not an item."
)]
pub const MAX_GALLERY: u32 = 50_000;
pub const MIN_SKILL_LEVEL: u8 = 1;
pub const MAX_SKILL_LEVEL: u8 = 8;

const fn default_skill_level() -> u8 {
    MIN_SKILL_LEVEL
}

/// You may be wondering where every non-UR and non-idolized level caps are.
/// Simple - They don't matter :) Only idolized URs can have cap increases.
const fn default_idolized_ur_level_cap() -> u16 {
    level_cap::MIN_LEVEL
}

/// I hate the name `Problem` but suck at naming error variants so shrug.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Problem {
    UnknownCard,
    SkillLevel,
    SisSlots,
    CapLevel,
    Full,
}

/// I hate this name too. This really just like, defaults if the
/// user enters an invalid value. Like, if a user tries to add an
/// unidolized promo card, it "repairs" it by making it idolized.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repair {
    Idolized,
    CapLevel,
    SkillLevel,
}

impl Repair {
    const fn bit(self) -> u8 {
        match self {
            Self::Idolized => 1,
            Self::CapLevel => 2,
            Self::SkillLevel => 4,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct Repairs(u8);

impl Repairs {
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, repair: Repair) -> bool {
        self.0 & repair.bit() != 0
    }

    fn insert(&mut self, repair: Repair) {
        self.0 |= repair.bit();
    }
}

#[expect(clippy::indexing_slicing, reason = "`Form` has two variants.")]
const fn per_form(pair: [u8; 2], form: Form) -> u8 {
    pair[form as usize]
}

/// If you were not around when this mechanic was added and have no idea what it does,
/// it's just a flat, additive base-stat bonus that gets applied to every card you own.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, Serialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct Gallery([u32; 3]);

/// I love needing to roll my own Deserialize traits yayyy
impl<'de> Deserialize<'de> for Gallery {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = <[u32; 3]>::deserialize(deserializer)?;
        Self::new(values)
            .ok_or_else(|| D::Error::custom(format!("gallery bonus past {MAX_GALLERY}")))
    }
}

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

    /// # Errors
    /// Every rule the entry breaks.
    pub fn vet(mut self, cards: &CardDb) -> Result<(Self, Repairs), Vec<Problem>> {
        let i = cards.index(self.card_id).map(|idx| usize::from(idx.0));
        let Some((card, meta)) = i.and_then(|i| cards.cards.get(i).zip(cards.meta.get(i))) else {
            return Err(vec![Problem::UnknownCard]);
        };
        let mut repairs = Repairs::default();
        let mut problems = Vec::new();

        if !self.idolized && meta.is_promo {
            self.idolized = true;
            repairs.insert(Repair::Idolized);
        }

        let skilled = card.skill.is_some();
        if !skilled && self.skill_level != MIN_SKILL_LEVEL {
            self.skill_level = MIN_SKILL_LEVEL;
            repairs.insert(Repair::SkillLevel);
        }
        if skilled && !(MIN_SKILL_LEVEL..=MAX_SKILL_LEVEL).contains(&self.skill_level) {
            problems.push(Problem::SkillLevel);
        }

        let expandable = meta.rarity == Rarity::UR && self.idolized;
        if !expandable && self.cap_level != level_cap::MIN_LEVEL {
            self.cap_level = level_cap::MIN_LEVEL;
            repairs.insert(Repair::CapLevel);
        }
        if expandable && !level_cap::in_table(self.cap_level) {
            problems.push(Problem::CapLevel);
        }

        let form = self.form();
        let slots = per_form(meta.sis_base, form)..=per_form(meta.sis_max, form);
        if !slots.contains(&self.sis_slots) {
            problems.push(Problem::SisSlots);
        }

        if problems.is_empty() {
            Ok((self, repairs))
        } else {
            Err(problems)
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rejected {
    pub row: usize,
    pub entry: DeckEntry,
    pub problems: Vec<Problem>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Repaired {
    pub row: usize,
    pub id: EntryId,
    pub repairs: Repairs,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loaded {
    pub deck: Deck,
    pub repaired: Vec<Repaired>,
    pub rejected: Vec<Rejected>,
}

fn compact<S>(entries: &[Option<DeckEntry>], s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.collect_seq(entries.iter().flatten())
}

fn rows<'de, D>(d: D) -> Result<Vec<Option<DeckEntry>>, D::Error>
where
    D: Deserializer<'de>,
{
    struct Rows;

    impl<'de> Visitor<'de> for Rows {
        type Value = Vec<Option<DeckEntry>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(formatter, "at most {MAX_ROWS} deck entries")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_CARDS));
            while let Some(entry) = seq.next_element()? {
                if out.len() == MAX_ROWS {
                    return Err(A::Error::custom(format!("past {MAX_ROWS} deck entries")));
                }
                out.push(entry);
            }
            Ok(out)
        }
    }

    d.deserialize_seq(Rows)
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Deck {
    pub name: String,
    #[serde(default)]
    pub gallery: Gallery,
    #[serde(serialize_with = "compact", deserialize_with = "rows")]
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

    /// # Errors
    /// A rule break or an already full deck.
    pub fn add(
        &mut self,
        cards: &CardDb,
        card_id: u16,
        idolized: bool,
        skill_level: u8,
        sis_slots: u8,
        cap_level: u16,
    ) -> Result<(EntryId, Repairs), Vec<Problem>> {
        if self.is_full() {
            return Err(vec![Problem::Full]);
        }
        let entry = DeckEntry::new(card_id, idolized, skill_level, sis_slots, cap_level);
        let (entry, repairs) = entry.vet(cards)?;
        let id = self.insert(entry).ok_or_else(|| vec![Problem::Full])?;
        Ok((id, repairs))
    }

    /// # Errors
    /// An I/O issue, not a deck. A file that *is* a deck but
    /// carries but has bad rows.
    pub fn load(path: &Path, cards: &CardDb) -> io::Result<Loaded> {
        Self::parse(&std::fs::read(path)?, cards)
    }

    /// # Errors
    /// As [`Deck::load`], minus the I/O.
    pub fn parse(bytes: &[u8], cards: &CardDb) -> io::Result<Loaded> {
        let deck: Self = serde_json::from_slice(bytes)?;
        Ok(deck.vet(cards))
    }

    #[must_use]
    pub fn vet(self, cards: &CardDb) -> Loaded {
        let mut deck = Self::new(self.name, self.gallery);
        let (mut repaired, mut rejected) = (Vec::new(), Vec::new());

        for (row, entry) in self.entries.into_iter().enumerate() {
            let Some(entry) = entry else { continue };
            match entry.vet(cards) {
                // Capacity is the file's problem
                Ok((entry, repairs)) => match deck.insert(entry) {
                    Some(id) if !repairs.is_empty() => {
                        repaired.push(Repaired { row, id, repairs });
                    }
                    Some(_) => {}
                    None => {
                        rejected.push(Rejected { row, entry, problems: vec![Problem::Full] });
                    }
                },
                Err(problems) => rejected.push(Rejected { row, entry, problems }),
            }
        }
        Loaded { deck, repaired, rejected }
    }

    /// # Errors
    /// I/O skill issue.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.to_json()?)
    }

    /// # Errors
    /// Serialization, which a `Deck` shouldn't actually fail in the first place.
    pub fn to_json(&self) -> io::Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub(crate) fn insert(&mut self, entry: DeckEntry) -> Option<EntryId> {
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

    #[must_use]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "It's a u32 even the highest stat UR at level 500 with max gallery
        only hits like 70k."
    )]
    pub fn base_stats(&self, id: EntryId, cards: &CardDb, caps: &LevelCaps) -> Option<[u32; 3]> {
        let entry = self.get(id)?;
        let i = cards.index(entry.card_id())?.0 as usize;
        let (card, meta) = (cards.cards.get(i)?, cards.meta.get(i)?);
        let expandable = meta.rarity == Rarity::UR && entry.form() == Form::Idolized;
        let bonus = if expandable {
            caps.bonus(entry.cap_level(), meta)?
        } else {
            0
        };

        let base = card.stats(entry.form());
        let stat = |a: Attribute| u32::from(base.get(a)) + bonus + self.gallery.get(a);
        Some([stat(Attribute::Smile), stat(Attribute::Pure), stat(Attribute::Cool)])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Deck, DeckEntry, EntryId, Gallery, MAX_CARDS, MAX_GALLERY, MAX_SKILL_LEVEL,
        Problem, Repair, Repaired, Repairs,
    };
    use crate::level_cap::{self, tests::corpus, tests::meta};
    use core::num::NonZeroU16;
    use simulator::card::{Card, CardDb, LeaderId, LeaderSkill, LeaderTargets};
    use simulator::scalar::{Attribute, CharacterId, Form, Rarity, Roster, Stats};
    use simulator::skill::{SkillArena, SkillId, Targets};

    fn entry(card_id: u16) -> DeckEntry {
        DeckEntry::new(card_id, true, 8, 8, 500)
    }

    fn cards() -> CardDb {
        let card = Card {
            stats: [Stats([1, 2, 3]), Stats([1021, 2, 315])],
            skill: None,
            leader: LeaderId(0),
            character: CharacterId(0),
            attribute: Attribute::Smile,
        };
        let skilled = Card { skill: Some(SkillId(NonZeroU16::new(1).unwrap())), ..card };
        CardDb {
            cards: vec![skilled, card],
            meta: vec![meta(1, false), meta(2, true)],
            skills: SkillArena::default(),
            skill_targets: Targets::default(),
            leaders: vec![LeaderSkill {
                attribute: Attribute::Smile,
                secondary: None,
                value: 0,
                extra: None,
            }],
            leader_targets: LeaderTargets::default(),
            roster: Roster::build(["Sonoda Umi"]),
        }
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
        let (db, mut d) = (cards(), deck());
        d.gallery = Gallery::new([1, 2, 3]).unwrap();
        let a = d.insert(entry(1)).unwrap();
        let b = d.insert(entry(2)).unwrap();
        d.remove(a);

        let json = d.to_json().unwrap();
        assert!(!String::from_utf8(json.clone()).unwrap().contains("null"));

        let back = Deck::parse(&json, &db).unwrap();
        assert_eq!(back.deck.gallery, d.gallery);
        assert_eq!(back.deck.len(), 1);
        assert_ne!(b, a);
        assert_eq!(back.deck.get(a).map(DeckEntry::card_id), Some(2));
        assert_eq!(back.deck.get(b), None);
    }

    #[test]
    fn check_load_vets() {
        let (db, mut d) = (cards(), deck());
        d.insert(entry(1)).unwrap();
        d.insert(DeckEntry::new(3, true, 8, 8, 500)).unwrap();
        d.insert(DeckEntry::new(2, false, 5, 8, level_cap::MAX_LEVEL)).unwrap();

        let loaded = Deck::parse(&d.to_json().unwrap(), &db).unwrap();
        assert_eq!(loaded.rejected.len(), 1);
        let bad = &loaded.rejected[0];
        assert_eq!((bad.row, bad.entry.card_id()), (1, 3));
        assert_eq!(bad.problems, vec![Problem::UnknownCard]);

        // The survivor behind the dropped row moved up into its id.
        assert_eq!(loaded.deck.len(), 2);
        let id = EntryId::from_index(1);
        assert_eq!(loaded.deck.get(id).map(DeckEntry::card_id), Some(2));
        assert_eq!(
            loaded.repaired,
            vec![Repaired {
                row: 2,
                id,
                repairs: {
                    let mut r = Repairs::default();
                    r.insert(Repair::Idolized);
                    r.insert(Repair::SkillLevel);
                    r
                }
            }]
        );
    }

    #[test]
    fn check_gallery_cap_rejection() {
        let json = format!(r#"{{"name":"d","gallery":[0,{},0],"entries":[]}}"#, MAX_GALLERY + 1);
        match Deck::parse(json.as_bytes(), &cards()) {
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
            Ok(_) => panic!(
                "`parse` accepted a gallery serde should not have built you absolute buffoon"
            ),
        }
    }

    #[test]
    fn check_base_plus_gallert() {
        let (db, caps) = (cards(), corpus());
        let mut d = deck();
        d.gallery = Gallery::new([1000, 0, 0]).unwrap();

        let a = d.insert(entry(1)).unwrap();
        assert_eq!(d.base_stats(a, &db, &caps), Some([14_471, 12_452, 12_765]));
        let b = d.insert(entry(2)).unwrap();
        assert_eq!(d.base_stats(b, &db, &caps), Some([10421, 8402, 8715]));
    }

    #[test]
    fn check_only_idolized_urs_get_bonus() {
        let (mut db, caps) = (cards(), corpus());
        let mut d = deck();

        let un = d.insert(DeckEntry::new(1, false, 8, 8, 500)).unwrap();
        assert_eq!(d.base_stats(un, &db, &caps), Some([1, 2, 3]));

        let ur = d.insert(entry(1)).unwrap();
        db.meta[0].rarity = Rarity::SSR;
        assert_eq!(d.base_stats(ur, &db, &caps), Some([1021, 2, 315]));
    }

    #[test]
    fn check_an_unknown_card_breaks() {
        let (db, mut d) = (cards(), deck());
        assert_eq!(d.add(&db, 3, true, 99, 99, 999), Err(vec![Problem::UnknownCard]));
        assert!(d.is_empty());
    }

    #[test]
    fn check_every_bad_field() {
        let (db, mut d) = (cards(), deck());
        let bad = d.add(&db, 1, true, MAX_SKILL_LEVEL + 1, 9, level_cap::MAX_LEVEL + 1);
        assert_eq!(bad, Err(vec![Problem::SkillLevel, Problem::CapLevel, Problem::SisSlots]));
        assert!(d.is_empty());
    }

    #[test]
    fn check_the_cap_bounds() {
        let (db, caps, mut d) = (cards(), corpus(), deck());
        let over = d.add(&db, 1, true, 8, 8, level_cap::MAX_LEVEL + 1);
        assert_eq!(over, Err(vec![Problem::CapLevel]));

        let (id, repairs) = d.add(&db, 1, true, 8, 8, level_cap::MAX_LEVEL).unwrap();
        assert_eq!(repairs, Repairs::default());
        assert!(d.base_stats(id, &db, &caps).is_some());
    }

    #[test]
    fn check_a_pre_idolized_skillless_card_is_repaired() {
        let (db, mut d) = (cards(), deck());
        let (id, repairs) = d.add(&db, 2, false, 5, 8, level_cap::MAX_LEVEL).unwrap();

        assert!(repairs.contains(Repair::Idolized));
        assert!(repairs.contains(Repair::SkillLevel));
        assert!(!repairs.contains(Repair::CapLevel));

        let e = d.get(id).unwrap();
        assert_eq!(e.form(), Form::Idolized);
        assert_eq!(e.skill_level(), 1);
        assert_eq!(e.cap_level(), level_cap::MAX_LEVEL);
    }

    #[test]
    fn check_a_cap_is_useless_for_non_urs() {
        let (db, mut d) = (cards(), deck());
        let (id, repairs) = d.add(&db, 1, false, 8, 4, level_cap::MAX_LEVEL).unwrap();

        assert!(repairs.contains(Repair::CapLevel));
        assert!(!repairs.contains(Repair::Idolized));
        assert_eq!(d.get(id).unwrap().cap_level(), level_cap::MIN_LEVEL);
    }

    #[test]
    fn check_a_full_deck_says_so() {
        let (db, mut d) = (cards(), deck());
        for _ in 0..MAX_CARDS {
            d.insert(entry(1)).unwrap();
        }
        assert_eq!(d.add(&db, 1, true, 99, 99, 999), Err(vec![Problem::Full]));
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
