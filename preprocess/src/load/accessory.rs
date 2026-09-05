//! Mostly the same as card. The big differences that matter:
//! - No `LeaderSkill`
//! - All accs have a skill so no placeholder.

use std::collections::HashMap;

use simulator::accessory::{
    Accessory, AccessoryArena, AccessoryDb, AccessoryMeta, AccessorySkillId, STAT_ROWS, StatArena,
    StatsId,
};
use simulator::scalar::{Centi, Millis, Pct, Roster, Stats};
use simulator::skill::{SkillEffect, TRIGGER_ROWS, TargetId, Targets};

use crate::load::convert::{self, At, ErrorKind, LoadError};
use crate::load::raw::{RawAccessories, RawAccessory, RawTargetMap};

/// Used when iterating over the nested skill values. base skill level is 1..=8 here
/// and there can be more than 8 entries (which are used by amp)
const ROWS: usize = TRIGGER_ROWS as usize;
const LAST_BASE: usize = ROWS - 1;

type StatTable = [Stats; STAT_ROWS as usize];

#[derive(PartialEq, Eq, Hash)]
struct Key {
    effect: SkillEffect,
    target: Option<TargetId>,
    threshold: u8,
    chances: [Pct; ROWS],
    payload: Vec<(Centi, Millis)>,
}

/// # Errors
/// An unexpected or broken field
pub fn build(
    raw: &RawAccessories,
    map: &RawTargetMap,
    roster: &Roster,
) -> Result<AccessoryDb, LoadError> {
    let mut skills = AccessoryArena::default();
    let mut stats = StatArena::default();
    let mut targets = Targets::default();
    let mut seen: HashMap<Key, AccessorySkillId> = HashMap::new();
    let mut tables: HashMap<StatTable, StatsId> = HashMap::new();
    let mut accessories = Vec::with_capacity(raw.len());
    let mut meta = Vec::with_capacity(raw.len());

    for (&acc_id, a) in raw {
        let key = self::key(acc_id, a, &mut targets, map, roster)?;
        let skill = if let Some(&id) = seen.get(&key) {
            id
        } else {
            let id = skills.push(key.effect, key.target, key.threshold, &key.chances, &key.payload);
            seen.insert(key, id);
            id
        };
        let table = stat_table(acc_id, a)?;
        let id = *tables.entry(table).or_insert_with(|| stats.push(&table));
        accessories.push(Accessory { skill, stats: id });
        meta.push(AccessoryMeta {
            acc_id,
            card_id: card_id(acc_id, a)?,
            name: a.name.as_str().into(),
        });
    }

    Ok(AccessoryDb { accessories, meta, skills, stats, targets })
}

fn key(
    acc_id: u16,
    a: &RawAccessory,
    targets: &mut Targets,
    map: &RawTargetMap,
    roster: &Roster,
) -> Result<Key, LoadError> {
    let (trig, eff) = (&a.skill.trigger, &a.skill.effect);
    let rows = a.stats.len();
    if rows < ROWS
        || trig.chances.len() != rows
        || trig.values.len() != rows
        || eff.durations.len() != rows
        || eff.values.len() != rows
    {
        return Err(ErrorKind::Invariant).at(acc_id, "rows");
    }
    if !flat_past_base(&trig.chances) {
        return Err(ErrorKind::Invariant).at(acc_id, "trigger.chances");
    }

    // Only relevant for the Spark encore counter. This value never
    // changes, so can be a little lazy with extracting it.
    let threshold = trig.values.first().copied().unwrap_or_default();
    if trig.values.iter().any(|v| *v != threshold) {
        return Err(ErrorKind::Invariant).at(acc_id, "trigger.values");
    }

    let effect = convert::effect(&eff.effect).at(acc_id, "type")?;
    let target = a
        .skill
        .target()
        .map(|t| targets.intern(t, || convert::accessory_target(t, map, roster)))
        .transpose()
        .at(acc_id, "target")?;

    // Accessory chances are already percents unlike what's in the card files.
    let mut chances = [0; ROWS];
    for (out, &c) in chances.iter_mut().zip(&trig.chances) {
        *out = c;
    }

    let mut payload = Vec::with_capacity(rows);
    for (&v, &d) in eff.values.iter().zip(&eff.durations) {
        payload.push((
            value(effect, threshold, v).at(acc_id, "values")?,
            convert::millis(d).at(acc_id, "durations")?,
        ));
    }

    while payload.len() > 1 && payload.last() == payload.iter().nth_back(1) {
        payload.pop();
    }

    Ok(Key { effect, target, threshold, chances, payload })
}

