//! Assemble the SIS table

use simulator::scalar::{Attribute, Milli, Roster};
use simulator::sis::{Sis, SisDb, SisEffect, SisMeta};
use simulator::skill::{TargetId, Targets};

use crate::load::convert::{self, At, Culprit, ErrorKind, LoadError};
use crate::load::raw::{RawSis, RawTargetMap};

const YEARS: [&str; 3] = ["1st years", "2nd years", "3rd years"];

/// Charms in sis.json are 1.5, but they're actually a 2.5x multiplier.
/// Figure it makes sense to just add in that 1.0 it's missing here.
/// Multiplied by 1000 because floats are cringe (I just like to keep clippy happy).
const CHARM_BASE: Milli = 1000;

/// # Errors
/// Any field that does not convert
///
/// # Panics
/// Past 65535 SIS, which is what a [`SisIdx`](simulator::sis::SisIdx) holds.
#[expect(clippy::panic_in_result_fn, reason = "capacity bound, documented above")]
pub fn build(
    raw: &[RawSis],
    years: &RawTargetMap,
    groups: &RawTargetMap,
    roster: &Roster,
) -> Result<SisDb, LoadError> {
    let mut order: Vec<&RawSis> = raw.iter().collect();
    order.sort_unstable_by_key(|s| s.id);
    assert!(u16::try_from(order.len()).is_ok(), "{} SIS do not fit a SisIdx", order.len());

    let mut targets = Targets::default();
    let mut sis = Vec::with_capacity(order.len());
    let mut meta: Vec<SisMeta> = Vec::with_capacity(order.len());
    for s in order {
        if meta.last().is_some_and(|m| m.sis_id == s.id) {
            return Err(ErrorKind::Invariant).at(Culprit::Sis(s.id), "id");
        }
        let effect = convert::sis_effect(&s.effect).at(Culprit::Sis(s.id), "effect")?;
        let attribute = convert::attribute(&s.attribute).at(Culprit::Sis(s.id), "attribute")?;
        if (s.target == "all") != (effect == SisEffect::AllPercent) {
            return Err(ErrorKind::Invariant).at(Culprit::Sis(s.id), "target");
        }
        if !(1..=6).contains(&s.slots) {
            return Err(ErrorKind::Invariant).at(Culprit::Sis(s.id), "slots");
        }
        let group = s
            .group()
            .map(|g| targets.intern(g, || convert::character_set(g, groups, roster)))
            .transpose()
            .at(Culprit::Sis(s.id), "group")?;

        sis.push(Sis::new(
            effect,
            attribute,
            s.slots,
            value(effect, s.value).at(Culprit::Sis(s.id), "value")?,
            restriction(s, effect, attribute, &mut targets, years, roster)?,
            group,
        ));
        meta.push(SisMeta { sis_id: s.id, name: s.name.as_str().into() });
    }

    Ok(SisDb { sis, meta, targets })
}

/// Everything in thousandths to keep it consistent. Needed bnecause
/// the all percents are 3 decimals places in sis.json.
fn value(effect: SisEffect, raw: f64) -> Result<Milli, ErrorKind> {
    let value = convert::milli(raw)?;
    match effect {
        SisEffect::Charm => value.checked_add(CHARM_BASE).ok_or(ErrorKind::OutOfRange(raw)),
        SisEffect::SelfFlat
        | SisEffect::SelfPercent
        | SisEffect::AllPercent
        | SisEffect::Heal
        | SisEffect::Trick => Ok(value),
    }
}

