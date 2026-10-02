//! The priority queue of events, in charge of determining what event comes next.
//! other modules handle dispatching the event to its relevant handler.

use super::tap::combo_bonus;
use crate::song::{Chart, ChartRows, Event, EventKind, LANES, Time};

/// Hold notes are two events but one combo. Actually they're 3 events
/// Regular notes are two events, a spawn and a press.
/// Hold notes are 3 notes, a hold spawn, a hold tap start, and a hold tap end
/// Combo increments on a regular tap or a hold release.
const fn scores(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldRelease)
}

const fn spawns_icon(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldStart)
}

/// The big star of the show. This is the priority queue that drives the entire
/// simulation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Queue<'a> {
    times: &'a [Time],
    events: &'a [Event],
    icons: Vec<Time>,
    combo: Vec<u16>,
    lanes: [u32; LANES],
    notes: u32,
    stars: u32,
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
        let mut stars = 0_u32;
        for (&time, &event) in times.iter().zip(events) {
            if scores(event) {
                notes = notes.saturating_add(1);
                stars = stars.saturating_add(u32::from(event.is_star()));
                if let Some(n) = lanes.get_mut(event.lane() as usize) {
                    *n = n.saturating_add(1);
                }
            }
            if spawns_icon(event) {
                icons.push(time);
            }
            combo.push(combo_bonus(notes));
        }
        Some(Self { times, events, icons, combo, lanes, notes, stars })
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
    pub const fn stars(&self) -> u32 {
        self.stars
    }

    #[must_use]
    pub fn icons(&self) -> u32 {
        u32::try_from(self.icons.len()).unwrap_or(u32::MAX)
    }

    /// The tap time of each note carrying a rhythm icon.
    pub(super) fn icon_times(&self) -> &[Time] {
        &self.icons
    }

    /// This is used for Time skill triggers, believe it or not.
    /// Just used to not try to activate timer skills past the last note.
    #[must_use]
    pub fn last_judgement(&self) -> Time {
        self.times.iter().rev().nth(1).copied().unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::Queue;
    use crate::play::Scoring;
    use crate::play::tests::{arena, only, play, tap};
    use crate::song::{Event, EventKind, Time};

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
    fn check_lane_counts() {
        let stream: Vec<_> =
            (0_u32..).zip([0_u8, 3, 3, 8, 3]).map(|(i, l)| tap(2000 + i * 500, l)).collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        assert_eq!(queue.lanes(), [1, 0, 0, 3, 0, 0, 0, 0, 1]);
        assert_eq!(queue.lanes().iter().sum::<u32>(), queue.notes());
    }
}