/// Shared with cards.
fn value(effect: SkillEffect, threshold: u8, raw: u32) -> Result<Centi, ErrorKind> {
    let factor: u32 = match effect {
        // Cards write a percentage as a fraction where accessories write it as
        // percent, so these two are the file's only pre-scaled values.
        SkillEffect::AppealBoost | SkillEffect::SkillRateUp => 1,
        // Encore's stored `1` is a placeholder
        SkillEffect::Encore => 0,
        // A Spark's effective value is `value x threshold`.
        SkillEffect::Spark => u32::from(threshold).saturating_mul(100),
        // Counts and flat amounts,
        SkillEffect::Scorer
        | SkillEffect::Healer
        | SkillEffect::PerfectLock
        | SkillEffect::Amplify
        | SkillEffect::TotalTrick
        | SkillEffect::PerfectScoreUp
        | SkillEffect::ComboBonusUp
        | SkillEffect::Sync => 100,
    };
    raw.checked_mul(factor).ok_or(ErrorKind::OutOfRange(f64::from(raw)))
}

fn stat_table(acc_id: u16, a: &RawAccessory) -> Result<StatTable, LoadError> {
    if !flat_past_base(&a.stats) {
        return Err(ErrorKind::Invariant).at(acc_id, "stats");
    }
    let mut table = [Stats::default(); STAT_ROWS as usize];
    for (out, &row) in table.iter_mut().zip(&a.stats) {
        *out = Stats(row);
    }
    Ok(table)
}

fn card_id(acc_id: u16, a: &RawAccessory) -> Result<Option<u16>, LoadError> {
    let Some(s) = a.card_id() else {
        return Ok(None);
    };
    let id = s
        .parse::<u16>()
        .ok()
        .ok_or_else(|| ErrorKind::Unknown(s.into()))
        .at(acc_id, "card_id");
    Ok(Some(id?))
}

