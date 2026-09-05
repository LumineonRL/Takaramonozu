//! Turns `data/*.json` into the archives `simulator` mmaps.

pub mod load;

use core::error::Error;
use std::path::Path;

use simulator::accessory::AccessoryDb;
use simulator::card::CardDb;
use simulator::scalar::Roster;

use crate::load::{accessory, card, raw};

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

/// Reads `accessories.json` out of `dir` and builds against the card roster.
///
/// # Errors
/// A missing or unreadable file, a schema change serde rejects, or an accessory
/// column that contradicts the shape the stored rows assume.
pub fn accessories(dir: &Path, roster: &Roster) -> Result<AccessoryDb, Failed> {
    let raw_accs = raw::parse_accessories(&std::fs::read(dir.join("accessories.json"))?)?;
    let year_group = raw::parse_target_map(&std::fs::read(dir.join("year_group_target.json"))?)?;
    Ok(accessory::build(&raw_accs, &year_group, roster)?)
}
