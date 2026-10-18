//! Orchestrator of one playthrough of one chart with one team.

pub mod activation;
pub mod config;
mod dispatch;
mod effect;
pub mod judgement;
pub mod queue;
pub mod rng;
pub mod tap;

use crate::scalar::Ms;
use crate::song::{EventKind, LANES, Time};
use activation::chain::Chain;
use activation::ladder::Ladder;
use activation::{AMP_LEVELS, AccessorySkill, Payload, Skill, Tally, Triggers, fire};
use config::PlayConfig;
use dispatch::{effects, healing, lasting};
use effect::locks::{Expiry, Tricked};
use effect::{Instant, Lasting};
use judgement::Judgements;
use queue::Queue;
use rng::Rng;
use tap::{JUDGEMENT, UNIT, note_eighths, note_multiplier, note_score, overheal_score};

/// `repr(C)` in the order rustc laid it out before Sync, so the fields every `walk` copy
/// reads kept their offsets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Scoring {
    pub skills: [Skill; LANES],
    pub accessories: [AccessorySkill; LANES],
    pub team_strength: u32,
    pub trick: u32,
    pub durations: [[Ms; 2]; LANES],
    pub lanes: [u16; LANES],
    /// For Year-Group triggers.
    /// One bit for each lane's character. The bit of the first lane that character is in.
    pub members: [u16; LANES],
    /// HP each lane's card and accessory heal. 0 unless a Healer.
    pub heals: [[u8; 2]; LANES],
    /// Tap bonus per heart
    pub per_heart: u16,
    pub max_hearts: u16,
    pub max_hp: u8,
    /// Each lane's stat in the song's attribute, after the leader skill. They sum to `team_strength`.
    pub stats: [u32; LANES],
    /// The lanes an Appeal Boost boosts or a Sync may copy, card then accessory.
    pub targets: [[u16; 2]; LANES],
    /// An Appeal Boost's percent, card then accessory.
    pub boosts: [[u16; 2]; LANES],
    /// Card then accessory, at own level + 1..=16 for an Amplify.
    pub amplified: [[[Payload; AMP_LEVELS]; 2]; LANES],
}