/// For skill levels past the 8 base levels
fn flat_past_base<T>(col: &[T]) -> bool
where
    T: PartialEq,
{
    let (Some(base), Some(tail)) = (col.get(LAST_BASE), col.get(ROWS..)) else {
        return false;
    };
    tail.iter().all(|v| *v == *base)
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

    use simulator::accessory::{AccessoryRows, StatRows};

    use super::*;
    use crate::load::raw::{self, RawCard};

    fn read(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        std::fs::read(format!("{path}{name}")).expect("data file")
    }

    fn accessories() -> RawAccessories {
        raw::parse_accessories(&read("accessories.json")).expect("schema")
    }

    fn map() -> RawTargetMap {
        raw::parse_target_map(&read("year_group_target.json")).expect("schema")
    }

    fn roster() -> Roster {
        let cards: Vec<RawCard> = raw::parse(&read("cards.json")).expect("schema");
        Roster::build(cards.iter().map(|c| c.character.as_str()))
    }

    fn built() -> AccessoryDb {
        build(&accessories(), &map(), &roster())
            .unwrap_or_else(|e| panic!("Accessory file is valid: {e}"))
    }

    /// [`AccessoryDb`] has no `Debug`, so `expect_err` is unavailable.
    fn rejected(raw: &RawAccessories, why: &str) -> LoadError {
        match build(raw, &map(), &roster()) {
            Err(e) => e,
            Ok(_) => panic!("built anyway: {why}"),
        }
    }

    #[test]
    fn check_tables_populate() {
        let db = built();
        assert_eq!(db.len(), 549);
        assert_eq!(db.meta.len(), 549);
        assert_eq!(db.skills.defs().len(), 155);
        assert_eq!(db.stats.rows().len(), 165 * STAT_ROWS as usize);
        assert_eq!(db.targets.len(), 22);
        assert_eq!(db.meta.iter().filter(|m| m.card_id.is_some()).count(), 484);
        assert!(db.meta.windows(2).all(|w| w[0].acc_id < w[1].acc_id));
    }

    #[test]
    fn check_tables_trimmed_to_cap() {
        let db = built();
        let raw = accessories();
        let rows: usize = db
            .skills
            .defs()
            .iter()
            .map(|d| {
                assert!(d.rows >= 1 && d.rows <= 16, "{} rows", d.rows);
                d.rows as usize
            })
            .sum();
        assert_eq!(rows, 2040, "8758 stored rows fold to 2040");
        // Only encores should have constant payloads
        let flat = db
            .accessories
            .iter()
            .filter(|a| db.skills.def(a.skill).expect("in range").rows == 1)
            .count();
        assert_eq!(flat, 71);

        for (idx, (_, a)) in db.accessories.iter().zip(&raw) {
            let def = db.skills.def(idx.skill).expect("in range");
            let last = a.skill.effect.values.len();
            #[expect(clippy::cast_possible_truncation, reason = "16 rows at most")]
            let (value, _) = db.skills.payload(def, last as u8);
            let (trimmed, _) = db.skills.payload(def, 24);
            assert_eq!(value, trimmed, "a trimmed table must read its last row back");
        }
    }

    /// The card <-> acc equip limitations
    #[test]
    fn check_the_units_pair_correctly() {
        let db = built();
        let raw = accessories();
        let mut seen = [0_u32; 4];
        for (a, (id, r)) in db.accessories.iter().zip(&raw) {
            let def = db.skills.def(a.skill).expect("in range");
            let (got, _) = db.skills.payload(def, 1);
            let first = r.skill.effect.values.first().copied().expect("16 rows");
            let (expected, case) = match def.effect {
                SkillEffect::AppealBoost | SkillEffect::SkillRateUp => (first, 0),
                // The placeholder `1` normalizes away.
                SkillEffect::Encore => (0, 1),
                SkillEffect::Spark => (first * u32::from(def.threshold) * 100, 2),
                SkillEffect::Scorer
                | SkillEffect::Healer
                | SkillEffect::PerfectLock
                | SkillEffect::Amplify
                | SkillEffect::TotalTrick
                | SkillEffect::PerfectScoreUp
                | SkillEffect::ComboBonusUp
                | SkillEffect::Sync => (first * 100, 3),
            };
            assert_eq!(got, expected, "accessory {id}");
            seen[case] += 1;
        }
        assert_eq!(seen, [48 + 85, 71, 23, 549 - 48 - 85 - 71 - 23]);

        let guren = db.index(217).expect("acc 217");
        let a = &db.accessories[guren.0 as usize];
        let def = db.skills.def(a.skill).expect("in range");
        assert_eq!(def.effect, SkillEffect::Spark);
        assert_eq!(def.threshold, 5);
        assert_eq!(db.skills.payload(def, 1), (5000, 1400));
        assert_eq!(db.skills.chance(def, 1), 9);
    }

    #[test]
    fn check_every_accessory_round_trips_through_its_index() {
        let db = built();
        for (&acc_id, r) in &accessories() {
            let idx = db.index(acc_id).expect("accessory present");
            let meta = &db.meta[idx.0 as usize];
            assert_eq!(&*meta.name, r.name);
            assert_eq!(meta.card_id.is_some(), r.card_id().is_some());

            let a = &db.accessories[idx.0 as usize];
            let def = db.skills.def(a.skill).expect("in range");
            assert_eq!(def.target.and_then(|t| db.targets.name(t)), r.skill.target());
            for level in 1..=ROWS {
                #[expect(clippy::cast_possible_truncation, reason = "level <= 8")]
                let level = level as u8;
                let row = usize::from(level).saturating_sub(1);
                assert_eq!(db.skills.chance(def, level), r.skill.trigger.chances[row]);
                assert_eq!(db.stats.get(a.stats, level), Stats(r.stats[row]));
            }
        }
    }

    #[test]
    fn check_reject_out_of_range_stats() {
        let mut raw = accessories();
        raw.get_mut(&217).expect("acc 217").stats[ROWS][0] += 1;
        let err = rejected(&raw, "a stat table past row 8 is never read");
        assert_matches!(err, LoadError { card_id: Some(217), field: "stats", .. });
    }

    #[test]
    fn check_a_threshold_column_that_varies_is_rejected() {
        let mut raw = accessories();
        raw.get_mut(&217).expect("acc 217").skill.trigger.values[ROWS] += 1;
        let err = rejected(&raw, "only the first threshold row is stored");
        assert_matches!(err, LoadError { field: "trigger.values", .. });
    }

    #[test]
    fn check_a_ragged_column_is_rejected() {
        let mut raw = accessories();
        raw.get_mut(&217).expect("acc 217").skill.trigger.chances.truncate(LAST_BASE);
        let err = rejected(&raw, "a short column cannot fill the trigger half");
        assert_matches!(err, LoadError { field: "rows", kind: ErrorKind::Invariant, .. });
    }
}
