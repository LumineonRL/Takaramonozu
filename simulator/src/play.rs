//! Orchestrator of one playthrough of one chart with one team.

use crate::play_config::PlayConfig;
use crate::rng::Rng;
use crate::scalar::{Centi, RATE_UNIT, Rate};
use crate::skill::{SkillEffect, TriggerType};
use crate::song::{Chart, ChartRows, Event, EventKind, LANES, Time};

/// I love magic numbers! PPN is calculated by `team_strength / 100 * 1.25`.
/// But of course float arithmetic is the greatest evil of man-kind,
/// Thankfully we can multiply by the nice rational 1/80 instead because it's the same number.
const BASE: u64 = 80;

/// 1.00x -> 100. Again, floats are cringe.
const UNIT: u16 = 100;

/// Note multiplierse can have an extra decimal. A swing hold gets a 0.625x multiplier. Wait a minute SIF1 doesn't
/// even have swing holds when I checked `songs.json` it has to though or else why would we have the multipliers for it.
/// My script searching `songs.json` must've been wrong. Surely. Maybe.
const NOTE_UNIT: u16 = 1000;

const JUDGEMENT_UNIT: u16 = 100;

/// Perfects are a 1.00x multiplier.
const PERFECT: u16 = JUDGEMENT_UNIT;

/// Greats are a 0.88x multiplier
const GREAT: u16 = 88;

const DIVISOR: u64 = BASE * 100 * 100 * 1000 * 100;

/// Transcription of `combo_bonuses.json`.
///
/// TODO: might move to a table module once there are several of these.
const COMBO_BONUS: [(u32, u16); 7] = [
    (0, 100),
    (51, 110),
    (101, 115),
    (201, 120),
    (401, 125),
    (601, 130),
    (801, 135),
];

/// I thought .`find` would be pretty slow but apparently the compiler makes it
/// not slow.
#[must_use]
pub fn combo_bonus(combo: u32) -> u16 {
    COMBO_BONUS
        .iter()
        .rev()
        .find(|&&(at, _)| combo >= at)
        .map_or(100, |&(_, mult)| mult)
}

/// `floor(team_strength * 0.0125)`
#[must_use]
#[expect(clippy::integer_division, reason = "Game mechanic.")]
pub fn points_per_note(team_strength: u32) -> u32 {
    let scaled = u64::from(team_strength) / BASE;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// Hold notes are two events but one combo. Actually they're 3 events
/// Regular notes are two events, a spawn and a press.
/// Hold notes are 3 notes, a hold spawn, a hold tap start, and a hold tap end
/// Combo increments on a regular tap or a hold release.
const fn scores(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldRelease)
}

const fn note_multiplier(event: Event) -> u16 {
    match (matches!(event.kind(), EventKind::HoldRelease), event.is_swing()) {
        (true, true) => 625,
        (true, false) => 1250,
        (false, true) => 500,
        (false, false) => NOTE_UNIT,
    }
}

/// `floor(team_strength * combo * lane * note * accuracy / 80_000_000_000)`.
/// TODO: Trick SIS.
#[expect(clippy::integer_division, reason = "Game mechanic.")]
fn note_score(team_strength: u32, combo: u16, lane: u16, note: u16, judgement: u16) -> u64 {
    u64::from(team_strength)
        .saturating_mul(u64::from(combo))
        .saturating_mul(u64::from(lane))
        .saturating_mul(u64::from(note))
        .saturating_mul(u64::from(judgement))
        / DIVISOR
}

/// Won't work for all skills, but there are some that we can pre-compute at compile-time
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Skill {
    pub value: Centi,
    pub chance: Rate,
    pub threshold: u16,
    pub effect: SkillEffect,
    pub trigger: TriggerType,
}

impl Skill {
    pub const NONE: Self = Self {
        value: 0,
        chance: 0,
        threshold: 0,
        effect: SkillEffect::Scorer,
        trigger: TriggerType::RhythmIcons,
    };
}

impl Default for Skill {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessorySkill {
    pub value: Centi,
    pub chance: Rate,
    pub effect: SkillEffect,
}

impl AccessorySkill {
    pub const NONE: Self = Self { value: 0, chance: 0, effect: SkillEffect::Scorer };

