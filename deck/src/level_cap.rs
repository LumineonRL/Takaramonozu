// transcription of `level_cap_bonuses.json`.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Deserialize;
use simulator::card::CardMeta;

pub const MIN_LEVEL: u16 = 100;
pub const MAX_LEVEL: u16 = 500;
const LEVELS: usize = 401;

#[must_use]
pub const fn in_table(cap_level: u16) -> bool {
    MIN_LEVEL <= cap_level && cap_level <= MAX_LEVEL
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    promo: BTreeMap<u16, u16>,
    non_promo: BTreeMap<u16, u16>,
}

pub struct LevelCaps {
    promo: [u16; LEVELS],
    non_promo: [u16; LEVELS],
}

impl LevelCaps {
    /// # Errors
    /// I/O issue, a schema change, or missing a value on [100, 500]
    pub fn load(path: &Path) -> io::Result<Self> {
        Self::parse(&std::fs::read(path)?)
    }

    /// # Errors
    /// As [`LevelCaps::load`], minus the I/O.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let raw: Raw = serde_json::from_slice(bytes)?;
        Ok(Self { promo: dense(&raw.promo)?, non_promo: dense(&raw.non_promo)? })
    }

    #[must_use]
    pub fn bonus(&self, cap_level: u16, meta: &CardMeta) -> Option<u32> {
        let column = if meta.is_promo && !meta.is_preidolized_non_promo {
            &self.promo
        } else {
            &self.non_promo
        };
        let i = usize::from(cap_level.saturating_sub(MIN_LEVEL));
        column.get(i).copied().map(u32::from)
    }
}

fn dense(rows: &BTreeMap<u16, u16>) -> io::Result<[u16; LEVELS]> {
    if rows.len() != LEVELS {
        return Err(not_dense());
    }
    let mut out = [0; LEVELS];
    for ((&level, &bonus), (want, slot)) in rows.iter().zip((MIN_LEVEL..=MAX_LEVEL).zip(&mut out)) {
        if level != want {
            return Err(not_dense());
        }
        *slot = bonus;
    }
    Ok(out)
}

fn not_dense() -> io::Error {
    let why = "missing a level on [100, 500]";
    io::Error::new(io::ErrorKind::InvalidData, why)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{LevelCaps, MAX_LEVEL, MIN_LEVEL};
    use simulator::card::CardMeta;
    use simulator::scalar::Rarity;
    use std::path::Path;

    pub(crate) fn corpus() -> LevelCaps {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/level_cap_bonuses.json");
        LevelCaps::load(Path::new(path)).expect("the level cap bonuses")
    }

    pub(crate) fn meta(card_id: u16, promo: bool) -> CardMeta {
        CardMeta {
            card_id,
            rarity: Rarity::UR,
            sis_base: [4, 4],
            sis_max: [4, 8],
            hp: [5, 6],
            display_name: "testy mctest".into(),
            is_promo: promo,
            is_preidolized_non_promo: false,
        }
    }

    #[test]
    fn check_the_shipped_table_is_the_one_documented() {
        let caps = corpus();
        let (promo, normal) = (meta(1, true), meta(2, false));

        assert_eq!(caps.bonus(MIN_LEVEL, &normal), Some(0));
        assert_eq!(caps.bonus(MAX_LEVEL, &promo), Some(8400));
        assert_eq!(caps.bonus(MAX_LEVEL, &normal), Some(12_450));
        assert_eq!(caps.bonus(MAX_LEVEL + 1, &normal), None);
        assert_eq!(caps.bonus(0, &normal), Some(0));
    }

    #[test]
    fn check_a_preidolized_non_promo_reads_the_normal_column() {
        let caps = corpus();
        let mut m = meta(3, true);
        m.is_preidolized_non_promo = true;
        assert_eq!(caps.bonus(MAX_LEVEL, &m), Some(12_450));
    }

    #[test]
    fn check_a_column_with_a_hole_is_refused() {
        let column = |skip: u16| {
            (MIN_LEVEL..=MAX_LEVEL)
                .filter(|l| *l != skip)
                .map(|l| format!(r#""{l}":0"#))
                .collect::<Vec<_>>()
                .join(",")
        };
        let json = format!("{{\"promo\":{{{}}},\"non_promo\":{{{}}}}}", column(300), column(0));

        match LevelCaps::parse(json.as_bytes()) {
            Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
            Ok(_) => panic!("`parse` accepted a 400-row column the absolute nerve"),
        }
    }
}
