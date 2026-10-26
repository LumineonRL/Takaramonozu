#![expect(clippy::expect_used, reason = "it's a benchmark so it's okay")]

//! Lower-level that controls the configuration of a single trial of a benchmark irrespective of how many
//! threads its running across.
//!
//! cargo bench -p bench --bench trial [fuzzy match on spec json title]

use std::sync::LazyLock;

use bench::{Archives, Prepared, catalog, root};
use divan::Bencher;
use divan::counter::ItemsCount;
use rayon::prelude::*;

/// Number of seeds
const BATCH: u64 = 50000;

static ARCHIVES: LazyLock<Archives> =
    LazyLock::new(|| Archives::open(&root()).expect("the archives"));

static TEAMS: LazyLock<Vec<Prepared<'static>>> = LazyLock::new(|| {
    let specs = catalog(&root().join("bench/specs")).expect("specs");
    specs.into_iter().map(|s| ARCHIVES.prepare(s).expect("playable spec")).collect()
});

fn names() -> impl Iterator<Item = &'static str> {
    TEAMS.iter().map(|t| t.spec.name.as_str())
}

fn team(name: &str) -> &'static Prepared<'static> {
    TEAMS.iter().find(|t| t.spec.name == name).expect("team")
}

fn main() {
    divan::main();
}

#[divan::bench(args = names())]
fn one_thread(bencher: Bencher, name: &str) {
    let team = team(name);
    let mut seed = 0_u64;
    bencher.counter(ItemsCount::new(1_usize)).bench_local(|| {
        seed = seed.wrapping_add(1);
        team.trial(seed)
    });
}

#[divan::bench(args = names())]
fn all_threads(bencher: Bencher, name: &str) {
    let team = team(name);
    let mut first = 0_u64;
    bencher.counter(ItemsCount::new(BATCH)).bench_local(|| {
        let seeds = first..first.wrapping_add(BATCH);
        first = first.wrapping_add(BATCH);
        seeds
            .into_par_iter()
            .map(|seed| team.trial(seed).score())
            .reduce(|| 0, u64::wrapping_add)
    });
}
