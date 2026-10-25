#![expect(clippy::expect_used, reason = "it's a benchmark so it's okay")]

//! Single threaded for now. Next todo will be to wrap it in rayon and loop
//! run a 1 thread and max thread version. idk if I'll do any threads in between
//! I frankly don't care how well this performs on say like, 8 out of 24 threads
//! I care about 1 thread to compare to the other one thread sim and 24 threads
//! because that's what my cpu has and what I will be using.
//!
//! cargo bench -p bench [fuzzy match on spec json title]

use std::sync::LazyLock;

use bench::{Archives, Prepared, catalog, root};
use divan::Bencher;
use divan::counter::ItemsCount;

static ARCHIVES: LazyLock<Archives> =
    LazyLock::new(|| Archives::open(&root()).expect("the archives"));

static TEAMS: LazyLock<Vec<Prepared<'static>>> = LazyLock::new(|| {
    let specs = catalog(&root().join("bench/specs")).expect("specs");
    specs.into_iter().map(|s| ARCHIVES.prepare(s).expect("playable spec")).collect()
});

fn names() -> impl Iterator<Item = &'static str> {
    TEAMS.iter().map(|t| t.spec.name.as_str())
}

fn main() {
    divan::main();
}

#[divan::bench(args = names())]
fn trial(bencher: Bencher, name: &str) {
    let team = TEAMS.iter().find(|t| t.spec.name == name).expect("team");
    let mut seed = 0_u64;
    bencher.counter(ItemsCount::new(1_usize)).bench_local(|| {
        seed = seed.wrapping_add(1);
        team.trial(seed)
    });
}
