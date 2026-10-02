//! Does a little bit more than card/acc/sis. This one also does a bit of additional processing.
//!
//! It takes the chart notes and makes an array of timings upfront, so that when a song is selected
//! at the beginning of a trial, it doesn't need to spend time rearranging the data.

use std::collections::HashMap;

use simulator::scalar::Roster;
use simulator::skill::Targets;
use simulator::song::{
    ChartArena, Difficulty, Event, EventKind, LANES, Song, SongDb, SongIdx, Time,
};

use crate::load::convert::{self, At, ErrorKind, LoadError};
use crate::load::raw::{RawSong, RawSongs, RawTargetMap};

struct Slot<'a> {
    song: SongIdx,
    difficulty: Difficulty,
    key: &'a str,
    raw: &'a RawSong,
}

/// `groups` comes from `additional_leader_skill_map.json` as the names used there are exactly the
/// same as the group names in songs.json.
///
/// # Errors
/// Any field that does not convert or a duplicate `(title, difficulty)` tuple.
///
/// # Panics
/// Past 65535 songs or charts
#[expect(clippy::panic_in_result_fn, reason = "capacity bounds, documented above")]
pub fn build(raw: &RawSongs, groups: &RawTargetMap, roster: &Roster) -> Result<SongDb, LoadError> {
    let mut titles = self::titles(raw)?;
    titles.sort_unstable_by_key(|t| t.title);
    assert!(u16::try_from(titles.len()).is_ok(), "{} songs do not fit a SongIdx", titles.len());
    assert!(u16::try_from(raw.len()).is_ok(), "{} charts do not fit a ChartIdx", raw.len());

    let mut targets = Targets::default();
    let mut songs = Vec::with_capacity(titles.len());
    for t in &titles {
        let group = t
            .song
            .group()
            .map(|g| targets.intern(g, || convert::character_set(g, groups, roster)))
            .transpose()
            .at(t.key, "group")?;
        songs.push(Song {
            title: t.title.into(),
            group,
            attribute: convert::attribute(&t.song.attribute).at(t.key, "attribute")?,
        });
    }

    let mut slots = self::slots(raw, &titles)?;

    slots.sort_unstable_by_key(key);
    if let Some([_, dup]) = slots.windows(2).find(|w| match w {
        [a, b] => key(a) == key(b),
        _ => false,
    }) {
        return Err(ErrorKind::Invariant).at(dup.key, "difficulty");
    }

    let mut charts = ChartArena::default();
    let mut stream = Vec::new();
    for slot in &slots {
        judgements(slot, &mut stream)?;
        charts.push(slot.song, slot.difficulty, &stream);
    }

    Ok(SongDb { songs, charts, groups: targets })
}

fn key(slot: &Slot<'_>) -> (u16, u8) {
    (slot.song.0, slot.difficulty as u8)
}

struct Distinct<'a> {
    title: &'a str,
    key: &'a str,
    song: &'a RawSong,
}

fn titles(raw: &RawSongs) -> Result<Vec<Distinct<'_>>, LoadError> {
    let mut seen: HashMap<&str, Distinct<'_>> = HashMap::with_capacity(raw.len());
    for (key, s) in raw {
        let first =
            seen.entry(s.title.as_str())
                .or_insert(Distinct { title: &s.title, key, song: s });
        if first.song.group != s.group || first.song.attribute != s.attribute {
            return Err(ErrorKind::Invariant).at(key.as_str(), "title");
        }
    }
    Ok(seen.into_values().collect())
}

