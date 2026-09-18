//! Assemble the cards from skills and leader skills

use simulator::card::{Card, CardDb, CardMeta};
use simulator::scalar::{Roster, Stats};

use crate::load::convert::{self, At, Culprit, ErrorKind, LoadError};
use crate::load::raw::{RawCard, RawStats, RawTargetMap, RawUniqueLeader};
use crate::load::{leader, skill};

// unidolized min, unidolized max, idolized min, idolized max.
// regular first, promos second.
const SIS: [[[u8; 4]; 2]; 5] = [
    [[0, 0, 1, 1], [0, 0, 1, 1]], // N
    [[1, 1, 1, 2], [1, 1, 1, 1]], // R
    [[2, 2, 2, 4], [2, 2, 2, 2]], // SR
    [[3, 3, 3, 6], [2, 2, 2, 2]], // SSR
    [[4, 4, 4, 8], [2, 2, 2, 2]], // UR
];

/// # Errors
/// Any field that does not convert
///
/// # Panics
/// Past 65535 cards, which is what a [`CardIdx`](simulator::card::CardIdx) holds.
#[expect(clippy::panic_in_result_fn, reason = "capacity bound, documented above")]
pub fn build(
    raw: &[RawCard],
    unique_leaders: &[RawUniqueLeader],
    leader_map: &RawTargetMap,
    year_group: &RawTargetMap,
) -> Result<CardDb, LoadError> {
    let roster = Roster::build(raw.iter().map(|c| c.character.as_str()));
    let mut order: Vec<&RawCard> = raw.iter().collect();
    order.sort_unstable_by_key(|c| c.card_id);
    assert!(u16::try_from(order.len()).is_ok(), "{} cards do not fit a CardIdx", order.len());

    let mut skills = skill::Table::default();
    let mut leaders = leader::Table::seeded(unique_leaders, leader_map, &roster)?;
    let mut cards = Vec::with_capacity(order.len());
    let mut meta: Vec<CardMeta> = Vec::with_capacity(order.len());
    for c in order {
        if meta.last().is_some_and(|m| m.card_id == c.card_id) {
            return Err(ErrorKind::Invariant).at(Culprit::Card(c.card_id), "card_id");
        }
        cards.push(Card {
            stats: [stats(&c.stats.unidolized), stats(&c.stats.idolized)],
            skill: skills.insert(c.card_id, &c.skill, year_group, &roster)?,
            leader: leaders.resolve(c.card_id, &c.leader_skill, leader_map, &roster)?,
            #[expect(clippy::expect_used, reason = "the roster is built from these characters")]
            character: roster.id(&c.character).expect("names of the characters"),
            attribute: convert::attribute(&c.attribute)
                .at(Culprit::Card(c.card_id), "attribute")?,
        });
        meta.push(self::meta(c)?);
    }

    Ok(CardDb {
        cards,
        meta,
        skills: skills.arena,
        skill_targets: skills.targets,
        leaders: leaders.by_id,
        leader_targets: leaders.targets,
        roster,
    })
}

fn stats(s: &RawStats) -> Stats {
    Stats([s.smile, s.pure, s.cool])
}