    #[must_use]
    pub const fn is_inert(self) -> bool {
        self.chance == 0
    }
}

impl Default for AccessorySkill {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Scoring {
    pub team_strength: u32,
    pub lanes: [u16; LANES],
    pub skills: [Skill; LANES],
    pub accessories: [AccessorySkill; LANES],
}

impl Scoring {
    #[must_use]
    pub const fn flat(team_strength: u32) -> Self {
        Self {
            team_strength,
            lanes: [UNIT; LANES],
            skills: [Skill::NONE; LANES],
            accessories: [AccessorySkill::NONE; LANES],
        }
    }
}

const fn spawns_icon(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldStart)
}

/// The big star of the show and it's kind of buried in the middle of the file,
/// but this is the priority queue that drives the entire simulation. much important many wow
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Queue<'a> {
    times: &'a [Time],
    events: &'a [Event],
    icons: Vec<Time>,
    combo: Vec<u16>,
    lanes: [u32; LANES],
    notes: u32,
}

impl<'a> Queue<'a> {
    #[must_use]
    pub fn build<R>(rows: &'a R, chart: &Chart) -> Option<Self>
    where
        R: ChartRows,
    {
        let (times, events) = rows.stream(chart);
        if events.last()?.kind() != EventKind::End {
            return None;
        }

        let mut combo = Vec::with_capacity(events.len());
        let mut icons = Vec::with_capacity(events.len());
        let mut lanes = [0_u32; LANES];
        let mut notes = 0_u32;
        for (&time, &event) in times.iter().zip(events) {
            if scores(event) {
                notes = notes.saturating_add(1);
                if let Some(n) = lanes.get_mut(event.lane() as usize) {
                    *n = n.saturating_add(1);
                }
            }
            if spawns_icon(event) {
                icons.push(time);
            }
            combo.push(combo_bonus(notes));
        }
        Some(Self { times, events, icons, combo, lanes, notes })
    }

    /// Tap event. So tap single note or hold start/end.
    #[must_use]
    pub fn judgements(&self) -> usize {
        self.events.len().saturating_sub(1)
    }

    /// Judgements that score and advance combo, so a hold counts once.
    #[must_use]
    pub const fn notes(&self) -> u32 {
        self.notes
    }

    #[must_use]
    pub fn icons(&self) -> u32 {
        u32::try_from(self.icons.len()).unwrap_or(u32::MAX)
    }

    #[must_use]
    pub const fn lanes(&self) -> [u32; LANES] {
        self.lanes
    }

    #[must_use]
    pub const fn times(&self) -> &[Time] {
        self.times
    }

    #[must_use]
    pub const fn events(&self) -> &[Event] {
        self.events
    }

    #[must_use]
    pub fn combo(&self) -> &[u16] {
        &self.combo
    }
}

/// Precompute acc rolls before a trial starts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Judgements {
    perfect: Vec<bool>,
    seed: u64,
    accuracy: Rate,
    perfects: u32,
}

impl Judgements {
    #[must_use]
    pub fn build(queue: &Queue, config: &PlayConfig) -> Self {
        let records = queue.events().len();
        let rolls = records.saturating_sub(1);

        let (perfect, perfects) = match config.accuracy {
            0 => (vec![false; records], 0),
            rate if rate >= RATE_UNIT => (vec![true; records], rolls),
            rate => {
                let mut rng = Rng::new(config.seed, Rng::ACCURACY);
                let mut perfect = vec![false; records];
                let mut hits = 0_usize;
                for slot in perfect.iter_mut().take(rolls) {
                    *slot = rng.roll(rate);
                    hits = hits.saturating_add(usize::from(*slot));
                }
                (perfect, hits)
            }
        };

        Self {
            perfect,
            seed: config.seed,
            accuracy: config.accuracy,
            perfects: u32::try_from(perfects).unwrap_or(u32::MAX),
        }
    }

    /// Whether a given judgement rolled as Perfect
    #[inline]
    #[must_use]
    pub fn at(&self, judgement: usize) -> bool {
        self.perfect.get(judgement).copied().unwrap_or(true)
    }