impl Scoring {
    #[must_use]
    pub const fn flat(team_strength: u32) -> Self {
        Self {
            skills: [Skill::NONE; LANES],
            accessories: [AccessorySkill::NONE; LANES],
            team_strength,
            trick: 0,
            durations: [[0; 2]; LANES],
            lanes: [UNIT; LANES],
            members: [0; LANES],
            heals: [[0; 2]; LANES],
            per_heart: 0,
            max_hearts: 0,
            max_hp: 0,
            stats: [0; LANES],
            targets: [[0; 2]; LANES],
            boosts: [[0; 2]; LANES],
            amplified: [[[Payload { value: 0, duration: 0, extra: 0 }; AMP_LEVELS]; 2]; LANES],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Outcome {
    pub tap: u64,
    pub skill: u64,
    pub notes: u32,
    pub icons: u32,

    /// Not to be confused with the perfect judgement. Hold notes require both
    /// the head and tail or a hold to be perfect, else its a great.
    pub perfects: u32,
    pub activations: u32,
    pub accessory_activations: u32,
}

impl Outcome {
    #[must_use]
    pub const fn score(&self) -> u64 {
        self.tap.saturating_add(self.skill)
    }
}

#[must_use]
pub fn run(
    queue: &Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome {
    if triggers.heals() != 0 {
        return healing(queue, config, scoring, triggers, judgements);
    }
    if triggers.lasting() != 0 {
        return lasting::<Expiry, Tricked>(queue, config, scoring, triggers, judgements);
    }
    effects::<Instant>(queue, config, scoring, triggers, judgements)
}

#[expect(clippy::indexing_slicing, reason = "I'll allow it.")]
#[expect(clippy::else_if_without_else, reason = "lazy")]
#[expect(clippy::too_many_lines, reason = "Benchmarked. Splitting it moved what LLVM inlined.")]
#[inline(never)]
fn walk<'q, const STARS: bool, const SCORE: bool, const YEAR: bool, E>(
    queue: &'q Queue,
    config: &PlayConfig,
    scoring: &Scoring,
    triggers: &Triggers,
    judgements: &Judgements,
) -> Outcome
where
    E: Lasting<'q>,
{
    debug_assert_eq!(judgements.records(), queue.events().len(), "judgements of another chart");
    debug_assert_eq!(judgements.seed(), config.seed, "judgements of another trial");

    let offset = config.speed.offset();
    let tricked = scoring.team_strength.saturating_add(scoring.trick);
    let mut out = Outcome::default();
    let mut rng = Rng::new(config.seed, Rng::SKILLS);
    let mut heads = [false; LANES];
    let mut tap = 0_usize;
    let (icon_at, icon_lanes) = triggers.icons().columns();
    let (timer_at, timer_lanes) = triggers.time().columns();
    let mut spawns = Spawns {
        icons: queue.icon_times(),
        at: icon_at,
        lanes: icon_lanes,
        icon: 0,
        cursor: 0,
        next: icon_at[0] as usize,
    };
    let mut combo = Tally::new(triggers.combo());
    let mut perfects = Tally::new(triggers.perfects());
    let mut stars = Tally::new(triggers.stars());
    let mut star_perfects = 0_u32;
    let mut expiry = E::new(queue.last_judgement(), queue, scoring);
    if E::PARAM || E::AMP {
        expiry.seed(config.seed);
    }
    if E::FEVER && SCORE {
        expiry.fever_per_note();
    }
    let mut timer_cursor = 0_usize;
    let mut next_timer = Time(timer_at[0]);
    let mut ladder = Ladder::new(scoring, if SCORE { triggers.score() } else { 0 });
    let mut chain = Chain::new(if YEAR { triggers.year() } else { 0 });

    loop {
        let time = queue.times()[tap];

        // Spawns and taps processed before time. Actually unsure if this is how
        // it works in-game, but it is how burrito's sim does it.
        // Duration skills follow the same principle, actually, which gives some
        // credence to that assumption.
        while next_timer.min(expiry.next()) <= time {
            // timer skills really don't roll very often
            core::hint::cold_path();
            let at = next_timer.min(expiry.next());
            spawns.drain::<SCORE, YEAR, false, E>(
                at,
                offset,
                scoring,
                &mut rng,
                &mut out,
                &mut ladder,
                &mut chain,
                &mut expiry,
            );
            if E::ANY && expiry.next() <= at {
                (rng, out) = release::<SCORE, YEAR, _>(
                    at,
                    scoring,
                    rng,
                    out,
                    &mut ladder,
                    &mut chain,
                    &mut expiry,
                );
                continue;
            }
            if SCORE {
                ladder.catch_up::<YEAR>(at, scoring, &mut rng, &mut out, &mut chain, &mut expiry);
            }
            moment::<YEAR>(
                timer_lanes[timer_cursor],
                at,
                scoring,
                &mut rng,
                &mut out,
                &mut chain,
                &mut expiry,
            );
            timer_cursor = timer_cursor.saturating_add(1);
            next_timer = Time(timer_at[timer_cursor]);
            if SCORE {
                ladder.settle::<YEAR>(at, scoring, &mut rng, &mut out, &mut chain, &mut expiry);
            }
        }
        spawns.drain::<SCORE, YEAR, true, E>(
            time,
            offset,
            scoring,
            &mut rng,
            &mut out,
            &mut ladder,
            &mut chain,
            &mut expiry,
        );

        let event = queue.events()[tap];
        let lane = event.lane() as usize;
        match event.kind() {
            EventKind::End => break,
            EventKind::HoldStart => {
                if let Some(head) = heads.get_mut(lane) {
                    *head = judgements.at(tap);
                }
            }

            kind @ (EventKind::Tap | EventKind::HoldRelease) => {
                if SCORE {
                    ladder.catch_up::<YEAR>(
                        time,
                        scoring,
                        &mut rng,
                        &mut out,
                        &mut chain,
                        &mut expiry,
                    );
                }
                let tail = judgements.at(tap);
                let head = (kind == EventKind::Tap) | heads.get(lane).copied().unwrap_or(true);
                let converted = E::ANY && expiry.converted(time);
                let locked = E::ANY && expiry.locked(time);
                out.tap = out.tap.saturating_add(if E::TRICKS {
                    expiry.total_trick_tap(
                        time,
                        head,
                        tail,
                        queue.combo()[tap],
                        scoring.lanes.get(lane).copied().unwrap_or(UNIT),
                        if E::HEARTS {
                            note_eighths(event)
                        } else {
                            note_multiplier(event)
                        },
                    )
                } else if E::HEARTS {
                    overheal_score(
                        if E::PARAM {
                            expiry.param_hearted_strength(locked)
                        } else {
                            expiry.heart_strength(locked)
                        },
                        queue.combo()[tap],
                        scoring.lanes.get(lane).copied().unwrap_or(UNIT),
                        note_eighths(event),
                        JUDGEMENT[usize::from(locked)][usize::from(head)][usize::from(tail)],
                    )
                } else {
                    note_score(
                        if E::PARAM {
                            expiry.param_strength(locked)
                        } else if locked {
                            tricked
                        } else {
                            scoring.team_strength
                        },
                        queue.combo()[tap],
                        scoring.lanes.get(lane).copied().unwrap_or(UNIT),
                        note_multiplier(event),
                        JUDGEMENT[usize::from(locked)][usize::from(head)][usize::from(tail)],
                    )
                });
                out.notes = out.notes.saturating_add(1);
                if E::FEVER && SCORE {
                    out.skill = out.skill.saturating_add(expiry.fever_tap(time, out.notes));
                }
                let perfect = converted || (head & tail);
                out.perfects = out.perfects.saturating_add(u32::from(perfect));
                if E::PSU && SCORE {
                    out.skill = out.skill.saturating_add(expiry.psu_tap(perfect));
                }

                // Shoutouts to Snow Maiden Umi. I wonder why they gave a skill to one card and then just never again.
                // I wonder why. Could've been interesting.
                if STARS & event.is_star() & perfect {
                    core::hint::cold_path();
                    star_perfects = star_perfects.saturating_add(1);
                    let due = stars.due(star_perfects)
                        | combo.due(out.notes)
                        | perfects.due(out.perfects);
                    hit::<SCORE, YEAR>(
                        due,
                        time,
                        scoring,
                        &mut rng,
                        &mut out,
                        &mut ladder,
                        &mut chain,
                        &mut expiry,
                    );
                } else if out.notes == combo.next()
                    || out.perfects == perfects.next()
                    || (SCORE && out.score() >= ladder.low())
                {
                    let due = combo.due(out.notes) | perfects.due(out.perfects);
                    hit::<SCORE, YEAR>(
                        due,
                        time,
                        scoring,
                        &mut rng,
                        &mut out,
                        &mut ladder,
                        &mut chain,
                        &mut expiry,
                    );
                }
            }
        }

        tap = tap.saturating_add(1);
    }

    out.icons = u32::try_from(spawns.icon).unwrap_or(u32::MAX);
    if E::FEVER && !SCORE {
        out.skill = out.skill.saturating_add(expiry.fever_paid());
    }
    if E::PSU && !SCORE {
        out.skill = out.skill.saturating_add(expiry.psu_paid(out.perfects));
    }
    out
}

#[inline(always)]
#[expect(
    clippy::inline_always,
    reason = "I benchmarked and it didn't get inlined with regular inline."
)]
#[expect(
    clippy::too_many_arguments,
    reason = "don't feel like splitting it up. Not even sure if that would affect performance."
)]
fn hit<'q, const SCORE: bool, const YEAR: bool>(
    due: u16,
    now: Time,
    scoring: &Scoring,
    rng: &mut Rng,
    out: &mut Outcome,
    ladder: &mut Ladder,
    chain: &mut Chain,
    expiry: &mut impl Lasting<'q>,
) {
    // tldr here. Score-based scorers can activate once per frame, so this banks
    // their trigger for the next frame after they activate.
    if SCORE && out.score() >= ladder.low() {
        ladder.climb::<YEAR>(due, now, scoring, rng, out, chain, expiry);
    } else {
        moment::<YEAR>(due, now, scoring, rng, out, chain, expiry);
        if SCORE {
            ladder.settle::<YEAR>(now, scoring, rng, out, chain, expiry);
        }
    }
}

