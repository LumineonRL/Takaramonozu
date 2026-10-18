//! Picks out what skills a team has on it so it can pre-compute values outside the hot trial loop
//! while ignoring skills that the team will never need.

use super::activation::Triggers;
use super::config::PlayConfig;
use super::effect::amp::Amp;
use super::effect::encore::{NoRepeat, Slot};
use super::effect::fever::Fever;
use super::effect::healing::Healing;
use super::effect::locks::{Expiry, Tricked};
use super::effect::param::Param;
use super::effect::psu::Psu;
use super::effect::sru::Sru;
use super::effect::{Instant, Lasting};
use super::judgement::Judgements;
use super::queue::Queue;
use super::walk;
use super::{Outcome, Scoring};

#[inline(never)]
pub(super) fn healing(
    queue: &Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome {
    if triggers.lasting() != 0 {
        return lasting::<Healing<Expiry>, Healing<Tricked>>(
            queue, config, scoring, triggers, judgements,
        );
    }
    effects::<Healing<Instant>>(queue, config, scoring, triggers, judgements)
}

/// `X` is the duration table. `T` the same but with Total Trick.
#[inline(never)]
pub(super) fn lasting<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    if (triggers.param() | triggers.sync()) != 0 {
        return appealing::<X, T>(queue, config, scoring, triggers, judgements);
    }
    boosting::<X, T>(queue, config, scoring, triggers, judgements)
}

/// Stat changes for Param Boost and Sync.
#[inline(never)]
fn appealing<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    boosting::<Param<X>, Param<T>>(queue, config, scoring, triggers, judgements)
}

/// Rate boosts, SRU.
fn boosting<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    if triggers.sru() != 0 {
        return rated::<X, T>(queue, config, scoring, triggers, judgements);
    }
    flat::<X, T>(queue, config, scoring, triggers, judgements)
}

#[inline(never)]
fn rated<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    flat::<Sru<X>, Sru<T>>(queue, config, scoring, triggers, judgements)
}

/// Flat bonuses, PSU and Combo Fever.
fn flat<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    if triggers.psu() != 0 {
        return perfect_score_up::<X, T>(queue, config, scoring, triggers, judgements);
    }
    if triggers.fever() != 0 {
        return durations::<Fever<X>, Fever<T>>(queue, config, scoring, triggers, judgements);
    }
    durations::<X, T>(queue, config, scoring, triggers, judgements)
}

#[inline(never)]
fn perfect_score_up<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    if triggers.fever() != 0 {
        return durations::<Fever<Psu<X>>, Fever<Psu<T>>>(
            queue, config, scoring, triggers, judgements,
        );
    }
    durations::<Psu<X>, Psu<T>>(queue, config, scoring, triggers, judgements)
}

fn durations<'q, X, T>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    X: Lasting<'q>,
    T: Lasting<'q>,
{
    // Total Trick getting its own copies measured to be faster. I kind of hate it really
    // belongs with regular PL.
    if triggers.tricks() != 0 {
        return effects::<T>(queue, config, scoring, triggers, judgements);
    }
    effects::<X>(queue, config, scoring, triggers, judgements)
}

pub(super) fn effects<'q, E>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    E: Lasting<'q>,
{
    // Encore always goes last
    if triggers.encore() != 0 {
        return repeated::<E>(queue, config, scoring, triggers, judgements);
    }
    if triggers.amp() != 0 {
        return amplified::<E>(queue, config, scoring, triggers, judgements);
    }
    years::<E>(queue, config, scoring, triggers, judgements)
}

#[inline(never)]
fn amplified<'q, E>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    E: Lasting<'q>,
{
    years::<Amp<E, NoRepeat>>(queue, config, scoring, triggers, judgements)
}

/// Encore teams get their own Amp copies
#[inline(never)]
fn repeated<'q, E>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    E: Lasting<'q>,
{
    years::<Amp<E, Slot>>(queue, config, scoring, triggers, judgements)
}

/// Year Group gets its own too but this one makes more sense I don't see a clean way around it.
fn years<'q, E>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    E: Lasting<'q>,
{
    if triggers.year() != 0 {
        return branch::<true, E>(queue, config, scoring, triggers, judgements);
    }
    branch::<false, E>(queue, config, scoring, triggers, judgements)
}

/// Only one card has a star note skill trigger and it's a very old SR that's not very good.
/// So this has const generics since, in almost every case, we can completely ignore star notes pre-trial.
#[inline(never)]
fn branch<'q, const YEAR: bool, E>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    E: Lasting<'q>,
{
    match (!triggers.stars().is_empty(), triggers.score() != 0) {
        (false, false) => {
            walk::<false, false, YEAR, E>(queue, config, scoring, triggers, judgements)
        }
        (false, true) => walk::<false, true, YEAR, E>(queue, config, scoring, triggers, judgements),
        (true, false) => {
            core::hint::cold_path();
            walk::<true, false, YEAR, E>(queue, config, scoring, triggers, judgements)
        }
        (true, true) => {
            core::hint::cold_path();
            walk::<true, true, YEAR, E>(queue, config, scoring, triggers, judgements)
        }
    }
}
