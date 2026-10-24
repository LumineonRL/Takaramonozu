// WIP

//! Catalog of scenarios to benchmark.
//!
//! Two main goals here:
//! A) Test the performance of as many scnearios of *this* simulator as possible
//! B) A/B test this simulator's performance against Burrito's where possible.
//! Apples-to-apples comparisons won't always be possible (see `DIVERGENCES.md`) but where possible I'm
//! going to construct the test scenarios in a way that minimizes differences.
//! Which means we can't A/B test maps that have swing-holds, no teams with Sync or Score-Scorers, etc.
//! TODO: Will need to submodule burrito's sim and then wire it up so they can both read the same spec.

use deck::accessory::Accessories;
use deck::card::{Deck, Gallery};
use deck::compile::compile;
use deck::level_cap::{self, LevelCaps};
use deck::manager::{EntryId, Repairs};
use deck::sis::SisInventory;
use deck::team::{Context, LANES, Team};
use serde::Deserialize;
use simulator::accessory::AccessoryDb;
use simulator::archive::{Kind, Mapped};
use simulator::card::CardDb;
use simulator::play::activation::Triggers;
use simulator::play::config::{NoteSpeed, PlayConfig};
use simulator::play::judgement::Judgements;
use simulator::play::queue::Queue;
use simulator::play::{Outcome, Scoring, run};
use simulator::scalar::Rate;
use simulator::sis::SisDb;
use simulator::skill::Targets;
use simulator::song::{ChartRows, Difficulty, Matching, SongDb};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    #[serde(skip)]
    pub name: String,
    pub run_burrito: bool,
    pub chart: ChartKey,
    pub lanes: [Lane; LANES],
    #[serde(default)]
    pub guest: Option<u16>,
    pub accuracy: Rate,
    pub speed: u8,
}

// This isn't actually unique for Burrito (it wasn't unique here either until a month or two ago)
// but there's a few charts with the same title, same difficulty, same attribute, but different charts.
// I had renamed these in `songs.json` to differentiate the originals from the Utamero versions, but that
// fix isn't in Burrito's sim. All of that is to say I should avoid those maps when benchmarking anything
// against Burrito to ensure the wrong chart for a song doesn't get picked. There's only like 5 such songs
// I don't remember all of them off the top of my head but borarara is one.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChartKey {
    pub title: String,
    pub difficulty: Level,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
pub enum Level {
    Easy,
    Normal,
    Hard,
    Expert,
    Master,
}

impl From<Level> for Difficulty {
    fn from(level: Level) -> Self {
        match level {
            Level::Easy => Self::Easy,
            Level::Normal => Self::Normal,
            Level::Hard => Self::Hard,
            Level::Expert => Self::Expert,
            Level::Master => Self::Master,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    pub card: u16,
    #[serde(default = "idolized")]
    pub idolized: bool,
    #[serde(default = "level_one")]
    pub skill_level: u8,
    #[serde(default = "min_cap")]
    pub cap_level: u16,
    #[serde(default)]
    pub sis_slots: Option<u8>,
    #[serde(default)]
    pub accessory: Option<Worn>,
    #[serde(default)]
    pub sis: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worn {
    pub id: u16,
    pub skill_level: u8,
}

const fn idolized() -> bool {
    true
}

const fn level_one() -> u8 {
    1
}

const fn min_cap() -> u16 {
    level_cap::MIN_LEVEL
}