/// You can think of this as a millisecond even if that's not quite accurate.
/// Handles the skill activation mask, most skills firing right->left, and then year groups.
#[inline(always)]
fn moment<'q, const YEAR: bool>(
    mask: u16,
    now: Time,
    scoring: &Scoring,
    rng: &mut Rng,
    out: &mut Outcome,
    chain: &mut Chain,
    expiry: &mut impl Lasting<'q>,
) {
    expiry.begin();
    if YEAR {
        let landed = fire::<true, true, _>(mask, now, scoring, rng, out, expiry);
        chain.after(landed, now, scoring, rng, out, expiry);
    } else {
        fire::<false, true, _>(mask, now, scoring, rng, out, expiry);
    }
}

/// Ends a duration effect.
#[inline(never)]
fn release<'q, const SCORE: bool, const YEAR: bool, E>(
    at: Time,
    scoring: &Scoring,
    mut rng: Rng,
    mut out: Outcome,
    ladder: &mut Ladder,
    chain: &mut Chain,
    expiry: &mut E,
) -> (Rng, Outcome)
where
    E: Lasting<'q>,
{
    if E::PSU || E::PARAM {
        expiry.expire(at, out.perfects);
        if expiry.next() > at {
            return (rng, out);
        }
    }
    if SCORE {
        ladder.catch_up::<YEAR>(at, scoring, &mut rng, &mut out, chain, expiry);
    }
    let due = expiry.release(at);
    if SCORE {
        ladder.climb::<YEAR>(due, at, scoring, &mut rng, &mut out, chain, expiry);
    } else {
        moment::<YEAR>(due, at, scoring, &mut rng, &mut out, chain, expiry);
    }
    (rng, out)
}

