#![expect(clippy::tests_outside_test_module, reason = "integration test")]

//! Tests the whole pipeline of JSON -> Db -> archive -> mmap.

use std::path::PathBuf;

use simulator::accessory::AccessoryDb;
use simulator::archive::{self, Mapped};
use simulator::card::CardDb;
use simulator::sis::SisDb;
use simulator::song::{ChartRows, Difficulty, SongDb};

fn data() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../data"))
}

#[test]
fn check_cards_survive_the_archive() {
    let db = preprocess::cards(&data()).unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    assert_eq!(db.len(), 3826);

    let path = std::env::temp_dir().join("llsif-corpus-round-trip.rkyv");
    archive::write(&path, &db).expect("write");

    // SAFETY: `write` literally just produced this file and nothing else touches it.
    let mapped: Mapped<CardDb> = unsafe { Mapped::open(&path) }.expect("open");
    assert!(mapped.root() == &db, "the archive is not the database it was built from");
}

#[test]
fn check_accessories_survive_the_archive() {
    let cards = preprocess::cards(&data()).unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    let db = preprocess::accessories(&data(), &cards.roster)
        .unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    assert_eq!(db.len(), 549);

    let path = std::env::temp_dir().join("llsif-accessory-corpus-round-trip.rkyv");
    archive::write(&path, &db).expect("write");

    // SAFETY: `write` literally just produced this file and nothing else touches it.
    let mapped: Mapped<AccessoryDb> = unsafe { Mapped::open(&path) }.expect("open");
    assert!(mapped.root() == &db, "the archive is not the database it was built from");
}

#[test]
fn check_sis_survive_the_archive() {
    let cards = preprocess::cards(&data()).unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    let db =
        preprocess::sis(&data(), &cards.roster).unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    assert_eq!(db.len(), 546);

    let path = std::env::temp_dir().join("llsif-sis-corpus-round-trip.rkyv");
    archive::write(&path, &db).expect("write");

    // SAFETY: `write` literally just produced this file and nothing else touches it.
    let mapped: Mapped<SisDb> = unsafe { Mapped::open(&path) }.expect("open");
    assert!(mapped.root() == &db, "the archive is not the database it was built from");
}

#[test]
fn check_songs_survive_the_archive() {
    let cards = preprocess::cards(&data()).unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    let db = preprocess::songs(&data(), &cards.roster)
        .unwrap_or_else(|e| panic!("corpus is valid: {e}"));
    assert_eq!(db.len(), 1912);
    assert_eq!(db.songs.len(), 473);

    let path = std::env::temp_dir().join("llsif-song-corpus-round-trip.rkyv");
    archive::write(&path, &db).expect("write");

    // SAFETY: `write` literally just produced this file and nothing else touches it.
    let mapped: Mapped<SongDb> = unsafe { Mapped::open(&path) }.expect("open");
    assert!(mapped.root() == &db, "the archive is not the database it was built from");

    let root = mapped.root();
    let id = db.chart("Storm in Lover", Difficulty::Master).expect("a bop");
    let chart = root.charts.chart(id).expect("in range");
    let (times, events) = root.charts.stream(chart);
    assert_eq!(times.len(), events.len());
    assert!(times.windows(2).all(|w| w[0] <= w[1]));
}
