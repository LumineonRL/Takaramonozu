//! Catalog of scenarios to benchmark.
//!
//! Two main goals here:
//! A) Test the performance of as many scnearios of *this* simulator as possible
//! B) A/B test this simulator's performance against Burrito's where possible.
//! Apples-to-apples comparisons won't always be possible (see `DIVERGENCES.md`) but where possible I'm
//! going to construct the test scenarios in a way that minimizes differences.
//! Which means we can't A/B test maps that have swing-holds, no teams with Sync or Score-Scorers, etc.
//! TODO: Will need to submodule burrito's sim and then wire it up so they can both read the same spec.

use core::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

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

pub type Failed = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    #[serde(skip)]
    pub name: String,
    // If the benchmark should only run for this sim or burrito's too.
    // Turned off if the spec contains mechanics or songs that burrito sim
    // either didn't implement or has an error on.
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

#[must_use]
pub fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

/// # Errors
/// Bad dir, invalid spec.
pub fn catalog(dir: &Path) -> Result<Vec<Spec>, Failed> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "json"));
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let mut spec: Spec = serde_json::from_slice(&fs::read(p)?)
                .map_err(|e| format!("{}: {e}", p.display()))?;
            let stem = p.file_stem().and_then(|s| s.to_str());
            spec.name = stem.ok_or_else(|| format!("{}: not UTF-8", p.display()))?.into();
            Ok(spec)
        })
        .collect()
}

pub struct Archives {
    cards: CardDb,
    accessories: AccessoryDb,
    sis: SisDb,
    caps: LevelCaps,
    songs: Mapped<SongDb>,
    groups: Targets,
}

impl Archives {
    /// # Errors
    /// Bad rkyv
    pub fn open(root: &Path) -> Result<Self, Failed> {
        let songs: Mapped<SongDb> = mapped(&root.join("archive/songs.rkyv"))?;
        let groups = rkyv::deserialize::<Targets, rkyv::rancor::Error>(&songs.root().groups)?;
        Ok(Self {
            cards: owned(&root.join("archive/cards.rkyv"))?,
            accessories: owned(&root.join("archive/accessories.rkyv"))?,
            sis: owned(&root.join("archive/sis.rkyv"))?,
            caps: LevelCaps::load(&root.join("data/level_cap_bonuses.json"))?,
            songs,
            groups,
        })
    }