struct Spawns<'a> {
    icons: &'a [Time],
    at: &'a [u32],
    lanes: &'a [u16],
    icon: usize,
    cursor: usize,
    next: usize,
}

impl Spawns<'_> {
    #[inline(always)]
    fn firing(&self, offset: u32) -> Time {
        self.icons
            .get(self.next.wrapping_sub(1))
            .map_or(Time(u32::MAX), |t| t.spawn(offset))
    }

    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "`at` is terminated, `icons` is bounded")]
    #[expect(
        clippy::too_many_arguments,
        reason = "Legit trying to fix this lead to a +7% performance regression. `offset` as a field of `Spawns` moved register allocation to all of `walk`."
    )]
    fn drain<'q, const SCORE: bool, const YEAR: bool, const LAST: bool, E>(
        &mut self,
        bound: Time,
        offset: u32,
        scoring: &Scoring,
        rng: &mut Rng,
        out: &mut Outcome,
        ladder: &mut Ladder,
        chain: &mut Chain,
        expiry: &mut E,
    ) where
        E: Lasting<'q>,
    {
        let mut stop = bound;
        loop {
            while self.icon < self.icons.len() && self.icons[self.icon].spawn(offset) < stop {
                self.icon = self.icon.saturating_add(1);
                if self.icon == self.next {
                    let now = self.icons[self.icon.saturating_sub(1)].spawn(offset);
                    if SCORE {
                        ladder.catch_up::<YEAR>(now, scoring, rng, out, chain, expiry);
                    }
                    moment::<YEAR>(self.lanes[self.cursor], now, scoring, rng, out, chain, expiry);
                    self.cursor = self.cursor.saturating_add(1);
                    self.next = self.at[self.cursor] as usize;
                    if SCORE {
                        ladder.settle::<YEAR>(now, scoring, rng, out, chain, expiry);
                    }
                    if E::ANY {
                        stop = stop.min(expiry.next());
                    }
                    while LAST && E::ANY && expiry.next() == bound && self.firing(offset) >= bound {
                        (*rng, *out) = release::<SCORE, YEAR, _>(
                            bound,
                            scoring,
                            rng.clone(),
                            *out,
                            ladder,
                            chain,
                            expiry,
                        );
                    }
                }
            }
            if !E::ANY || stop == bound {
                return;
            }
            core::hint::cold_path();
            (*rng, *out) =
                release::<SCORE, YEAR, _>(stop, scoring, rng.clone(), *out, ladder, chain, expiry);
            stop = bound.min(expiry.next());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessorySkill, Outcome, Scoring, Skill, Triggers, run};
    use crate::play::config::{NoteSpeed, PlayConfig};
    use crate::play::judgement::Judgements;
    use crate::play::queue::Queue;
    use crate::scalar::{RATE_UNIT, Rate};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{
        Chart, ChartArena, ChartIdx, ChartRows, Difficulty, Event, EventKind, SongIdx, Time,
    };

    /// Fake skill that always procs so they can be tested.
    pub(super) const ALWAYS: Rate = RATE_UNIT;

    pub(super) fn arena(judgements: &[(Time, Event)]) -> ChartArena {
        let mut arena = ChartArena::default();
        arena.push(SongIdx(0), Difficulty::Master, judgements);
        arena
    }

    pub(super) fn only(arena: &ChartArena) -> Chart {
        *arena.chart(ChartIdx(0)).expect("the one chart just pushed")
    }

    pub(super) fn tap(ms: u32, lane: u8) -> (Time, Event) {
        (Time(ms), Event::new(lane, EventKind::Tap, false, false))
    }

    pub(super) fn config() -> PlayConfig {
        PlayConfig::new(NoteSpeed::FASTEST, PlayConfig::ALL_PERFECT, 0)
    }

    pub(super) fn sloppy(accuracy: Rate, seed: u64) -> PlayConfig {
        PlayConfig { accuracy, seed, ..config() }
    }

    pub(super) fn scorer(threshold: u16, chance: Rate, value: u32) -> Skill {
        Skill {
            value,
            chance,
            threshold,
            effect: SkillEffect::Scorer,
            trigger: TriggerType::RhythmIcons,
            limit: None,
        }
    }

    pub(super) fn worn(chance: Rate, value: u32) -> AccessorySkill {
        AccessorySkill { value, chance, effect: SkillEffect::Scorer }
    }

    pub(super) fn play(queue: &Queue, scoring: &Scoring) -> Outcome {
        seeded(queue, scoring, 0)
    }

    pub(super) fn seeded(queue: &Queue, scoring: &Scoring, seed: u64) -> Outcome {
        at(queue, scoring, &PlayConfig { seed, ..config() })
    }

    pub(super) fn at(queue: &Queue, scoring: &Scoring, config: &PlayConfig) -> Outcome {
        let triggers = Triggers::build(queue, scoring);
        let judgements = Judgements::build(queue, config);
        run(queue, config, scoring, &triggers, &judgements)
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "why can't you configure clippy.toml to disable this in tests?"
    )]
    pub(super) fn fifty_one() -> ChartArena {
        let stream: Vec<_> = (0..51).map(|i| tap(2000 + i * 100, (i % 9) as u8)).collect();
        arena(&stream)
    }

    #[test]
    fn check_hold_note_start() {
        let stream = [
            (Time(2000), Event::new(0, EventKind::HoldStart, false, false)),
            tap(4000, 1),
            (Time(9000), Event::new(0, EventKind::HoldRelease, false, false)),
        ];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.icons(), 2, "one for the hold, one for the tap");
        assert_eq!(queue.icons(), queue.notes(), "same count");

        let mut scoring = Scoring::flat(0);
        scoring.skills[3] = scorer(1, ALWAYS, 7);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 2);
        assert_eq!(out.skill, 14);

        let slow = PlayConfig::new(NoteSpeed::SLOWEST, PlayConfig::ALL_PERFECT, 0);
        assert_eq!(at(&queue, &scoring, &slow).icons, 2);
    }

    #[test]
    fn check_simultaneous_note_spawns() {
        let stream = [tap(1400, 0), tap(2000, 1)];
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        assert_eq!(queue.times()[1].spawn(NoteSpeed::FASTEST.offset()), queue.times()[0]);

        let mut scoring = Scoring::flat(0);
        scoring.skills[0] = scorer(1, ALWAYS, 5);
        let out = play(&queue, &scoring);
        assert_eq!(out.icons, 2);
        assert_eq!(out.activations, 2);
    }
}
