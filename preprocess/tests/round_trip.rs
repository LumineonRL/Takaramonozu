#![expect(clippy::tests_outside_test_module, reason = "integration test")]

//! Tests the whole pipeline of JSON -> `CardDb` -> archive -> mmap.

use std::path::PathBuf;

use simulator::accessory::AccessoryDb;
use simulator::archive::{self, Mapped};
use simulator::card::CardDb;

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