#[expect(clippy::indexing_slicing, reason = "`Rarity` has exactly the 5 rows used")]
fn meta(c: &RawCard) -> Result<CardMeta, LoadError> {
    let rarity = convert::rarity(&c.rarity).at(Culprit::Card(c.card_id), "rarity")?;
    let (u, i) = (&c.stats.unidolized, &c.stats.idolized);

    // Pre-idolized non-promo is a fix of `is_promo`. The tldr for this is that I scraped the
    // promo status from schooido.lu. However, it lists anything pre-idolized as a promo, which
    // held at the beginning of the game's life but not later on. So "pre-idolized non-promo" is
    // a mark I manually added to denote what's *actually* a promo vs what's a card that just came
    // pre-idolized.
    if c.is_preidolized_non_promo && !c.is_promo {
        return Err(ErrorKind::Invariant).at(Culprit::Card(c.card_id), "is_preidolized_non_promo");
    }
    let promo = c.is_promo && !c.is_preidolized_non_promo;
    if [u.sis_base, u.sis_max, i.sis_base, i.sis_max] != SIS[rarity as usize][usize::from(promo)] {
        return Err(ErrorKind::Invariant).at(Culprit::Card(c.card_id), "sis_max");
    }
    // promos are pre-idolized.
    if promo && (stats(u) != stats(i) || u.hp != i.hp) {
        return Err(ErrorKind::Invariant).at(Culprit::Card(c.card_id), "stats");
    }

    Ok(CardMeta {
        card_id: c.card_id,
        rarity,
        sis_base: [u.sis_base, i.sis_base],
        sis_max: [u.sis_max, i.sis_max],
        hp: [u.hp, i.hp],
        display_name: c.display_name.as_str().into(),
        is_promo: c.is_promo,
        is_preidolized_non_promo: c.is_preidolized_non_promo,
    })
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

    use simulator::skill::SkillRows;

    use super::*;
    use simulator::card::LeaderId;
    use simulator::scalar::Rarity;

    use crate::load::raw;

    fn read(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        std::fs::read(format!("{path}{name}")).expect("data file")
    }

    fn cards() -> Vec<RawCard> {
        raw::parse(&read("cards.json")).expect("schema")
    }

    fn map() -> RawTargetMap {
        raw::parse_target_map(&read("additional_leader_skill_map.json")).expect("schema")
    }

    fn year_group() -> RawTargetMap {
        raw::parse_target_map(&read("year_group_target.json")).expect("schema")
    }

    fn unique() -> Vec<RawUniqueLeader> {
        raw::parse_unique_leaders(&read("unique_leader_skills.json")).expect("schema")
    }

    fn built() -> CardDb {
        build(&cards(), &unique(), &map(), &year_group())
            .unwrap_or_else(|e| panic!("corpus is valid: {e}"))
    }

    /// [`CardDb`] has no `Debug`, so `expect_err` is unavailable.
    fn rejected(cards: &[RawCard], why: &str) -> LoadError {
        match build(cards, &unique(), &map(), &year_group()) {
            Err(e) => e,
            Ok(_) => panic!("built anyway: {why}"),
        }
    }

    #[test]
    fn check_corpus_builds_with_the_expected_tables() {
        let db = built();
        assert_eq!(db.len(), 3826);
        assert_eq!(db.meta.len(), 3826);
        assert_eq!(db.skills.defs().len(), 2461);
        assert_eq!(db.skill_targets.len(), 14);
        assert_eq!(db.leaders.len(), 313);
        assert_eq!(db.leader_targets.len(), 74);
        assert_eq!(db.roster.len(), 99);
    }

    #[test]
    fn check_card_idx_is_sorted() {
        let db = built();
        assert!(db.meta.windows(2).all(|w| w[0].card_id < w[1].card_id));

        let mut shuffled = cards();
        shuffled.reverse();
        let other =
            build(&shuffled, &unique(), &map(), &year_group()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(db.cards, other.cards);
        assert_eq!(db.meta, other.meta);
        assert_eq!(db.skill_targets, other.skill_targets);
        assert_eq!(db.leader_targets, other.leader_targets);
    }

    /// Both halves of a card have to stay on the same row, and a saved deck's
    /// `card_id` has to land back where it started
    #[test]
    fn check_every_card_round_trips_through_its_index() {
        let db = built();
        for c in &cards() {
            let idx = db.index(c.card_id).expect("card present");
            let meta = &db.meta[idx.0 as usize];
            assert_eq!(meta.card_id, c.card_id);
            assert_eq!(&*meta.display_name, c.display_name);
            assert_eq!(meta.hp, [c.stats.unidolized.hp, c.stats.idolized.hp]);
            let card = &db.cards[idx.0 as usize];
            assert_eq!(card.stats[1], stats(&c.stats.idolized));
            assert_eq!(db.roster.name(card.character), Some(c.character.as_str()));
            assert_eq!(card.skill.is_some(), c.skill.is_present());
            assert_eq!(card.leader == LeaderId(0), c.leader_skill.leader_attribute.is_none());
        }
        assert_eq!(db.index(0), None);
        assert_eq!(db.index(3912), None);
    }

    #[test]
    fn check_promo_classes() {
        let db = built();
        let promos = db.meta.iter().filter(|m| m.is_promo && !m.is_preidolized_non_promo);
        assert_eq!(promos.clone().count(), 540);
        assert_eq!(db.meta.iter().filter(|m| m.is_preidolized_non_promo).count(), 1268);
        // Every actual promo of SR and above shares one layout, so SIS slots
        // alone can't tell a promo SR from a promo SSR.
        assert!(
            promos
                .filter(|m| m.rarity >= Rarity::SR)
                .all(|m| m.sis_base == [2, 2] && m.sis_max == [2, 2])
        );
    }

    #[test]
    fn check_a_broken_sis_layout_is_rejected() {
        let mut cards = cards();
        let c = cards.iter_mut().find(|c| c.card_id == 66).expect("card 66");
        c.stats.idolized.sis_max += 1;
        let err = rejected(&cards, "the layout no longer matches the card's rarity");
        assert_matches!(err, LoadError { culprit: Culprit::Card(66), field: "sis_max", .. });
    }

    #[test]
    fn check_an_impossible_promo_flag_pair_is_rejected() {
        let mut cards = cards();
        let c = cards.iter_mut().find(|c| c.card_id == 66).expect("card 66");
        (c.is_promo, c.is_preidolized_non_promo) = (false, true);
        let err = rejected(&cards, "pre-idolized non-promo implies promo");
        assert_matches!(err, LoadError { field: "is_preidolized_non_promo", .. });
    }

    #[test]
    fn check_a_promo_whose_idolizations_disagree_is_rejected() {
        let mut cards = cards();
        let c = cards
            .iter_mut()
            .find(|c| c.is_promo && !c.is_preidolized_non_promo)
            .expect("the corpus has real promos");
        c.stats.idolized.smile += 530;
        let id = c.card_id;
        let err = rejected(&cards, "a promo can not idolize again");
        assert_matches!(err, LoadError { culprit: Culprit::Card(got), field: "stats", .. } if got == id);
    }
}
