#![expect(clippy::expect_used, reason = "it's a benchmark so it's okay")]

/// Higher level orchestrator of trial.rs. For benchmarking on different parts of the simulation process.
/// Basically splits the sim process into several components and benchmarks each one individually.
use std::sync::LazyLock;

use bench::{Archives, Prepared, catalog, root};
use divan::Bencher;
use simulator::play::activation::Triggers;
use simulator::play::config::PlayConfig;
use simulator::play::judgement::Judgements;

const TABLES: u64 = 64;

static ARCHIVES: LazyLock<Archives> = LazyLock::new(|| Archives::open(&root()).expect("rkyvs"));

static TEAMS: LazyLock<Vec<Prepared<'static>>> = LazyLock::new(|| {
    let specs = catalog(&root().join("bench/specs")).expect("specs");
    specs.into_iter().map(|s| ARCHIVES.prepare(s).expect("valid spec")).collect()
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
fn queue(bencher: Bencher, name: &str) {
    let team = team(name);
    bencher.bench_local(|| ARCHIVES.queue(&team.chart));
}

#[divan::bench(args = names())]
fn compile(bencher: Bencher, name: &str) {
    let team = team(name);
    bencher.bench_local(|| ARCHIVES.compile(&team.built));
}

#[divan::bench(args = names())]
fn triggers(bencher: Bencher, name: &str) {
    let team = team(name);
    bencher.bench_local(|| Triggers::build(&team.queue, &team.scoring));
}

#[divan::bench(args = names())]
fn rebuild(bencher: Bencher, name: &str) {
    let team = team(name);
    let mut triggers = Triggers::build(&team.queue, &team.scoring);
    bencher.bench_local(|| triggers.rebuild(&team.queue, &team.scoring));
}

#[divan::bench(args = names())]
fn judgements(bencher: Bencher, name: &str) {
    let team = team(name);
    let mut seed = 0_u64;
    bencher.bench_local(|| {
        seed = seed.wrapping_add(1);
        Judgements::build(&team.queue, &PlayConfig { seed, ..team.config })
    });
}

#[divan::bench(args = names())]
fn run(bencher: Bencher, name: &str) {
    let team = team(name);
    let tables: Vec<(PlayConfig, Judgements)> = (0..TABLES)
        .map(|seed| {
            let config = PlayConfig { seed, ..team.config };
            (config, Judgements::build(&team.queue, &config))
        })
        .collect();
    let mut next = tables.iter().cycle();
    bencher.bench_local(|| {
        let (config, table) = next.next().expect("cycle");
        simulator::play::run(&team.queue, config, &team.scoring, &team.triggers, table)
    });
}