fn slots<'a>(raw: &'a RawSongs, titles: &[Distinct<'a>]) -> Result<Vec<Slot<'a>>, LoadError> {
    let mut slots = Vec::with_capacity(raw.len());
    for (key, s) in raw {
        let i = titles
            .binary_search_by_key(&s.title.as_str(), |t| t.title)
            .ok()
            .and_then(|i| u16::try_from(i).ok())
            .ok_or(ErrorKind::Invariant)
            .at(key.as_str(), "title")?;
        slots.push(Slot {
            song: SongIdx(i),
            difficulty: convert::difficulty(&s.difficulty).at(key.as_str(), "difficulty")?,
            key,
            raw: s,
        });
    }
    Ok(slots)
}

/// Merge the note starts with the hold releases into one stream, sorted on
/// time.
///
/// Previously, I was sorting the charts by their lane. So if one chart had two notes at the
/// same millisecond in lanes 3 and 8 (3 listed first then 8) and another chart had two notes
/// also at the same millisecond in lanes 8 and 3 (8 first then 3), I was sorting them so it'd always
/// process 3, then 8. Burrito's reference sim does *not* do this — (yes, I typed an emdash)
/// they just process the notes in `songs.json` order. This can have a small effect on scores when
/// combo-tier thresholds are crossed. It doesn't *really* matter and *barely* changes the score, but
/// it's because it barely matters that I'd rather match burrito. Just helps me validate this one.
/// Reference sim I'm referring to: <https://github.com/NonSpicyBurrito/sif-team-simulator>
///
/// I'll note that this is a bit of an assumption being made here / divergence from actual
/// play. In practice, if two notes occur at the same time, the player may not deterministcally hit
/// one before the other. Sometimes they might hit the further left note first, sometimes further right.
/// Some players might always hit left first. Some players might always hit right first.
///
/// This *does* *very subtly* *sometimes* affect score, if the two notes would put the combo
/// counter over a threshold that increases the multiplier, and if the cards in the left/right lanes
/// don't have the same attribute/group bonus, then the order that they're tapped does impact
/// the resultant score.
///
/// But this is so minor and modelling it requires making strong assumption about the player's habits
/// which means it's just not worth adjusting. How did I write this much about something that
/// impacts scores by maybe like, 10 total points in a game where you score in the millions?
fn judgements(slot: &Slot<'_>, out: &mut Vec<(Time, Event)>) -> Result<(), LoadError> {
    out.clear();
    if slot.raw.notes.is_empty() {
        return Err(ErrorKind::Invariant).at(slot.key, "notes");
    }
    for n in &slot.raw.notes {
        let start = convert::time(n.start_time).at(slot.key, "start_time")?;
        let end = convert::time(n.end_time).at(slot.key, "end_time")?;
        if end < start {
            return Err(ErrorKind::Invariant).at(slot.key, "end_time");
        }
        let lane = n
            .position
            .checked_sub(1)
            .filter(|l| usize::from(*l) < LANES)
            .ok_or(ErrorKind::Invariant)
            .at(slot.key, "position")?;

        let hold = end > start;
        let kind = if hold {
            EventKind::HoldStart
        } else {
            EventKind::Tap
        };
        out.push((start, Event::new(lane, kind, n.is_star, n.is_swing)));
        if hold {
            out.push((end, Event::new(lane, EventKind::HoldRelease, n.is_star, n.is_swing)));
        }
    }

    out.sort_by_key(|&(time, _)| time);

    let collides = out.chunk_by(|a, b| a.0 == b.0).any(|same| {
        same.iter()
            .enumerate()
            .any(|(i, a)| same.iter().take(i).any(|b| b.1.lane() == a.1.lane()))
    });
    if collides {
        return Err(ErrorKind::Invariant).at(slot.key, "start_time");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use core::assert_matches;

    use simulator::scalar::Attribute;
    use simulator::song::{Chart, ChartRows};

    use super::*;
    use crate::load::raw::{self, RawCard};

    fn read(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/");
        std::fs::read(format!("{path}{name}")).expect("data file")
    }

    fn songs() -> RawSongs {
        raw::parse_songs(&read("songs.json")).expect("schema")
    }

    fn groups() -> RawTargetMap {
        raw::parse_target_map(&read("additional_leader_skill_map.json")).expect("schema")
    }

    fn roster() -> Roster {
        let cards: Vec<RawCard> = raw::parse(&read("cards.json")).expect("schema");
        Roster::build(cards.iter().map(|c| c.character.as_str()))
    }

    fn built() -> SongDb {
        build(&songs(), &groups(), &roster())
            .unwrap_or_else(|e| panic!("the song file is valid: {e}"))
    }

    #[test]
    fn check_tables_populate() {
        let db = built();
        assert_eq!(db.songs.len(), 473);
        assert_eq!(db.len(), 1912);
        assert_eq!(db.groups.len(), 4);
        assert!(db.songs.windows(2).all(|w| w[0].title < w[1].title));
    }

    #[test]
    fn check_the_file_key_order_does_not_reach_the_archive() {
        let db = built();
        let shuffled: RawSongs = songs()
            .into_iter()
            .map(|(key, s)| (format!("{:04}{key}", 9999 - key.len()), s))
            .collect();
        let other =
            build(&shuffled, &groups(), &roster()).unwrap_or_else(|e| panic!("same corpus: {e}"));

        assert!(other.songs == db.songs, "the song table depends on the file's key order");
        assert_eq!(other.charts.charts(), db.charts.charts());
        assert_eq!(other.charts.times(), db.charts.times());
        assert_eq!(other.charts.events(), db.charts.events());
    }

    #[test]
    fn check_merged_stream_is_whole_corpus() {
        let db = built();
        let events: usize = db.charts.charts().iter().map(Chart::judgements).sum();
        assert_eq!(events, 710_103, "636,085 notes plus 74,018 hold releases");
        assert_eq!(db.charts.times().len(), events + db.len(), "one terminator per chart");
        assert_eq!(db.charts.times().len(), db.charts.events().len());

        let longest = db.charts.charts().iter().map(Chart::judgements).max().expect("1912 charts");
        assert_eq!(longest, 1132);
    }

    #[test]
    fn check_the_opening_of_the_first_chart() {
        let db = built();
        let title = "Bokura no LIVE Kimi to no LIFE";
        let id = db.chart(title, Difficulty::Easy).expect("Live_0001");
        let chart = db.charts.chart(id).expect("in range");
        let (times, events) = db.charts.stream(chart);

        // A hold in lane 5 from 6.664 to 8.950, then a tap in lane 3 at 10.664.
        assert_eq!(times[..3], [Time(6664), Time(8950), Time(10_664)]);
        assert_eq!(events[0], Event::new(4, EventKind::HoldStart, false, false));
        assert_eq!(events[1], Event::new(4, EventKind::HoldRelease, false, false));
        assert_eq!(events[2], Event::new(2, EventKind::Tap, false, false));
        assert_eq!(events.last(), Some(&Event::END));

        let song = &db.songs[db.song(title).expect("present").0 as usize];
        assert_eq!(song.attribute, Attribute::Smile);
        let set = db.groups.set(song.group.expect("μ's")).expect("interned");
        assert_eq!(set.0.count_ones(), 9);
    }

    /// This chart has 2 notes at the same millisecond with the higher lane listed first.
    /// This is the thing that was fixed, where previously we were sorting the charts and always
    /// processing lower lanes in the event of a tie.
    #[test]
    fn check_a_millisecond_keeps_chart_order() {
        let db = built();
        let id = db.chart("Bokura no LIVE Kimi to no LIFE", Difficulty::Master).expect("present");
        let chart = db.charts.chart(id).expect("in range");
        let (times, events) = db.charts.stream(chart);
        let tied: Vec<Event> = times
            .iter()
            .zip(events)
            .filter(|&(&t, _)| t == Time(69_807))
            .map(|(_, &e)| e)
            .collect();
        assert_eq!(
            tied,
            [
                Event::new(6, EventKind::HoldRelease, false, false),
                Event::new(1, EventKind::Tap, false, false),
            ]
        );
    }

    /// Checks that the one chart in `songs.json` with out-of-order notes gets fixed properly
    #[test]
    fn check_the_out_of_order_chart_sorts() {
        let db = built();
        let id = db.chart("Omoiyo Hitotsuninare", Difficulty::Expert).expect("Live_s0607");
        let chart = db.charts.chart(id).expect("in range");
        let (times, _) = db.charts.stream(chart);
        assert!(times.windows(2).all(|w| w[0] <= w[1]), "the chart is still backwards");
        assert!(times.contains(&Time(69_580)) && times.contains(&Time(69_583)));
    }

    #[test]
    fn check_every_chart_round_trips() {
        let db = built();
        for s in songs().values() {
            let difficulty = convert::difficulty(&s.difficulty).expect("tier");
            let id = db.chart(&s.title, difficulty).expect("chart present");
            let chart = db.charts.chart(id).expect("in range");
            assert_eq!(chart.difficulty, difficulty);

            let holds = s.notes.iter().filter(|n| n.end_time > n.start_time).count();
            assert_eq!(chart.judgements(), s.notes.len() + holds, "{}", s.title);
        }
        assert!(db.chart("Snow Halation", Difficulty::Easy).is_some());
        assert_eq!(db.chart("Not A Song", Difficulty::Easy), None);
    }

    #[test]
    fn check_the_no_group_charts() {
        let db = built();
        let none = db.songs.iter().filter(|s| s.group.is_none()).count();
        assert_eq!(none, 26, "26 titles over 87 charts");

        let idx = db.song("Burn My Dread (Cool)").expect("a Persona 3 collab");
        let persona = &db.songs[idx.0 as usize];
        assert_eq!(persona.group, None);
        assert_eq!(persona.attribute, Attribute::Cool);
    }

    #[test]
    fn check_a_song_whose_charts_disagree_is_rejected() {
        let mut raw = songs();
        let key = raw.keys().find(|k| k.as_str() == "Live_0001.json").cloned().expect("borarara");
        raw.get_mut(&key).expect("present").attribute = "Cool".to_owned();
        let Err(err) = build(&raw, &groups(), &roster()) else {
            panic!("built anyway: a title carries one attribute")
        };
        assert_matches!(err, LoadError { field: "title", kind: ErrorKind::Invariant, culprit: _ });
    }

    #[test]
    fn check_a_lane_outside_the_nine_is_rejected() {
        let mut raw = songs();
        raw.get_mut("Live_0001.json").expect("present").notes[0].position = 10;
        let Err(err) = build(&raw, &groups(), &roster()) else {
            panic!("built anyway: lane 10 does not exist")
        };
        assert_matches!(
            err,
            LoadError { field: "position", kind: ErrorKind::Invariant, culprit: _ }
        );
    }
}
