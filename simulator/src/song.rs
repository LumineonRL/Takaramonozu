use rkyv::{Archive, Deserialize, Serialize};

use crate::archive::{Kind, portable};
use crate::scalar::{Attribute, CharacterSet};
use crate::skill::{self, TargetId, Targets};

pub const LANES: usize = 9;

/// Transcription of `note_speed_map.json`
pub const NOTE_SPEEDS: [u32; 10] = [1750, 1600, 1450, 1300, 1150, 1000, 900, 800, 700, 600];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct SongIdx(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct ChartIdx(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
#[repr(transparent)]
pub struct Time(pub u32);

impl Time {
    pub const SECOND: u32 = 1000;

    /// Tap time - note speed offset = spawn time.
    #[inline]
    #[must_use]
    pub const fn spawn(self, offset: u32) -> Self {
        Self(self.0.saturating_sub(offset))
    }

    #[must_use]
    #[expect(clippy::integer_division, reason = "game mechanic")]
    pub const fn seconds(self) -> u32 {
        self.0 / Self::SECOND
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Difficulty {
    Easy = 0,
    Normal = 1,
    Hard = 2,
    Expert = 3,
    Master = 4,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum EventKind {
    Tap = 0,
    HoldStart = 1,
    HoldRelease = 2,
    /// An artificial terminal event that will signal when the song is over.
    /// `push` puts it a ms past the final judgement; what's unverified is
    /// whether the real game agrees the song ends there. `songs.json` has no
    /// song length, so a Time skill firing past the last note is not modelled.
    End = 3,
}

/// Took some liberties here since star + swing never occur together,
/// star + hold never occur together, and apparently swing + hold never occur together???
/// (Maybe it's only SIF2 charts that have swings that end in holds?)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct Event(u8);

impl Event {
    const LANE: u8 = 0b0000_1111;
    const KIND: u8 = 0b0011_0000;
    const STAR: u8 = 0b0100_0000;
    const SWING: u8 = 0b1000_0000;

    const KIND_BITS: [u8; 4] = [0b0000_0000, 0b0001_0000, 0b0010_0000, 0b0011_0000];

    pub const END: Self = Self(Self::KIND);

    /// Made `lane` 0-based, i.e. the file's `position` minus one.
    ///
    /// # Panics
    /// Past lane 8.
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`EventKind` has exactly these 4 variants")]
    pub const fn new(lane: u8, kind: EventKind, star: bool, swing: bool) -> Self {
        assert!((lane as usize) < LANES, "lane is 0-based, so 0..=8");
        let mut bits = lane | Self::KIND_BITS[kind as usize];
        if star {
            bits |= Self::STAR;
        }
        if swing {
            bits |= Self::SWING;
        }
        Self(bits)
    }

    #[inline]
    #[must_use]
    pub const fn lane(self) -> u8 {
        self.0 & Self::LANE
    }

    #[inline]
    #[must_use]
    pub const fn kind(self) -> EventKind {
        match self.0 & Self::KIND {
            0b0000_0000 => EventKind::Tap,
            0b0001_0000 => EventKind::HoldStart,
            0b0010_0000 => EventKind::HoldRelease,
            _ => EventKind::End,
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_star(self) -> bool {
        (self.0 & Self::STAR) != 0
    }

    #[inline]
    #[must_use]
    pub const fn is_swing(self) -> bool {
        (self.0 & Self::SWING) != 0
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct Song {
    pub title: Box<str>,
    pub group: Option<TargetId>,
    pub attribute: Attribute,
}

/// For checking if the song's attribute/group matches each card's on a team.
/// I originally figured that the simulator would own bonuses like this and that this file would only
/// deal with the data representation and I personally still don't like this approach but my tests
/// show that it's much more performant to have the song itself own it and only modify the masks at trial start,
/// at least from what I tried.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Matching {
    pub attribute: Attribute,
    pub group: Option<CharacterSet>,
}

impl Matching {
    #[must_use]
    pub fn new(attribute: Attribute, group: Option<TargetId>, groups: &Targets) -> Option<Self> {
        let group = match group {
            None => None,
            Some(id) => Some(groups.set(id)?),
        };
        Some(Self { attribute, group })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Chart {
    start: u32,
    len: u32,
    pub song: SongIdx,
    pub difficulty: Difficulty,
    _pad: u8,
}

impl Chart {
    #[must_use]
    pub const fn judgements(&self) -> usize {
        self.len.saturating_sub(1) as usize
    }
}

#[derive(Default, Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct ChartArena {
    charts: Vec<Chart>,
    times: Vec<Time>,
    events: Vec<Event>,
}

impl ChartArena {
    /// Notes on the same exact millisecond keep the order they spawned in
    ///
    /// # Panics
    /// Empty note chart, times out of order, two notes in one lane at one
    /// millisecond, more than `u32::MAX` events, more than 65535 charts.
    #[expect(clippy::expect_used, reason = "capacity bounds, documented above")]
    pub fn push(
        &mut self,
        song: SongIdx,
        difficulty: Difficulty,
        judgements: &[(Time, Event)],
    ) -> ChartIdx {
        let &(last, _) = judgements
            .last()
            .expect("a chart with no judgements would read the next chart's");

        let mut prev: Option<(Time, u16)> = None;
        for &(time, event) in judgements {
            assert!(event.kind() != EventKind::End, "`push` appends the terminator itself");
            let bit = 1_u16.checked_shl(u32::from(event.lane())).expect("a lane is under 16");
            let seen = match prev {
                Some((at, seen)) if at == time => {
                    assert!(seen & bit == 0, "two records share a lane and a millisecond");
                    seen | bit
                }
                Some((at, _)) => {
                    assert!(at < time, "the stream is not sorted on time");
                    bit
                }
                None => bit,
            };
            prev = Some((time, seen));
        }

        // Naively the song is over when its final note is processed, but a small
        // epsilon is needed so that skills can proc off that note.
        let end = last.0.checked_add(1).expect("a chart ends well short of u32::MAX ms");
        let len = u32::try_from(judgements.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .expect("one chart fits u32");

        let chart = Chart {
            start: u32::try_from(self.times.len()).expect("stream arena fits u32"),
            len,
            song,
            difficulty,
            _pad: 0,
        };
        for &(time, event) in judgements {
            self.times.push(time);
            self.events.push(event);
        }
        self.times.push(Time(end));
        self.events.push(Event::END);

        let id = u16::try_from(self.charts.len()).expect("chart table fits u16");
        self.charts.push(chart);
        ChartIdx(id)
    }
}

pub trait ChartRows {
    fn charts(&self) -> &[Chart];
    fn times(&self) -> &[Time];
    fn events(&self) -> &[Event];

    #[must_use]
    fn chart(&self, id: ChartIdx) -> Option<&Chart> {
        self.charts().get(id.0 as usize)
    }

    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "`push` allocated `len` at `start`")]
    #[expect(clippy::arithmetic_side_effects, reason = "u32 + u32; see `archive`'s usize assert")]
    fn stream(&self, chart: &Chart) -> (&[Time], &[Event]) {
        let range = chart.start as usize..chart.start as usize + chart.len as usize;
        (&self.times()[range.clone()], &self.events()[range])
    }
}

impl ChartRows for ChartArena {
    fn charts(&self) -> &[Chart] {
        &self.charts
    }

    fn times(&self) -> &[Time] {
        &self.times
    }

    fn events(&self) -> &[Event] {
        &self.events
    }
}

impl ChartRows for ArchivedChartArena {
    fn charts(&self) -> &[Chart] {
        &self.charts
    }

    fn times(&self) -> &[Time] {
        &self.times
    }

    fn events(&self) -> &[Event] {
        &self.events
    }
}

#[derive(Archive, Serialize, Deserialize)]
#[rkyv(compare(PartialEq))]
pub struct SongDb {
    pub songs: Vec<Song>,
    pub charts: ChartArena,
    pub groups: skill::Targets,
}

impl Kind for SongDb {
    /// `LLSIFSDB` is the SIS database, so songs take `L` for live.
    const MAGIC: [u8; 8] = *b"LLSIFLDB";
    const VERSION: u32 = 3;
}

impl SongDb {
    /// Songs have an id but we need a way to search a song by `(song, difficulty)`
    #[must_use]
    pub fn song(&self, title: &str) -> Option<SongIdx> {
        let i = self.songs.binary_search_by(|s| (*s.title).cmp(title)).ok()?;
        Some(SongIdx(u16::try_from(i).ok()?))
    }

    /// Charts are sorted by `(song, difficulty)`, so this keys on the pair
    /// rather than asking [`Difficulty`] for an `Ord` it has no other use for.
    #[must_use]
    pub fn chart(&self, title: &str, difficulty: Difficulty) -> Option<ChartIdx> {
        let song = self.song(title)?;
        let key = (song.0, difficulty as u8);
        let i = self
            .charts
            .charts
            .binary_search_by_key(&key, |c| (c.song.0, c.difficulty as u8))
            .ok()?;
        Some(ChartIdx(u16::try_from(i).ok()?))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.charts.charts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.charts.charts.is_empty()
    }
}

portable!(SongIdx, ChartIdx, Time, Difficulty, Event, Chart);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{Mapped, write};

    /// The opening of `Live_0001.json`, `Bokura no LIVE Kimi to no LIFE` Easy.
    /// A hold in lane 5 from 6.664 to 8.950, then a tap in lane 3 at 10.664.
    fn stream() -> Vec<(Time, Event)> {
        vec![
            (Time(6664), Event::new(4, EventKind::HoldStart, false, false)),
            (Time(8950), Event::new(4, EventKind::HoldRelease, false, false)),
            (Time(10664), Event::new(2, EventKind::Tap, false, false)),
        ]
    }

    fn built() -> SongDb {
        let mut charts = ChartArena::default();
        charts.push(SongIdx(0), Difficulty::Easy, &stream());
        charts.push(SongIdx(0), Difficulty::Master, &stream());
        SongDb {
            songs: vec![Song {
                title: "Bokura no LIVE Kimi to no LIFE".into(),
                group: None,
                attribute: Attribute::Smile,
            }],
            charts,
            groups: skill::Targets::default(),
        }
    }

    #[test]
    fn check_stream_record_five_bytes() {
        assert_eq!(size_of::<Time>(), 4);
        assert_eq!(size_of::<Event>(), 1);
        assert_eq!(size_of::<rkyv::Archived<Time>>(), 4);
        assert_eq!(size_of::<rkyv::Archived<Event>>(), 1);
    }

    #[test]
    fn check_chart_nothing_uninitialized() {
        assert_eq!(size_of::<Chart>(), 12);
        assert_eq!(size_of::<rkyv::Archived<Chart>>(), 12);
    }

    #[test]
    fn check_the_packed_byte_round_trips_all_combos() {
        let kinds = [
            EventKind::Tap,
            EventKind::HoldStart,
            EventKind::HoldRelease,
            EventKind::End,
        ];
        for lane in 0..u8::try_from(LANES).expect("9 lanes") {
            for kind in kinds {
                for star in [false, true] {
                    for swing in [false, true] {
                        let e = Event::new(lane, kind, star, swing);
                        assert_eq!(e.lane(), lane, "{e:?}");
                        assert_eq!(e.kind(), kind, "{e:?}");
                        assert_eq!(e.is_star(), star, "{e:?}");
                        assert_eq!(e.is_swing(), swing, "{e:?}");
                    }
                }
            }
        }
        assert_eq!(Event::END, Event::new(0, EventKind::End, false, false));
    }

    #[test]
    fn check_the_terminator_lands_one_ms_past_the_last_judgement() {
        let mut arena = ChartArena::default();
        arena.push(SongIdx(0), Difficulty::Easy, &stream());
        let chart = arena.chart(ChartIdx(0)).expect("just pushed");
        let (times, events) = arena.stream(chart);

        assert_eq!(times.last(), Some(&Time(10665)));
        assert_eq!(events.last(), Some(&Event::END));
        assert!(Time(10665) < Time(15_000));
        assert!(Time(9000) < Time(10665));
    }

    #[test]
    fn check_the_earliest_spawn_in_the_corpus_does_not_underflow() {
        assert_eq!(Time(2047).spawn(NOTE_SPEEDS[0]), Time(297));
    }

    #[test]
    #[should_panic(expected = "not sorted on time")]
    fn check_an_unsorted_stream_fails() {
        // `Live_s0607` is written 69.583 then 69.580, out of order notes.
        let out_of_order = vec![
            (Time(69_583), Event::new(0, EventKind::HoldRelease, false, false)),
            (Time(69_580), Event::new(1, EventKind::HoldRelease, false, false)),
        ];
        ChartArena::default().push(SongIdx(0), Difficulty::Expert, &out_of_order);
    }

    #[test]
    fn check_a_millisecond_keeps_the_order_it_arrives_in() {
        // A higher lane before a lower one
        let chart_order = [
            (Time(69_807), Event::new(6, EventKind::HoldRelease, false, false)),
            (Time(69_807), Event::new(1, EventKind::Tap, false, false)),
        ];
        let mut arena = ChartArena::default();
        let id = arena.push(SongIdx(0), Difficulty::Master, &chart_order);
        let (_, events) = arena.stream(arena.chart(id).expect("just pushed"));
        assert_eq!(events[..2].iter().map(|e| e.lane()).collect::<Vec<_>>(), [6, 1]);
    }

    #[test]
    #[should_panic(expected = "share a lane and a millisecond")]
    fn check_one_lane_twice_in_a_millisecond_fails() {
        let twice = [
            (Time(69_807), Event::new(1, EventKind::Tap, false, false)),
            (Time(69_807), Event::new(1, EventKind::Tap, false, false)),
        ];
        ChartArena::default().push(SongIdx(0), Difficulty::Master, &twice);
    }

    #[test]
    fn check_columns_of_a_chart_align() {
        let db = built();
        let easy = db.chart("Bokura no LIVE Kimi to no LIFE", Difficulty::Easy);
        assert_eq!(easy, Some(ChartIdx(0)));
        assert_eq!(db.chart("Bokura no LIVE Kimi to no LIFE", Difficulty::Hard), None);
        assert_eq!(db.chart("Snow Halation", Difficulty::Easy), None);

        let chart = db.charts.chart(ChartIdx(1)).expect("just pushed");
        let (times, events) = db.charts.stream(chart);
        assert_eq!(times.len(), events.len());
        assert_eq!(chart.judgements(), 3, "the terminator is not a judgement");
        assert_eq!(times.len(), 4, "but it is in the stream");
        assert_eq!(times.first(), Some(&Time(6664)));
        assert_eq!(events.last(), Some(&Event::END));
        // The second chart's range must not start at the first one's.
        assert_eq!(events.first().copied().map(Event::kind), Some(EventKind::HoldStart));
    }

    #[test]
    fn check_the_song_db_survives_the_archive() {
        let db = built();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("llsif-song-{pid}.rkyv"));
        write(&path, &db).expect("write");
        // SAFETY: `write` just produced this file and nothing else touches it.
        let mapped: Mapped<SongDb> = unsafe { Mapped::open(&path) }.expect("open");
        assert!(mapped.root() == &db, "the archive is not the database it was built from");

        let root = mapped.root();
        let chart = root.charts.chart(ChartIdx(0)).expect("in range");
        let (times, events) = root.charts.stream(chart);
        assert_eq!(times, &[Time(6664), Time(8950), Time(10664), Time(10665)]);
        assert_eq!(events.last(), Some(&Event::END));

        drop(mapped);
        std::fs::remove_file(&path).expect("cleanup");
    }
}