    #[must_use]
    pub fn records(&self) -> usize {
        self.perfect.len()
    }

    /// Judgements actually rolled
    #[must_use]
    pub fn rolls(&self) -> u32 {
        u32::try_from(self.records()).unwrap_or(u32::MAX).saturating_sub(1)
    }

    #[must_use]
    pub const fn perfects(&self) -> u32 {
        self.perfects
    }

    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    #[must_use]
    pub const fn accuracy(&self) -> Rate {
        self.accuracy
    }

    #[must_use]
    pub fn observed(&self) -> Rate {
        let scaled = u64::from(self.perfects)
            .saturating_mul(u64::from(RATE_UNIT))
            .checked_div(u64::from(self.rolls()))
            .unwrap_or(0);
        Rate::try_from(scaled).unwrap_or(RATE_UNIT)
    }
}

/// TODO: I might have a separate trigger handling module because this is going to get big fast.
/// TODO: Combo and Star Notes belong here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Triggers {
    at: Vec<u32>,
    lanes: Vec<u16>,
}

impl Triggers {
    /// # Panics
    /// Neverrrrr
    #[must_use]
    #[expect(clippy::arithmetic_side_effects, reason = "`bounded by threshold")]
    #[expect(clippy::expect_used, reason = "9 lanes fit u16 no problemo so we gucci.")]
    pub fn build(queue: &Queue, scoring: &Scoring) -> Self {
        const { assert!(LANES <= u16::BITS as usize, "u16") };

        let icons = queue.icons();
        let mut dense = vec![0_u16; icons as usize + 1];
        for (lane, skill) in scoring.skills.iter().enumerate() {
            if skill.trigger != TriggerType::RhythmIcons || skill.threshold == 0 {
                continue;
            }
            let bit = 1_u16.checked_shl(u32::try_from(lane).expect("9 lanes")).expect("u16");
            let step = u32::from(skill.threshold);
            let mut at = step;
            while at <= icons {
                if let Some(mask) = dense.get_mut(at as usize) {
                    *mask |= bit;
                }
                at += step;
            }
        }

        let mut out = Self { at: Vec::new(), lanes: Vec::new() };
        for (at, &mask) in dense.iter().enumerate() {
            if mask != 0 {
                out.at.push(u32::try_from(at).unwrap_or(u32::MAX));
                out.lanes.push(mask);
            }
        }
        out.at.push(u32::MAX);
        out.lanes.push(0);
        out
    }