/// SIS equip restrictions are annoying because they can be a character, an
/// attribute, or a year-group and I don't really have a nice bitmask for those.
fn restriction(
    s: &RawSis,
    effect: SisEffect,
    attribute: Attribute,
    targets: &mut Targets,
    years: &RawTargetMap,
    roster: &Roster,
) -> Result<Option<TargetId>, LoadError> {
    let Some(name) = s.restriction() else {
        return Ok(None);
    };
    if let Ok(restricted) = convert::attribute(name) {
        let own = restricted == attribute && matches!(effect, SisEffect::Charm | SisEffect::Heal);
        return if own {
            Ok(None)
        } else {
            Err(ErrorKind::Invariant).at(Culprit::Sis(s.id), "equip_restriction")
        };
    }
    let id = if YEARS.contains(&name) {
        targets.intern(name, || convert::character_set(name, years, roster))
    } else {
        targets.intern(name, || convert::singleton(name, roster))
    };
    Ok(Some(id.at(Culprit::Sis(s.id), "equip_restriction")?))
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

    use simulator::scalar::CharacterSet;

    use super::*;
    use crate::load::raw::{self, RawCard};

    fn read(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        std::fs::read(format!("{path}{name}")).expect("data file")
    }

    fn sis() -> Vec<RawSis> {
        raw::parse_sis(&read("sis.json")).expect("schema")
    }

    fn years() -> RawTargetMap {
        raw::parse_target_map(&read("year_group_mapping.json")).expect("schema")
    }

    fn groups() -> RawTargetMap {
        raw::parse_target_map(&read("group_member_map.json")).expect("schema")
    }

    fn roster() -> Roster {
        let cards: Vec<RawCard> = raw::parse(&read("cards.json")).expect("schema");
        Roster::build(cards.iter().map(|c| c.character.as_str()))
    }

    fn built() -> SisDb {
        build(&sis(), &years(), &groups(), &roster())
            .unwrap_or_else(|e| panic!("the SIS file is valid: {e}"))
    }

    /// [`SisDb`] has no `Debug`, so `expect_err` is unavailable.
    fn rejected(raw: &[RawSis], why: &str) -> LoadError {
        match build(raw, &years(), &groups(), &roster()) {
            Err(e) => e,
            Ok(_) => panic!("built anyway: {why}"),
        }
    }

    #[test]
    fn check_tables_populate() {
        let db = built();
        assert_eq!(db.len(), 546);
        assert_eq!(db.meta.len(), 546);
        // 39 individual characters, 3 year groups, 4 full groups.
        assert_eq!(db.targets.len(), 46);
        assert!(db.meta.windows(2).all(|w| w[0].sis_id < w[1].sis_id));

        let mut shuffled = sis();
        shuffled.reverse();
        let other =
            build(&shuffled, &years(), &groups(), &roster()).unwrap_or_else(|e| panic!("{e}"));
        assert!(other.sis == db.sis && other.meta == db.meta, "the archive bakes in the order");
    }

    /// The four kinds of `equip_restriction`, of which only two are stored.
    #[test]
    fn check_the_restriction_splits_by_kind() {
        let db = built();
        let mut kinds = [0_u32; 4];
        for (s, raw) in db.sis.iter().zip(sis()) {
            let case = match raw.restriction() {
                None => 3,
                Some(name) if YEARS.contains(&name) => 1,
                Some(name) if convert::attribute(name).is_ok() => 2,
                Some(_) => 0,
            };
            kinds[case] += 1;
            // The attribute kind is the one that resolves to nothing stored.
            assert_eq!(s.restriction.is_some(), case != 2 && case != 3, "sis {}", raw.id);
            assert_eq!(matches!(s.effect, SisEffect::Charm | SisEffect::Heal), case == 2);
        }
        assert_eq!(kinds, [468, 27, 6, 45]);

        // year groups don't need to be from the same group. e.g. Aqours 2nd year
        // and Niji second year are both second year.
        let ring = db.sis[db.index(7).expect("Smile Ring [1st]").0 as usize];
        let set = db.targets.set(ring.restriction.expect("1st years")).expect("interned");
        assert_eq!(set.0.count_ones(), 19);
    }

    // Nonets require 9 different members from the same group.
    // I don't know why I'm spending so much time worrying about nonet it's terrible.
    #[test]
    fn check_the_nonet_groups_partition_their_half_of_the_roster() {
        let db = built();
        let nonets: Vec<TargetId> = db.sis.iter().filter_map(|s| s.group).collect();
        assert_eq!(nonets.len(), 24);

        let mut union = CharacterSet::default();
        let mut sizes: Vec<u32> = Vec::new();
        for name in ["μ's", "Aqours", "Nijigasaki", "Liella!"] {
            let id = db.targets.set(
                nonets
                    .iter()
                    .copied()
                    .find(|&t| db.targets.name(t) == Some(name))
                    .expect("group interned"),
            );
            let set = id.expect("in range");
            assert_eq!(union.0 & set.0, 0, "{name} overlaps another group");
            union.0 |= set.0;
            sizes.push(set.0.count_ones());
        }
        assert_eq!(sizes, [9, 9, 12, 9]);
        assert_eq!(union.0.count_ones(), 39);
    }

    #[test]
    fn check_every_sis_round_trips_through_its_index() {
        let db = built();
        for raw in &sis() {
            let idx = db.index(raw.id).expect("sis present");
            let meta = &db.meta[idx.0 as usize];
            assert_eq!(&*meta.name, raw.name);
            let s = db.sis[idx.0 as usize];
            assert_eq!(s.slots, raw.slots);
            assert_eq!(
                s.restriction.and_then(|t| db.targets.name(t)),
                raw.restriction().filter(|n| convert::attribute(n).is_err())
            );
            assert_eq!(s.group.and_then(|t| db.targets.name(t)), raw.group());
        }
        assert_eq!(db.index(0), None);
        assert_eq!(db.index(547), None);
    }

    #[test]
    fn check_the_value_column_scales_correctly() {
        let db = built();
        let value = |id: u16| db.sis[db.index(id).expect("present").0 as usize].value;
        assert_eq!(value(1), 200_000, "Smile Kiss, 200.0 flat");
        assert_eq!(value(25), 18, "Smile Aura, 1.8% as thousandths");
        assert_eq!(value(31), 2500, "Princess Charm, the file's 1.5 folded to 2.5x");
        assert_eq!(value(32), 480_000, "Princess Heal, 480 x the heal amount");
        assert_eq!(value(33), 330, "Princess Trick, +33%");
    }

    #[test]
    fn check_a_target_that_disagrees_with_its_effect_is_rejected() {
        let mut raw = sis();
        raw[0].target = "all".to_owned();
        let err = rejected(&raw, "`target` is only redundant while it agrees");
        assert_matches!(err, LoadError { culprit: Culprit::Sis(1), field: "target", .. });
    }

    #[test]
    fn check_an_attribute_restriction_on_the_wrong_effect_is_rejected() {
        let mut raw = sis();
        raw[0].equip_restriction = "Smile".to_owned();
        let err = rejected(&raw, "only a Charm or a Heal restricts by attribute");
        assert_matches!(
            err,
            LoadError { field: "equip_restriction", kind: ErrorKind::Invariant, .. }
        );
    }

    /// A dropped year key must fail as a year key rather than as a character.
    #[test]
    fn check_a_vanished_year_key_fails_where_it_is_understood() {
        let mut map = years();
        map.remove("1st years").expect("the key the 9 first-year SIS resolve against");
        let Err(err) = build(&sis(), &map, &groups(), &roster()) else {
            panic!("built anyway: the year map lost a key")
        };
        assert_matches!(err, LoadError { field: "equip_restriction", kind: ErrorKind::Unknown(s), .. }
            if &*s == "1st years");
    }
}
