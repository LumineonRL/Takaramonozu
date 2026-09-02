//! Turns `data/*.json` into the archives `simulator` mmaps.

pub mod load;

use core::error::Error;
use std::path::Path;

use simulator::card::CardDb;

use crate::load::{card, raw};

pub type Failed = Box<dyn Error>;

/// Reads the three card files out of `dir` and assembles the database.
///
/// # Errors
/// A missing or unreadable file, a schema change serde rejects, or a card that
/// fails one of the card list invariants.
pub fn cards(dir: &Path) -> Result<CardDb, Failed> {
    let raw_cards = raw::parse(&std::fs::read(dir.join("cards.json"))?)?;
    let leaders =
        raw::parse_target_map(&std::fs::read(dir.join("additional_leader_skill_map.json"))?)?;
    let year_group = raw::parse_target_map(&std::fs::read(dir.join("year_group_target.json"))?)?;
    Ok(card::build(&raw_cards, &leaders, &year_group)?)
}