    #[must_use]
    pub fn rolls(&self) -> u32 {
        self.lanes.iter().map(|m| m.count_ones()).sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lanes.len().saturating_sub(1)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Outcome {
    pub tap: u64,
    pub skill: u64,
    pub notes: u32,
    pub icons: u32,

    /// Not to be confused with the perfect judgement. Hold notes require both
    /// the head and tail or a hold to be perfect, else its a great.
    pub perfects: u32,
    pub activations: u32,
    pub accessory_activations: u32,
}

impl Outcome {
    #[must_use]
    pub const fn score(&self) -> u64 {
        self.tap.saturating_add(self.skill)
    }
}

/// Might move this to like, a `skill_activation_handler` module or something too Because again, when more skill effects get added
/// this can get unuwieldy
///
/// Ties happen right to left. Actually there's a coinflip mechanic involving Amplify, but we'll worry about that once I get around to Amplify.
#[inline]
fn fire(mask: u16, scoring: &Scoring, rng: &mut Rng, out: &mut Outcome) {
    let mut bits = mask;
    while let Some(lane) = bits.highest_one() {
        bits ^= bits.isolate_highest_one();
        let Some(skill) = scoring.skills.get(lane as usize) else {
            continue;
        };
        if rng.roll(skill.chance) {
            out.activations = out.activations.saturating_add(1);
            pay(skill.effect, skill.value, out);
            continue;
        }

        let Some(accessory) = scoring.accessories.get(lane as usize) else {
            continue;
        };
        if accessory.is_inert() || !rng.roll(accessory.chance) {
            continue;
        }
        out.accessory_activations = out.accessory_activations.saturating_add(1);
        pay(accessory.effect, accessory.value, out);
    }
}

/// TODO: every other effect.
#[inline]
fn pay(effect: SkillEffect, value: Centi, out: &mut Outcome) {
    if effect == SkillEffect::Scorer {
        out.skill = out.skill.saturating_add(u64::from(value));
    }
}

#[must_use]
#[expect(clippy::indexing_slicing, reason = "I'll allow it.")]
pub fn run(
    queue: &Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome {
    debug_assert_eq!(judgements.records(), queue.events.len(), "judgements of another chart");
    debug_assert_eq!(judgements.seed(), config.seed, "judgements of another trial");

    let offset = config.speed.offset();
    let icons: &[Time] = &queue.icons;
    let mut out = Outcome::default();
    let mut rng = Rng::new(config.seed, Rng::SKILLS);
    let mut heads = [false; LANES];
    let mut icon = 0_usize;
    let mut tap = 0_usize;
    let mut cursor = 0_usize;
    let mut next = triggers.at[0] as usize;

    loop {
        let time = queue.times[tap];

        while icon < icons.len() && icons[icon].spawn(offset) < time {
            icon = icon.saturating_add(1);
            if icon == next {
                fire(triggers.lanes[cursor], scoring, &mut rng, &mut out);
                cursor = cursor.saturating_add(1);
                next = triggers.at[cursor] as usize;
            }
        }

        let event = queue.events[tap];
        let lane = event.lane() as usize;
        match event.kind() {
            EventKind::End => break,
            EventKind::HoldStart => {
                if let Some(head) = heads.get_mut(lane) {
                    *head = judgements.at(tap);
                }
            }

            kind @ (EventKind::Tap | EventKind::HoldRelease) => {
                let mut perfect = judgements.at(tap);
                if kind == EventKind::HoldRelease {
                    perfect &= heads.get(lane).copied().unwrap_or(true);
                }
                out.tap = out.tap.saturating_add(note_score(
                    scoring.team_strength,
                    queue.combo[tap],
                    scoring.lanes.get(lane).copied().unwrap_or(UNIT),
                    note_multiplier(event),
                    if perfect { PERFECT } else { GREAT },
                ));
                out.notes = out.notes.saturating_add(1);
                out.perfects = out.perfects.saturating_add(u32::from(perfect));
                // TODO: the counter triggers roll here
            }
        }

        tap = tap.saturating_add(1);
    }

    out.icons = u32::try_from(icon).unwrap_or(u32::MAX);
    out
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::integer_division,
        reason = "Kind of need to verify that the things we're doing to avoid division still result in the same number."
    )]

    use super::{
        AccessorySkill, Judgements, Outcome, Queue, Scoring, Skill, Triggers, combo_bonus,
        points_per_note, run,
    };
    use crate::play_config::{NoteSpeed, PlayConfig};
    use crate::rng::Rng;
    use crate::scalar::{RATE_UNIT, Rate};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{
        Chart, ChartArena, ChartIdx, ChartRows, Difficulty, Event, EventKind, SongIdx, Time,
    };

    /// Fake skill that always procs so they can be tested.
    const ALWAYS: Rate = RATE_UNIT;

    fn arena(judgements: &[(Time, Event)]) -> ChartArena {
        let mut arena = ChartArena::default();
        arena.push(SongIdx(0), Difficulty::Master, judgements);
        arena
    }

    fn only(arena: &ChartArena) -> Chart {
        *arena.chart(ChartIdx(0)).expect("the one chart just pushed")
    }

    fn tap(ms: u32, lane: u8) -> (Time, Event) {
        (Time(ms), Event::new(lane, EventKind::Tap, false, false))
    }

    fn config() -> PlayConfig {
        PlayConfig::new(NoteSpeed::FASTEST, PlayConfig::ALL_PERFECT, 0)
    }

    fn scorer(threshold: u16, chance: Rate, value: u32) -> Skill {
        Skill {
            value,
            chance,
            threshold,
            effect: SkillEffect::Scorer,
            trigger: TriggerType::RhythmIcons,
        }
    }

    fn worn(chance: Rate, value: u32) -> AccessorySkill {
        AccessorySkill { value, chance, effect: SkillEffect::Scorer }
    }

    fn play(queue: &Queue, scoring: &Scoring) -> Outcome {
        seeded(queue, scoring, 0)
    }

    fn seeded(queue: &Queue, scoring: &Scoring, seed: u64) -> Outcome {
        at(queue, scoring, &PlayConfig { seed, ..config() })
    }

    fn at(queue: &Queue, scoring: &Scoring, config: &PlayConfig) -> Outcome {
        let triggers = Triggers::build(queue, scoring);
        let judgements = Judgements::build(queue, config);
        run(queue, config, scoring, &triggers, &judgements)
    }

    fn sloppy(accuracy: Rate, seed: u64) -> PlayConfig {
        PlayConfig { accuracy, seed, ..config() }
    }

    #[test]
    fn check_combo_table() {
        assert_eq!(combo_bonus(0), 100, "first 50 notes");
        assert_eq!(combo_bonus(50), 100);
        assert_eq!(combo_bonus(51), 110);
        assert_eq!(combo_bonus(100), 110);
        assert_eq!(combo_bonus(101), 115);
        assert_eq!(combo_bonus(200), 115);
        assert_eq!(combo_bonus(201), 120);
        assert_eq!(combo_bonus(401), 125);
        assert_eq!(combo_bonus(601), 130);
        assert_eq!(combo_bonus(800), 130);
        assert_eq!(combo_bonus(801), 135);
        assert_eq!(combo_bonus(u32::MAX), 135);
    }

    #[test]
    fn check_ppn() {
        assert_eq!(points_per_note(800), 10);
        assert_eq!(points_per_note(0), 0);
        assert_eq!(points_per_note(79), 0);
        assert_eq!(points_per_note(100_000), 1250);

        // Checks that the int and float versions give the same resolve.
        #[expect(
            clippy::float_cmp,
            reason = "comparing to more intuitive way to understand formula"
        )]
        for team_strength in [1_u32, 999, 12_345, 250_000, 1_000_000, 2_700_000] {
            let float = (f64::from(team_strength) * 0.0125).floor();
            assert_eq!(f64::from(points_per_note(team_strength)), float, "at {team_strength}");
        }
    }

    #[test]
    fn check_accuracy_multipliers() {
        const TABLE_PERFECT: u64 = 125;
        const TABLE_GREAT: u64 = 110;

        assert_eq!(100 * 100, super::BASE * TABLE_PERFECT, "0.01 * 1.25 = 1/80");
        assert_eq!(
            TABLE_GREAT * u64::from(super::JUDGEMENT_UNIT),
            u64::from(super::GREAT) * TABLE_PERFECT
        );
        assert_eq!(super::PERFECT, super::JUDGEMENT_UNIT, "assumed PERFECT");

        assert_eq!(11 * 100 * 100 * 100, 800 * TABLE_PERFECT * 110);

        assert_eq!(super::DIVISOR, 800_000_000 * u64::from(super::JUDGEMENT_UNIT));
        for team_strength in [1_u32, 999, 26_540, 80_400, 2_700_000, 16_000_000] {
            assert_eq!(
                super::note_score(team_strength, 135, 121, 1250, super::PERFECT),
                u64::from(team_strength) * 135 * 121 * 1250 / 800_000_000,
            );
        }
    }

    #[test]
    fn check_a_hold_is_two_judgements_but_one_note() {
        let stream = [
            tap(2000, 0),
            (Time(3000), Event::new(1, EventKind::HoldStart, false, false)),
            (Time(4000), Event::new(1, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("donege");

        assert_eq!(queue.judgements(), 3);
        assert_eq!(queue.notes(), 2);
        assert_eq!(queue.lanes()[1], 1);

        let out = play(&queue, &Scoring::flat(800_000));
        assert_eq!(out.notes, 2);
        assert_eq!(
            out.icons, 2,
            "a hold is one note, which shall hencefoth be referred to as one rhythm icon"
        );
        assert_eq!(out.score(), 10_000 + 12_500, "1.25x hold note multiplier");
    }

    #[test]
    fn check_hold_note_start() {
        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            tap(4000, 1),
            (Time(9000), Event::new(0, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.icons(), 2, "one for the hold, one for the tap");
        assert_eq!(queue.icons(), queue.notes(), "same count");

        let mut scoring = Scoring::flat(0);
        scoring.skills[3] = scorer(1, ALWAYS, 7);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 2);
        assert_eq!(out.skill, 14);

        let slow = PlayConfig::new(NoteSpeed::SLOWEST, PlayConfig::ALL_PERFECT, 0);
        assert_eq!(at(&queue, &scoring, &slow).icons, 2);
    }

    #[test]
    fn check_simultaneous_note_spawns() {
        let stream = [tap(1400, 0), tap(2000, 1)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.times()[1].spawn(NoteSpeed::FASTEST.offset()), queue.times()[0]);

        let mut scoring = Scoring::flat(0);
        scoring.skills[0] = scorer(1, ALWAYS, 5);
        let out = play(&queue, &scoring);
        assert_eq!(out.icons, 2);
        assert_eq!(out.activations, 2);
    }

    #[test]
    fn check_lane_counts() {
        let stream: Vec<_> =
            (0_u32..).zip([0_u8, 3, 3, 8, 3]).map(|(i, l)| tap(2000 + i * 500, l)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        assert_eq!(queue.lanes(), [1, 0, 0, 3, 0, 0, 0, 0, 1]);
        assert_eq!(queue.lanes().iter().sum::<u32>(), queue.notes());
    }

    #[test]
    fn check_score_floor() {
        let stream = [tap(2000, 0), tap(2500, 1), tap(3000, 2)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // 110_000 team_strength = 1375 ppn
        let out = play(&queue, &Scoring::flat(110_000));
        assert_eq!(
            out,
            Outcome {
                tap: 1375 * 3,
                notes: 3,
                icons: 3,
                perfects: 3,
                ..Outcome::default()
            }
        );

        assert_eq!(super::note_score(110_000, 110, 100, 1000, 100), 1512);

        assert_eq!(points_per_note(26_540), 331);
        assert_eq!(super::note_score(26_540, 130, 110, 1000, 100), 474);
        assert_eq!(331 * 130 * 110 / 10_000, 473, "pre-flooring the base is a no-no apparently.");
    }

    #[test]
    fn check_note_type_multipliers() {
        let stream = [
            tap(2000, 0),
            (Time(2100), Event::new(1, EventKind::Tap, true, false)),
            (Time(2200), Event::new(2, EventKind::Tap, false, true)),
            (Time(2300), Event::new(3, EventKind::HoldStart, false, false)),
            (Time(2400), Event::new(3, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let out = play(&queue, &Scoring::flat(800_000));
        assert_eq!(out.notes, 4, "hold note = 1 combo");
        assert_eq!(out.icons, 4, "and 1 rhythm icon, at the head");
        assert_eq!(out.score(), 10_000 + 10_000 + 5_000 + 12_500);

        let regular = Event::new(0, EventKind::Tap, false, false);
        let star = Event::new(0, EventKind::Tap, true, false);
        let swing = Event::new(0, EventKind::Tap, false, true);
        let start = Event::new(0, EventKind::HoldStart, false, false);
        let release = Event::new(0, EventKind::HoldRelease, false, false);
        assert_eq!(super::note_multiplier(regular), 1000);
        assert_eq!(super::note_multiplier(star), 1000, "a star is worth 1.0");
        assert_eq!(super::note_multiplier(swing), 500);
        assert_eq!(super::note_multiplier(release), 1250);
        assert_eq!(super::note_multiplier(start), 1000, "and it never scores anyway");

        let both = Event::new(0, EventKind::HoldRelease, false, true);
        assert_eq!(super::note_multiplier(both), 625, "1.25 * 0.5");
    }

    #[test]
    fn check_score_formula() {
        // One note per lane
        let stream: Vec<_> = (0_u8..9).map(|i| tap(2000 + u32::from(i) * 100, i)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // Under 51 notes, so combo is 1.00x.
        // 80,400 team strength is 1005 ppn
        let mut scoring = Scoring::flat(80_400);
        scoring.lanes = [100, 110, 121, 100, 110, 121, 100, 110, 121];
        let out = play(&queue, &scoring);
        assert_eq!(out.score(), 3 * (1005 + 1105 + 1216));

        assert_eq!(super::note_score(80_400, 100, 121, 1000, 100), 1216);
        assert_eq!(super::note_score(80_400, 100, 110, 1000, 100), 1105);
        assert_eq!(super::note_score(88_400, 100, 110, 1000, 100), 1215);

        assert_eq!(
            super::note_score(80_400, 110, 110, 1000, 100),
            1216,
            "1005 * 1.21 the long way"
        );
    }

    #[test]
    fn check_combo_multiplier() {
        // 51 notes. Last note is the first to have 1.10x.
        let stream: Vec<_> = (0..51).map(|i| tap(2000 + i * 100, (i % 9) as u8)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        assert_eq!(queue.combo()[49], 100, "50th note");
        assert_eq!(queue.combo()[50], 110, "51st");

        // 80k team_strength is 1000ppn
        let out = play(&queue, &Scoring::flat(80_000));
        assert_eq!(out.score(), 1000 * 50 + 1100);
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "why can't you configure clippy.toml to disable this in tests?"
    )]
    fn fifty_one() -> ChartArena {
        let stream: Vec<_> = (0..51).map(|i| tap(2000 + i * 100, (i % 9) as u8)).collect();
        arena(&stream)
    }

    #[test]
    fn check_scorer_and_charm() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        // This card is 26 icons at a 26% chance for 1120 points.
        let mut scoring = Scoring::flat(80_000);
        scoring.skills[2] = scorer(26, ALWAYS, 1120);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 1);
        assert_eq!(out.skill, 1120);
        assert_eq!(out.tap, 1000 * 50 + 1100, "tap unchanged");
        assert_eq!(out.score(), out.tap + out.skill);

        scoring.skills[2] = scorer(26, ALWAYS, 2800);
        assert_eq!(play(&queue, &scoring).skill, 2800);

        scoring.skills[2] = scorer(26, 0, 1120);
        let cold = play(&queue, &scoring);
        assert_eq!((cold.activations, cold.skill), (0, 0));
        assert_eq!(Triggers::build(&queue, &scoring).rolls(), 1);
    }

    #[test]
    fn check_tap_score_before_skills() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring::flat(80_000);
        let bare = play(&queue, &scoring).tap;

        scoring.skills[5] = scorer(1, ALWAYS, 1_000_000);
        let loud = play(&queue, &scoring);
        assert_eq!(loud.activations, 51, "every icon");
        assert_eq!(loud.tap, bare);
    }

    #[test]
    fn check_accessory_trigger() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[2] = scorer(1, ALWAYS, 1120);
        scoring.accessories[2] = worn(ALWAYS, 9000);
        let hits = play(&queue, &scoring);
        assert_eq!((hits.activations, hits.accessory_activations), (51, 0));
        assert_eq!(hits.skill, 51 * 1120, "acc never got a chance to activate");

        scoring.skills[2] = scorer(1, 0, 1120);
        let misses = play(&queue, &scoring);
        assert_eq!((misses.activations, misses.accessory_activations), (0, 51));
        assert_eq!(misses.skill, 51 * 9000, "acc can now activate");
    }

    /// A member with no skill has none to fail, so an unscheduled lane silences
    /// its accessory.
    #[test]
    fn check_no_card_skill_means_no_acc_skill() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[3] = Skill::NONE;
        scoring.accessories[3] = worn(ALWAYS, 9000);
        assert_eq!(Triggers::build(&queue, &scoring).rolls(), 0, "nothing to fail");

        let out = play(&queue, &scoring);
        assert_eq!((out.accessory_activations, out.skill), (0, 0));
    }

    #[test]
    fn check_all_great_12_perc_lower_than_ap() {
        let stream = [tap(2000, 0), tap(2500, 1), tap(3000, 2)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let scoring = Scoring::flat(800_000); // 10_000 a note flat

        assert_eq!(at(&queue, &scoring, &config()).tap, 3 * 10_000);
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).tap, 3 * 8_800, "0.88 a note");

        for seed in 0..16 {
            let all_perfect = at(&queue, &scoring, &sloppy(PlayConfig::ALL_PERFECT, seed));
            let all_great = at(&queue, &scoring, &sloppy(0, seed));
            assert_eq!(all_perfect.perfects, 3, "seed {seed}");
            assert_eq!(all_great.perfects, 0, "seed {seed}");
            assert_eq!(all_perfect.tap, 3 * 10_000, "seed {seed}");
            assert_eq!(all_great.tap, 3 * 8_800, "seed {seed}");
        }

        assert_eq!(at(&queue, &scoring, &sloppy(RATE_UNIT * 2, 7)).tap, 3 * 10_000);
    }

    #[test]
    fn check_a_hold_is_perfect_only_if_both_taps_are() {
        const FLIP: Rate = RATE_UNIT / 2;

        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            (Time(3000), Event::new(0, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let scoring = Scoring::flat(800_000);
        let triggers = Triggers::build(&queue, &scoring);

        let mut seen = [false; 4];
        for seed in 0..64 {
            let config = sloppy(FLIP, seed);
            let judged = Judgements::build(&queue, &config);
            assert_eq!(judged.rolls(), 2, "hold is 2 judgements");
            assert_eq!(judged.records(), queue.events().len());

            let (head, release) = (judged.at(0), judged.at(1));
            let out = run(&queue, &config, &scoring, &triggers, &judged);

            let both = head && release;
            assert_eq!(out.notes, 1, "holds score once at the end.");
            assert_eq!(out.perfects, u32::from(both), "seed {seed}: {head} then {release}");
            assert_eq!(
                out.tap,
                if both { 12_500 } else { 11_000 },
                "seed {seed}: {head} then {release}",
            );
            assert_eq!(judged.perfects(), u32::from(head) + u32::from(release));
            seen[usize::from(head) * 2 + usize::from(release)] = true;
        }
        assert!(seen.iter().all(|&s| s), "Seeds missed a (head, release) pair: {seen:?}");
    }

    #[test]
    fn check_diff_teams_same_judgements() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut one = Scoring::flat(80_000);
        one.skills[2] = scorer(5, RATE_UNIT / 2, 900);
        let mut two = Scoring::flat(123_456);
        two.lanes = [100, 110, 121, 100, 110, 121, 100, 110, 121];
        two.skills[7] = scorer(2, RATE_UNIT / 4, 40);
        two.accessories[7] = worn(RATE_UNIT / 2, 7);

        let mut differed = false;
        for seed in 0..32 {
            let config = sloppy(8000, seed);
            let judged = Judgements::build(&queue, &config);
            let a = run(&queue, &config, &one, &Triggers::build(&queue, &one), &judged);
            let b = run(&queue, &config, &two, &Triggers::build(&queue, &two), &judged);
            assert_eq!(a.perfects, b.perfects, "seed {seed}");
            assert_eq!(a.perfects, judged.perfects(), "no holds");
            differed |= a.skill != b.skill;
        }
        assert!(differed, "the two teams never rolled differently");
    }

    #[test]
    fn check_skills_resolve_right_to_left() {
        const FLIP: Rate = RATE_UNIT / 2;

        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut scoring = Scoring::flat(0);
        scoring.skills[7] = scorer(51, FLIP, 1000);
        scoring.skills[1] = scorer(51, FLIP, 1);
        assert_eq!(Triggers::build(&queue, &scoring).len(), 1, "one icon count, two lanes");

        // A manually and meticulously selected seed where the two rolls after different
        let seed = (0..64_u64)
            .find(|&s| {
                let mut rng = Rng::new(s, Rng::SKILLS);
                rng.roll(FLIP) != rng.roll(FLIP)
            })
            .expect("a seed where the first two draws differ");

        let mut rng = Rng::new(seed, Rng::SKILLS);
        let high = rng.roll(FLIP); // lane 8 first
        let low = rng.roll(FLIP);
        let want = u64::from(high) * 1000 + u64::from(low);

        assert_eq!(seeded(&queue, &scoring, seed).skill, want, "seed {seed}");
    }
}
