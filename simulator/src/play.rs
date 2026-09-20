//! Orchestrator of one playthrough of one chart with one team.

use crate::play_config::PlayConfig;
use crate::song::{Chart, ChartRows, Event, EventKind, LANES, Time};

/// Transcription of `combo_bonuses.json`.
/// TODO: Actually use these. Might also move to a different file.
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

///`floor(team_strength * 0.0125 * 1.1)`.
#[must_use]
#[expect(
    clippy::integer_division,
    reason = "That's just how the game works, Clippy I don't make the rules."
)]
pub fn points_per_note(strength: u32) -> u32 {
    let scaled = u64::from(strength).saturating_mul(11) / 800;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// Hold notes are two events but one combo. Actually they're 3 events
/// Regular notes are two events, a spawn and a press.
/// Hold notes are 3 notes, a hold spawn, a hold tap start, and a hold tap end
/// Combo increments on a regular tap or a hold release.
const fn scores(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldRelease)
}

/// `floor(ppn * combo / 100)`.
/// TODO: attribute/group modifiers.
#[expect(clippy::integer_division, reason = "Game mechanic.")]
fn note_score(ppn: u32, combo: u16) -> u64 {
    u64::from(ppn).saturating_mul(u64::from(combo)) / 100
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Queue<'a> {
    times: &'a [Time],
    events: &'a [Event],
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
        let mut lanes = [0_u32; LANES];
        let mut notes = 0_u32;
        for &event in events {
            if scores(event) {
                notes = notes.saturating_add(1);
                if let Some(n) = lanes.get_mut(event.lane() as usize) {
                    *n = n.saturating_add(1);
                }
            }
            combo.push(combo_bonus(notes));
        }

        Some(Self { times, events, combo, lanes, notes })
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

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Outcome {
    pub score: u64,
    pub notes: u32,

    /// TODO: Everything related to skills.
    pub spawned: u32,
}

#[must_use]
#[expect(
    clippy::indexing_slicing,
    reason = "``Queue::build` guarantees that `tap` won't be out of bounds."
)]
pub fn run(queue: &Queue, config: &PlayConfig, ppn: u32) -> Outcome {
    let offset = config.speed.offset();
    let judgements = queue.judgements();
    let mut out = Outcome::default();
    let mut spawn = 0_usize;
    let mut tap = 0_usize;

    loop {
        let time = queue.times[tap];
        while spawn < judgements && queue.times[spawn].spawn(offset) <= time {
            // TODO: rhythm icons
            out.spawned = out.spawned.saturating_add(1);
            spawn = spawn.saturating_add(1);
        }

        let event = queue.events[tap];
        if event.kind() == EventKind::End {
            break;
        }

        // TODO: I guess the accuracy will go here
        if scores(event) {
            // TODO: I guess the note-type multipliers go here
            out.score = out.score.saturating_add(note_score(ppn, queue.combo[tap]));
            out.notes = out.notes.saturating_add(1);
        }

        tap = tap.saturating_add(1);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{Outcome, Queue, combo_bonus, points_per_note, run};
    use crate::play_config::{NoteSpeed, PlayConfig};
    use crate::song::{
        Chart, ChartArena, ChartIdx, ChartRows, Difficulty, Event, EventKind, SongIdx, Time,
    };

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
        PlayConfig::new(NoteSpeed::FASTEST, 100, 0)
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
        assert_eq!(points_per_note(800), 11);
        assert_eq!(points_per_note(0), 0);
        assert_eq!(points_per_note(72), 0);
        assert_eq!(points_per_note(100_000), 1375);

        // Checks that the int and float versions give the same resolve.
        #[expect(
            clippy::float_cmp,
            reason = "comparing to more intuitive way to understand formula"
        )]
        for strength in [1_u32, 999, 12_345, 250_000, 1_000_000, 2_700_000] {
            let float = (f64::from(strength) * 0.0125 * 1.1).floor();
            assert_eq!(f64::from(points_per_note(strength)), float, "at {strength}");
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

        let out = run(&queue, &config(), 100);
        assert_eq!(out.notes, 2);
        assert_eq!(out.spawned, 3);
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

        let out = run(&queue, &config(), 1375);
        assert_eq!(out, Outcome { score: 1375 * 3, notes: 3, spawned: 3 });

        assert_eq!(super::note_score(1375, 110), 1512);
    }

    #[test]
    fn check_combo_multiplier() {
        // 51 notes. Last note is the first to have 1.10x.
        let stream: Vec<_> = (0..51).map(|i| tap(2000 + i * 100, (i % 9) as u8)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        assert_eq!(queue.combo()[49], 100, "50th note");
        assert_eq!(queue.combo()[50], 110, "51st");

        let out = run(&queue, &config(), 1000);
        assert_eq!(out.score, 1000 * 50 + 1100);
    }
}
