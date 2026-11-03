#![expect(
    clippy::print_stderr,
    reason = "it's just a tool for benchmarking it's not that serious bro"
)]

//! The Rust simulator's half of the A/B test against Burrito's sim.
//! Whatever Burrito's sim output is guided what gets output here.
//! See `bench/reference/runner.ts` as this file exists entirely to conform to that spec so
//! they can be directly compared.
//!
//! cargo run --release -p bench --bin runner -- <spec.json> <trials> [seed]

use core::time::Duration;
use std::env;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use bench::{Archives, Failed, load, root};
use serde::Serialize;
use simulator::play::Outcome;

const FIRST_SEED: u64 = 315;
const SEED_STEP: u64 = 1021;

#[derive(Serialize)]
struct Results<'a> {
    simulator: &'static str,
    spec: &'a str,
    chart: Chart<'a>,
    first_seed: u64,
    seed_step: u64,
    trials: usize,
    scores: Vec<u64>,
    tap: u64,
    skill: u64,
    timing: Timing,
}

#[derive(Serialize)]
struct Chart<'a> {
    title: &'a str,
    difficulty: bench::Level,
}

#[derive(Serialize)]
struct Timing {
    startup_ns: u64,
    trials_ns: u64,
}

fn main() -> ExitCode {
    match play() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("runner: {e}");
            ExitCode::FAILURE
        }
    }
}

fn play() -> Result<(), Failed> {
    let mut args = env::args().skip(1);
    let (Some(path), Some(count)) = (args.next(), args.next()) else {
        return Err("usage: runner <spec.json> <trials> [seed]".into());
    };
    let trials: usize = count.parse().map_err(|e| format!("bad trial count {count}: {e}"))?;
    let first_seed: u64 = match args.next() {
        Some(seed) => seed.parse().map_err(|e| format!("bad seed {seed}: {e}"))?,
        None => FIRST_SEED,
    };

    let started = Instant::now();
    let spec = load(Path::new(&path))?;
    let archives = Archives::open(&root())?;
    let team = archives.prepare(spec)?;
    let startup = started.elapsed();
    let mut outcomes: Vec<Outcome> = Vec::with_capacity(trials);
    let began = Instant::now();
    let mut seed = first_seed;
    for _ in 0..trials {
        outcomes.push(team.trial(seed));
        seed = seed.wrapping_add(SEED_STEP);
    }
    let elapsed = began.elapsed();

    let sum = |part: fn(&Outcome) -> u64| outcomes.iter().map(part).fold(0, u64::wrapping_add);
    let played = &team.spec;
    let results = Results {
        simulator: "Takaramonozu",
        spec: &played.name,
        chart: Chart {
            title: &played.chart.title,
            difficulty: played.chart.difficulty,
        },
        first_seed,
        seed_step: SEED_STEP,
        trials,
        scores: outcomes.iter().map(Outcome::score).collect(),
        tap: sum(|o| o.tap),
        skill: sum(|o| o.skill),
        timing: Timing { startup_ns: nanos(startup), trials_ns: nanos(elapsed) },
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, &results)?;
    writeln!(out)?;
    Ok(())
}

fn nanos(span: Duration) -> u64 {
    u64::try_from(span.as_nanos()).unwrap_or(u64::MAX)
}