    /// # Errors
    /// Bad spec. Like, invalid cards, or trying to equip something illegal.
    pub fn prepare(&self, spec: Spec) -> Result<Prepared<'_>, Failed> {
        let songs = self.songs.root();
        let title = &spec.chart.title;
        let difficulty = Difficulty::from(spec.chart.difficulty);
        let idx = songs
            .songs
            .iter()
            .position(|song| *song.title == **title)
            .ok_or_else(|| format!("{}: no song {title:?}", spec.name))?;
        let song = songs.songs.get(idx).ok_or("can't find")?;
        let chart = songs
            .charts
            .charts()
            .iter()
            .find(|c| usize::from(c.song.0) == idx && c.difficulty == difficulty)
            .ok_or_else(|| format!("{}: no {difficulty:?} chart of {title:?}", spec.name))?;
        let matching = Matching::new(song.attribute, song.group.as_ref().copied(), &self.groups)
            .ok_or("missing song")?;
        let scoring = self.compiled(&spec, matching).map_err(|e| format!("{}: {e}", spec.name))?;
        let queue = Queue::build(&songs.charts, chart).ok_or("an unterminated stream")?;
        let triggers = Triggers::build(&queue, &scoring);
        let speed = NoteSpeed::new(spec.speed).ok_or("bad approach rate")?;
        let config = PlayConfig::new(speed, spec.accuracy, 0);
        Ok(Prepared { spec, queue, scoring, triggers, config })
    }

    fn compiled(&self, spec: &Spec, matching: Matching) -> Result<Scoring, Failed> {
        let cards = &self.cards;
        let mut deck = Deck::new(spec.name.clone(), Gallery::default());
        let mut accessories = Accessories::new(spec.name.clone());
        let mut inventory = SisInventory::new(spec.name.clone());
        let mut owned: Vec<(EntryId, Option<EntryId>, Vec<EntryId>)> = Vec::with_capacity(LANES);
        for lane in &spec.lanes {
            let meta = cards
                .index(lane.card)
                .and_then(|i| cards.meta.get(usize::from(i.0)))
                .ok_or_else(|| format!("no card {}", lane.card))?;
            let form_max = meta.sis_max.get(usize::from(lane.idolized)).copied();
            let slots = lane.sis_slots.or(form_max).ok_or("no SIS bound")?;
            let card = exact(
                "card",
                lane.card,
                deck.add(cards, lane.card, lane.idolized, lane.skill_level, slots, lane.cap_level),
            )?;
            let worn = lane
                .accessory
                .map(|w| {
                    let added = accessories.add(&self.accessories, w.id, w.skill_level);
                    exact("accessory", w.id, added)
                })
                .transpose()?;
            let sis = lane
                .sis
                .iter()
                .map(|&id| exact("SIS", id, inventory.add(&self.sis, id)))
                .collect::<Result<_, _>>()?;
            owned.push((card, worn, sis));
        }
        let ctx = Context {
            deck: &deck,
            accessories: &accessories,
            inventory: &inventory,
            cards,
            acc_db: &self.accessories,
            sis_db: &self.sis,
        };
        let mut team = Team::default();
        for (lane, (card, worn, sis)) in owned.iter().enumerate() {
            let refused =
                |what: &'static str| move |p| format!("can't equip {what} on {lane}. {p:?}");
            team.equip_card(lane, *card, ctx).map_err(refused("card"))?;
            if let Some(acc) = *worn {
                team.equip_accessory(lane, acc, ctx).map_err(refused("accessory"))?;
            }
            for &id in sis {
                team.equip_sis(lane, id, ctx).map_err(refused("SIS"))?;
            }
        }
        if let Some(card_id) = spec.guest {
            let leader = cards
                .index(card_id)
                .and_then(|i| cards.cards.get(usize::from(i.0)))
                .ok_or_else(|| format!("invalid guest {card_id}"))?
                .leader;
            team.set_guest(leader, ctx).map_err(|p| format!("guest {card_id}: {p:?}"))?;
        }
        Ok(compile(&team, ctx, &self.caps, matching).scoring())
    }
}

pub struct Prepared<'a> {
    pub spec: Spec,
    pub queue: Queue<'a>,
    pub scoring: Scoring,
    pub triggers: Triggers,
    pub config: PlayConfig,
}

impl Prepared<'_> {
    #[must_use]
    pub fn trial(&self, seed: u64) -> Outcome {
        let config = PlayConfig { seed, ..self.config };
        let judgements = Judgements::build(&self.queue, &config);
        run(&self.queue, &config, &self.scoring, &self.triggers, &judgements)
    }
}

fn exact<P>(
    what: &str,
    id: u16,
    added: Result<(EntryId, Repairs), Vec<P>>,
) -> Result<EntryId, Failed>
where
    P: core::fmt::Debug,
{
    match added {
        Ok((entry, repairs)) if repairs.is_empty() => Ok(entry),
        Ok((_, repairs)) => Err(format!("{what} {id} would be repaired: {repairs:?}").into()),
        Err(problems) => Err(format!("{what} {id}: {problems:?}").into()),
    }
}

fn mapped<T>(path: &Path) -> Result<Mapped<T>, Failed>
where
    T: rkyv::Archive + Kind,
    T::Archived: rkyv::Portable,
{
    // SAFETY: rkyv built here
    let mapped = unsafe { Mapped::open(path) };
    mapped.map_err(|e| format!("{}: {e}; run `cargo run -p preprocess`", path.display()).into())
}

fn owned<T>(path: &Path) -> Result<T, Failed>
where
    T: rkyv::Archive + Kind,
    T::Archived: rkyv::Portable
        + rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rkyv::rancor::Error>>,
{
    Ok(rkyv::deserialize::<T, rkyv::rancor::Error>(mapped::<T>(path)?.root())?)
}
