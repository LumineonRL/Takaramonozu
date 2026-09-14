//! Turns `data/*.json` into the archives `simulator` mmaps.

pub mod load;

use core::error::Error;
use std::path::Path;

use simulator::accessory::AccessoryDb;
use simulator::card::CardDb;
use simulator::scalar::Roster;
use simulator::sis::SisDb;
use simulator::song::SongDb;

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
/// column that contradicts the shape the stored rows assume, or a missing char name.
pub fn accessories(dir: &Path, cards: &CardDb) -> Result<AccessoryDb, Failed> {
    let raw_accs = raw::parse_accessories(&std::fs::read(dir.join("accessories.json"))?)?;
    let year_group = raw::parse_target_map(&std::fs::read(dir.join("year_group_target.json"))?)?;
    Ok(accessory::build(&raw_accs, &year_group, cards)?)
}

/// Reads `sis.json` out of `dir` and builds against the card roster.
///
/// years for year-based SIS (e.g. "Second years")
/// groups for Nonet.
///
/// # Errors
/// A missing or unreadable file, a schema change serde rejects, or a restriction
/// that resolves against neither its map nor the roster.
pub fn sis(dir: &Path, roster: &Roster) -> Result<SisDb, Failed> {
    let raw_sis = raw::parse_sis(&std::fs::read(dir.join("sis.json"))?)?;
    let years = raw::parse_target_map(&std::fs::read(dir.join("year_group_mapping.json"))?)?;
    let groups = raw::parse_target_map(&std::fs::read(dir.join("group_member_map.json"))?)?;
    Ok(load::sis::build(&raw_sis, &years, &groups, roster)?)
}

/// Reads `songs.json` out of `dir` and builds against the card roster.
///
/// `additional_leader_skill_map` has each of the group -> member sets.
///
/// # Errors
/// A missing or unreadable file, a schema change serde rejects, or a chart that
/// contradicts what's possible.
pub fn songs(dir: &Path, roster: &Roster) -> Result<SongDb, Failed> {
    let raw_songs = raw::parse_songs(&std::fs::read(dir.join("songs.json"))?)?;
    let groups =
        raw::parse_target_map(&std::fs::read(dir.join("additional_leader_skill_map.json"))?)?;
    Ok(load::song::build(&raw_songs, &groups, roster)?)
}
