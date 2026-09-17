//! Every card a user owns, their skill levels, their memory gallery,
//! their level cap (if an idolized UR)

use core::num::NonZeroU8;

use serde::de::{Deserializer, Error as _};
use serde::{Deserialize, Serialize};

use simulator::card::CardDb;
use simulator::scalar::{Attribute, Form, Rarity};

use crate::level_cap::{self, LevelCaps};
use crate::manager::{Entry, EntryId, Flag, Manager, Repairs, Slots};
use crate::skill_level;

/// This essentially determines the search space I really should make this
/// number smaller but it needs to be at least somewhat realistic
pub const MAX_CARDS: usize = 300;

/// Real talk I have no idea what the Gallery actually capped out at
/// by EoS. TODO: What is the max possible gallery.
#[expect(
    clippy::doc_markdown,
    reason = "It was about 5x more effort to write
this exception than to just put backticks around EoS like clippy wanted, but
I need to stand my ground. EoS does not need backticks. It is not an item."
)]
pub const MAX_GALLERY: u32 = 50_000;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repair {
    Idolized,
    CapLevel,
    SkillLevel,
}

impl Flag for Repair {
    fn bit(self) -> u8 {
        match self {
            Self::Idolized => 1,
            Self::CapLevel => 2,
            Self::SkillLevel => 4,
        }
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

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DeckEntry {
    card_id: u16,
    idolized: bool,
    #[serde(default = "skill_level::default")]
    skill_level: u8,
    sis_slots: u8,
    /// Technically I'm giving *everything* a level of 100 here even like, SRs
    /// that cap at 60/80, but again... the level does not matter, only
    /// base stats + idolization status.
    #[serde(default = "default_idolized_ur_level_cap")]
    cap_level: u16,
    #[serde(skip)]
    hp: Option<NonZeroU8>,
}

impl DeckEntry {
    pub(crate) const fn new(
        card_id: u16,
        idolized: bool,
        skill_level: u8,
        sis_slots: u8,
        cap_level: u16,
    ) -> Self {
        Self {
            card_id,
            idolized,
            skill_level,
            sis_slots,
            cap_level,
            hp: None,
        }
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

    #[must_use]
    pub const fn hp(self) -> Option<NonZeroU8> {
        self.hp
    }
}

impl Entry for DeckEntry {
    type Db = CardDb;
    type Problem = Problem;

    const CAPACITY: usize = MAX_CARDS;
    const FULL: Problem = Problem::Full;

    fn vet(mut self, db: &CardDb) -> Result<(Self, Repairs), Vec<Problem>> {
        let i = db.index(self.card_id).map(|idx| usize::from(idx.0));
        let Some((card, meta)) = i.and_then(|i| db.cards.get(i).zip(db.meta.get(i))) else {
            return Err(vec![Problem::UnknownCard]);
        };
        let mut repairs = Repairs::default();
        let mut problems = Vec::new();

        if !self.idolized && meta.is_promo {
            self.idolized = true;
            repairs.insert(Repair::Idolized);
        }

        let skilled = card.skill.is_some();
        if !skilled && self.skill_level != skill_level::MIN_LEVEL {
            self.skill_level = skill_level::MIN_LEVEL;
            repairs.insert(Repair::SkillLevel);
        }
        if skilled && !skill_level::in_range(self.skill_level) {
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
        self.hp = NonZeroU8::new(per_form(meta.hp, form));
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

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Deck {
    pub name: String,
    #[serde(default)]
    pub gallery: Gallery,
    entries: Slots<DeckEntry>,
}

impl Manager for Deck {
    type Entry = DeckEntry;

    fn slots(&self) -> &Slots<DeckEntry> {
        &self.entries
    }

    fn slots_mut(&mut self) -> &mut Slots<DeckEntry> {
        &mut self.entries
    }
}

impl Deck {
    #[must_use]
    pub const fn new(name: String, gallery: Gallery) -> Self {
        Self { name, gallery, entries: Slots::empty() }
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
        let entry = DeckEntry::new(card_id, idolized, skill_level, sis_slots, cap_level);
        self.entries.add(cards, entry)
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
pub(crate) mod tests {
    use super::{Deck, DeckEntry, Gallery, MAX_CARDS, MAX_GALLERY, Problem, Repair};
    use crate::level_cap::{self, tests::corpus, tests::meta};
    use crate::manager::{EntryId, Manager, Repaired, Repairs};
    use crate::skill_level;
    use core::num::{NonZeroU8, NonZeroU16};
    use simulator::card::{Card, CardDb, LeaderId, LeaderSkill, LeaderTargets};
    use simulator::scalar::{Attribute, CharacterId, Form, Rarity, Roster, Stats};
    use simulator::skill::{SkillArena, SkillId, Targets};

    fn entry(card_id: u16) -> DeckEntry {
        DeckEntry::new(card_id, true, 8, 8, 500)
    }

    pub(crate) fn cards() -> CardDb {
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
        let ids: Vec<EntryId> = (0..3).map(|i| d.slots_mut().insert(entry(i)).unwrap()).collect();
        assert_eq!(d.remove(ids[2]), Some(entry(2)));
        assert_eq!(d.remove(ids[0]), Some(entry(0)));
        assert_eq!(d.len(), 1);

        assert_eq!(d.slots_mut().insert(entry(10)), Some(ids[0]));
        assert_eq!(d.slots_mut().insert(entry(11)), Some(ids[2]));
        assert_eq!(d.len(), 3);
        assert_eq!(d.get(ids[0]).map(DeckEntry::card_id), Some(10));
    }

    #[test]
    fn check_a_removed_id_doesnt_shift() {
        let mut d = deck();
        let a = d.slots_mut().insert(entry(1)).unwrap();
        let b = d.slots_mut().insert(entry(2)).unwrap();
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
            assert!(d.slots_mut().insert(entry(u16::try_from(i).unwrap())).is_some());
        }
        assert!(d.is_full());
        assert_eq!(d.slots_mut().insert(entry(0)), None);

        d.remove(EntryId::from_index(7));
        assert!(!d.is_full());
        assert_eq!(d.slots_mut().insert(entry(999)), Some(EntryId::from_index(7)));
    }

    #[test]
    fn check_a_removal_round_trip() {
        let (db, mut d) = (cards(), deck());
        d.gallery = Gallery::new([1, 2, 3]).unwrap();
        let a = d.slots_mut().insert(entry(1)).unwrap();
        let b = d.slots_mut().insert(entry(2)).unwrap();
        d.remove(a);

        let json = d.to_json().unwrap();
        assert!(!String::from_utf8(json.clone()).unwrap().contains("null"));

        let back = Deck::parse(&json, &db).unwrap();
        assert_eq!(back.value.gallery, d.gallery);
        assert_eq!(back.value.len(), 1);
        assert_ne!(b, a);
        assert_eq!(back.value.get(a).map(DeckEntry::card_id), Some(2));
        assert_eq!(back.value.get(b), None);
    }

    #[test]
    fn check_load_vets() {
        let (db, mut d) = (cards(), deck());
        d.slots_mut().insert(entry(1)).unwrap();
        d.slots_mut().insert(DeckEntry::new(3, true, 8, 8, 500)).unwrap();
        d.slots_mut()
            .insert(DeckEntry::new(2, false, 5, 8, level_cap::MAX_LEVEL))
            .unwrap();

        let loaded = Deck::parse(&d.to_json().unwrap(), &db).unwrap();
        assert_eq!(loaded.rejected.len(), 1);
        let bad = &loaded.rejected[0];
        assert_eq!((bad.row, bad.entry.card_id()), (1, 3));
        assert_eq!(bad.problems, vec![Problem::UnknownCard]);

        // The survivor behind the dropped row moved up into its id.
        assert_eq!(loaded.value.len(), 2);
        let id = EntryId::from_index(1);
        assert_eq!(loaded.value.get(id).map(DeckEntry::card_id), Some(2));
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

        let a = d.slots_mut().insert(entry(1)).unwrap();
        assert_eq!(d.base_stats(a, &db, &caps), Some([14_471, 12_452, 12_765]));
        let b = d.slots_mut().insert(entry(2)).unwrap();
        assert_eq!(d.base_stats(b, &db, &caps), Some([10421, 8402, 8715]));
    }

    #[test]
    fn check_only_idolized_urs_get_bonus() {
        let (mut db, caps) = (cards(), corpus());
        let mut d = deck();

        let un = d.slots_mut().insert(DeckEntry::new(1, false, 8, 8, 500)).unwrap();
        assert_eq!(d.base_stats(un, &db, &caps), Some([1, 2, 3]));

        let ur = d.slots_mut().insert(entry(1)).unwrap();
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
        let bad = d.add(&db, 1, true, skill_level::MAX_LEVEL + 1, 9, level_cap::MAX_LEVEL + 1);
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
            d.slots_mut().insert(entry(1)).unwrap();
        }
        assert_eq!(d.add(&db, 1, true, 99, 99, 999), Err(vec![Problem::Full]));
    }

    #[test]
    fn check_hp_derived() {
        assert_eq!(size_of::<DeckEntry>(), 8);
    }

    #[test]
    fn check_hp_follows_the_idolization() {
        let (db, mut d) = (cards(), deck());

        // Not pre-idolized
        let (un, _) = d.add(&db, 1, false, 8, 4, 100).unwrap();
        let (id, _) = d.add(&db, 1, true, 8, 8, 500).unwrap();
        assert_eq!(d.get(un).and_then(DeckEntry::hp).map(NonZeroU8::get), Some(5));
        assert_eq!(d.get(id).and_then(DeckEntry::hp).map(NonZeroU8::get), Some(6));

        // Pre-idolized
        let (promo, repairs) = d.add(&db, 2, false, 1, 8, 100).unwrap();
        assert!(repairs.contains(Repair::Idolized));
        assert_eq!(d.get(promo).and_then(DeckEntry::hp).map(NonZeroU8::get), Some(6));
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
