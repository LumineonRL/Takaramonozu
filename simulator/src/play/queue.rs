//! The queues of events, in charge of determining what event comes next.
//! other modules handle dispatching the event to its relevant handler.

use super::tap::{combo_bonus, fever_multiplier};
use crate::song::{Chart, ChartRows, Event, EventKind, LANES, Time};

/// Hold notes are two events but one combo. Actually they're 3 events
/// Regular notes are two events, a spawn and a press.
/// Hold notes are 3 notes, a hold spawn, a hold tap start, and a hold tap end
/// Combo increments on a regular tap or a hold release.
const fn scores(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldRelease)
}

/// Each `Queue` gets its own `Bucket`. There are many different queues which all
/// step in parallel and get processed a little bit differently.
pub(super) const BUCKET: u32 = 6;

const fn spawns_icon(event: Event) -> bool {
    matches!(event.kind(), EventKind::Tap | EventKind::HoldStart)
}

/// The big star of the show. This is the priority queue that drives the entire
/// simulation.
/// `repr(Rust)` was rearranging the fields in a way that was aversely affecting performance
/// with the addition of combo fever. freezing them helped performance.
/// TODO: Investigate further. Looking at this, it shouldn't be a huge deal because each field
/// is relatively the same size. Vec is 24 bytes, u32 is 4 bytes, [u32; lanes] is 36 bytes and
/// the largest field here. I just don't understand what the compiler could have done to make
/// a more sensible ordering slower. Quick mafs this is 8 aligned and adding everything up
/// is 196, padded to a multiple of 8 so it should be 200 bytes total.
/// Actually it comes out to 200 with both repr(Rust) and repr(C) so it's not a size issue.
/// It's probably cache line behavior. Everything at the beginning of the queue is hot and grouped together
/// combo fever is only relevant when combo fever cards are on the team.
/// repr(Rust) would force some of the combo fever fields to be brought in with more used
/// generic fields, hurting cache locality. That's the working theory at least.
#[derive(Clone, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Queue<'a> {
    icons: Vec<Time>,
    combo: Vec<u16>,
    times: &'a [Time],
    events: &'a [Event],
    lanes: [u32; LANES],
    notes: u32,
    stars: u32,
    /// Time of every note that actually increases combo.
    scored: Vec<Time>,
    /// Running total of combo fever multiplier.
    fever: Vec<u32>,
    buckets: Vec<u32>,
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
        let mut scored = Vec::with_capacity(events.len());
        let mut fever = Vec::with_capacity(events.len());
        let mut sum = 0_u32;
        fever.push(sum);
        for (&time, &event) in times.iter().zip(events) {
            if scores(event) {
                notes = notes.saturating_add(1);
                stars = stars.saturating_add(u32::from(event.is_star()));
                sum = sum.saturating_add(u32::from(fever_multiplier(notes)));
                scored.push(time);
                fever.push(sum);
                if let Some(n) = lanes.get_mut(event.lane() as usize) {
                    *n = n.saturating_add(1);
                }
            }
            if spawns_icon(event) {
                icons.push(time);
            }
            combo.push(combo_bonus(notes));
        }
        let buckets = buckets(&scored);
        Some(Self {
            icons,
            combo,
            times,
            events,
            lanes,
            notes,
            stars,
            scored,
            fever,
            buckets,
        })
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

    pub(super) fn buckets(&self) -> &[u32] {
        &self.buckets
    }

    pub(super) fn scored(&self) -> &[Time] {
        &self.scored
    }

    /// One longer than `scored`, starting at 0.
    pub(super) fn fever(&self) -> &[u32] {
        &self.fever
    }
}

/// This was originally in `fever.rs` but Spark needs it too, so moving it here.
/// Checks that a song hasn't ended because there's nothing to "`pay`" once a song is over
/// where "a song is over" is defined by "any ms greater than the highest ms in the chart."
#[inline(always)]
pub(super) fn reach(scored: &[Time], buckets: &[u32], from: usize, end: Time) -> usize {
    let bucket = ((end.0 >> BUCKET) as usize).min(buckets.len().saturating_sub(1));
    let mut to = buckets.get(bucket).map_or(from, |&n| (n as usize).max(from));
    while scored.get(to).is_some_and(|&t| t < end) {
        to = to.saturating_add(1);
    }
    to
}

fn buckets(scored: &[Time]) -> Vec<u32> {
    let last = scored.last().map_or(0, |t| t.0 >> BUCKET);
    let mut out = Vec::with_capacity(last as usize + 2);
    let mut note = 0_u32;
    for bucket in 0..=last.saturating_add(1) {
        let start = bucket.checked_shl(BUCKET).unwrap_or(u32::MAX);
        while scored.get(note as usize).is_some_and(|t| t.0 < start) {
            note = note.saturating_add(1);
        }
        out.push(note);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{BUCKET, Queue};
    use crate::play::Scoring;
    use crate::play::tap::fever_multiplier;
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

    #[test]
    fn check_the_fever_columns() {
        let stream = [
            tap(2000, 0),
            tap(2000, 1),
            (Time(2010), Event::new(2, EventKind::HoldStart, false, false)),
            tap(2063, 3),
            tap(2064, 4),
            (Time(2500), Event::new(2, EventKind::HoldRelease, false, false)),
            tap(9000, 5),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let times: Vec<u32> = queue.scored().iter().map(|t| t.0).collect();
        assert_eq!(times, [2000, 2000, 2063, 2064, 2500, 9000]);
        let mut sum = 0;
        let mut sums = vec![0];
        for n in 1..=queue.notes() {
            sum += u32::from(fever_multiplier(n));
            sums.push(sum);
        }
        assert_eq!(queue.fever(), sums);

        let buckets = queue.buckets();
        assert_eq!(buckets.len(), (9000 >> BUCKET) + 2);
        assert_eq!(buckets.last().copied(), Some(queue.notes()), "past the end");
        for (b, &first) in (0_u32..).zip(buckets) {
            let start = b << BUCKET;
            let want = times.iter().position(|&t| t >= start).unwrap_or(times.len());
            assert_eq!(first as usize, want, "bucket {b}");
        }
    }
}
