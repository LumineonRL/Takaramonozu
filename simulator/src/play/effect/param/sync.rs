//! Sync. Just the parts that are unique to Sync.
//! see param.rs, which is mostly the home of param boost, but is tightly coupled with
//! how Sync works.

use crate::play::Scoring;
use crate::play::rng::Rng;
use crate::song::{LANES, Time};

use super::NEVER;

/// Every lane's stats.
/// Sum of what each lane has with no boosts active.
pub(super) struct Copies {
    /// Lane's actual stat OR the stat it's copying.
    shown: u32,
    /// Lanes currently holding a copied stat.
    synced: u16,
    /// What lane each synced lane is syncing to.
    copied: [u32; LANES],
    until: [Time; LANES],
    /// Makes its own RNG stream when Sync first rolls.
    seed: u64,
    picks: Option<Rng>,
    /// See `Scoring`.
    stats: [u32; LANES],
}

impl Copies {
    #[inline(always)]
    pub(super) const fn new(scoring: &Scoring) -> Self {
        Self {
            shown: scoring.team_strength,
            synced: 0,
            copied: [0; LANES],
            until: [Time(0); LANES],
            seed: 0,
            picks: None,
            stats: scoring.stats,
        }
    }

    #[inline(always)]
    pub(super) const fn seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    #[inline(always)]
    pub(super) const fn shown(&self) -> u32 {
        self.shown
    }

    /// Holds a copy of each lane with a valid target until expiry.
    /// Sync passes on what it copies. does not pass along its own boost.
    /// Returns the bit of the lane it copied (or 0 if there's no one to copy)
    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    pub(super) fn land(&mut self, lane: u32, targets: u16, until: Time, boost: (u16, u16)) -> u16 {
        let seed = self.seed;
        let picks = self.picks.get_or_insert_with(|| Rng::new(seed, Rng::SYNC));
        let Some(target) = pick(targets, picks) else {
            return 0;
        };
        let (reach, percent) = boost;
        let bit = 1_u16.checked_shl(target).unwrap_or(0);
        let stat = self.stats[target as usize];
        let copy = if self.synced & bit != 0 {
            self.copied[target as usize]
        } else if reach & bit != 0 {
            stat.saturating_add(share(stat, percent))
        } else {
            stat
        };
        let own = 1_u16.checked_shl(lane).unwrap_or(0);
        debug_assert!(self.synced & own == 0, "lane lapsed before another successful roll");
        self.shown = self.shown.saturating_sub(self.stats[lane as usize]).saturating_add(copy);
        self.copied[lane as usize] = copy;
        self.until[lane as usize] = until;
        self.synced |= own;
        bit
    }

    /// In charge of setting stats back to normal when durations end.
    #[inline(always)]
    #[expect(clippy::indexing_slicing, reason = "lane bits")]
    pub(super) fn lapse(&mut self, now: Time) -> Time {
        let mut ending = NEVER;
        let mut bits = self.synced;
        while let Some(lane) = bits.lowest_one() {
            let bit = bits.isolate_lowest_one();
            bits ^= bit;
            let end = self.until[lane as usize];
            if end <= now {
                self.synced ^= bit;
                let (copy, stat) = (self.copied[lane as usize], self.stats[lane as usize]);
                self.shown = self.shown.saturating_sub(copy).saturating_add(stat);
            } else {
                ending = ending.min(end);
            }
        }
        ending
    }
}

/// `ceil(stat * percent%)`
fn share(stat: u32, percent: u16) -> u32 {
    let scaled = u64::from(stat).saturating_mul(u64::from(percent)).div_ceil(100);
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// This is why sync needs its own RNG stream.
/// It randomly picks its target among valid targets.
fn pick(lanes: u16, rng: &mut Rng) -> Option<u32> {
    let mut bits = lanes;
    let count = bits.count_ones();
    if count > 1 {
        for _ in 0..rng.below(count) {
            bits ^= bits.isolate_lowest_one();
        }
    }
    bits.lowest_one()
}

#[cfg(test)]
mod tests {
    use crate::play::activation::{AccessorySkill, Triggers};
    use crate::play::effect::tests::{
        FLIP, STATS, TEAM, clocked, fever, healer, lock, overhealing, param, psu, sync, trick,
    };
    use crate::play::queue::Queue;
    use crate::play::rng::Rng;
    use crate::play::tests::{
        ALWAYS, arena, at, fifty_one, only, play, scorer, seeded, sloppy, tap, worn,
    };
    use crate::play::{Outcome, Scoring};
    use crate::skill::{SkillEffect, TriggerType};
    use crate::song::{Event, EventKind, Time};

    /// mock fifty-one-note map's taps, AP, with note `n` at `strength(n)`.
    #[expect(clippy::integer_division, reason = "testy mctest")]
    #[expect(clippy::arithmetic_side_effects, reason = "still testing")]
    fn tally(strength: impl Fn(u64) -> u64) -> u64 {
        (1..=51).map(|n| strength(n) * if n > 50 { 110 } else { 100 } / 8000).sum()
    }

    fn synced() -> Scoring {
        Scoring { stats: STATS, ..Scoring::flat(TEAM) }
    }

    #[test]
    fn check_sync() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        // You know the drill by now, this is reused on plenty of tests.
        // [3000, 3500) and [6000, 6500): notes 11 to 15 and 41 to 45.
        // or maybe it's your first time seeing that.
        sync(&mut scoring, 0, TriggerType::Time, 3, ALWAYS, 500, 1 << 1);
        let triggers = Triggers::build(&queue, &scoring);
        assert_eq!((triggers.sync(), triggers.param(), triggers.lasting()), (1, 0, 1));

        let window = |n| (11..=15).contains(&n) || (41..=45).contains(&n);
        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.skill), (2, 0));
        assert_eq!(out.tap, tally(|n| if window(n) { 96_000 } else { TEAM.into() }));

        // A weaker lane lowers the team.
        scoring.targets[0][0] = 1 << 2;
        assert_eq!(play(&queue, &scoring).tap, tally(|n| if window(n) { 78_000 } else { 80_000 }));
    }

    /// Transcription of a real play frfr
    /// Two Planetarium Nozomis, one 3270 and the other 2990 Pure on a 12,851 team.
    /// The right one copies the left, then the left copies what the right
    /// now holds, which is its own stat.
    /// Both end on the left card's stats. The same note on the same chart with both skills active
    /// gave 180 points with the stronger nozomi on the left and 172 points with it on the right.
    #[test]
    #[expect(clippy::integer_division, reason = "testing a real game mechanic.")]
    fn check_mutual_sync_goes_right_to_left() {
        let arena = arena(&[tap(2000, 3)]);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        for (left, right, seen) in [(3270, 2990, 180), (2990, 3270, 172)] {
            let mut scoring = Scoring::flat(12_851);
            scoring.stats[0] = 12_851 - left - right;
            (scoring.stats[3], scoring.stats[5]) = (left, right);
            scoring.lanes[3] = 110;
            sync(&mut scoring, 3, TriggerType::RhythmIcons, 1, ALWAYS, 6500, 1 << 5);
            sync(&mut scoring, 5, TriggerType::RhythmIcons, 1, ALWAYS, 6500, 1 << 3);
            let out = play(&queue, &scoring);
            assert_eq!((out.activations, out.tap), (2, seen), "{left} on the left");
        }
        assert_eq!(12_851 * 110 / 8000, 176, "a swap");
    }

    /// Lane 1 copies lane 2 at 4000
    /// Boost from lane 8 gives lane 1 another 12,000.
    /// copy keeps them after the boost ends.
    /// Boosts land on icons 25 and 50: [3800, 4300) and [6300, 6800)
    #[test]
    fn check_sync_copies_the_boost_it_finds() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        param(&mut scoring, 8, TriggerType::RhythmIcons, 25, ALWAYS, 12_000, 500);
        (scoring.targets[8][0], scoring.boosts[8][0]) = (1 << 1, 50);
        sync(&mut scoring, 0, TriggerType::Time, 4, ALWAYS, 60_000, 1 << 1);

        let boosted = |n| (20..=23).contains(&n) || (45..=48).contains(&n);
        let want = |copied: u64| {
            tally(|n| {
                let boost = if boosted(n) { 12_000 } else { 0 };
                80_000 + boost + if n >= 21 { copied - 8000 } else { 0 }
            })
        };
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 3);
        assert_eq!(out.tap, want(36_000));
        assert_ne!(out.tap, want(24_000), "the bare stat");

        // Same test but from an acc in lane 8.
        let mut worn_boost = scoring;
        worn_boost.skills[8].chance = 0;
        (worn_boost.targets[8][0], worn_boost.boosts[8][0]) = (1 << 2, 100);
        worn_boost.accessories[8] =
            AccessorySkill { effect: SkillEffect::AppealBoost, ..worn(ALWAYS, 12_000) };
        worn_boost.durations[8][1] = 500;
        (worn_boost.targets[8][1], worn_boost.boosts[8][1]) = (1 << 1, 50);
        let behind = play(&queue, &worn_boost);
        assert_eq!((behind.activations, behind.accessory_activations), (1, 2));
        assert_eq!(behind.tap, want(36_000));
    }

    /// Lane 0 syncs lane 1's 24,000 from note 1.
    /// A boost to lanes 0 and 1 then adds 4000 and
    /// 12,000. From the copy it would add 24,000.
    #[test]
    fn check_a_boosting_synced_lane() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        sync(&mut scoring, 0, TriggerType::Time, 2, ALWAYS, 60_000, 1 << 1);
        param(&mut scoring, 8, TriggerType::RhythmIcons, 25, ALWAYS, 16_000, 500);
        (scoring.targets[8][0], scoring.boosts[8][0]) = (0b11, 50);

        let boosted = |n| (20..=23).contains(&n) || (45..=48).contains(&n);
        let want = |boost| tally(|n| 96_000 + if boosted(n) { boost } else { 0 });
        let out = play(&queue, &scoring);
        assert_eq!(out.tap, want(16_000));
        assert_ne!(out.tap, want(24_000), "boosted on the copy");
    }

    /// Lane 2 copies lane 0 at 4000
    /// Lane 0 syncs lane 1's 24,000 and a boost gives lane 0
    /// 4000 of its own.
    /// Lane 2 takes the 24,000.
    #[test]
    fn check_a_synced_lane_passes_only_what_it_copied() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        sync(&mut scoring, 0, TriggerType::Time, 2, ALWAYS, 60_000, 1 << 1);
        param(&mut scoring, 8, TriggerType::RhythmIcons, 25, ALWAYS, 19_000, 500);
        (scoring.targets[8][0], scoring.boosts[8][0]) = (0b111, 50);
        sync(&mut scoring, 2, TriggerType::Time, 4, ALWAYS, 60_000, 1);

        let boosted = |n| (20..=23).contains(&n) || (45..=48).contains(&n);
        let want = |copied: u64| {
            tally(|n| {
                let boost = if boosted(n) { 19_000 } else { 0 };
                96_000 + boost + if n >= 21 { copied - 6000 } else { 0 }
            })
        };
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 4);
        assert_eq!(out.tap, want(24_000));
        assert_ne!(out.tap, want(28_000), "its own boost passed on");
    }

    /// Checks that it can roll multiple targets and checks that the skill rng stream isn't affected.
    #[test]
    fn check_sync_picks_at_random() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        sync(&mut scoring, 3, TriggerType::Time, 1, ALWAYS, 60_000, 1 << 4);
        sync(&mut scoring, 0, TriggerType::Time, 2, ALWAYS, 60_000, 0b110);
        scoring.skills[4] = clocked(TriggerType::Time, 3, FLIP, 1);

        let mut seen = [0; 2];
        for (targets, choice) in [(0b110, true), (0b10, false)] {
            scoring.targets[0][0] = targets;
            for seed in 0..64 {
                let mut rng = Rng::new(seed, Rng::SKILLS);
                assert!(rng.roll(ALWAYS) && rng.roll(ALWAYS));
                let hits = u32::from(rng.roll(FLIP)) + u32::from(rng.roll(FLIP));
                let pick = if choice {
                    Rng::new(seed, Rng::SYNC).below(2)
                } else {
                    0
                };
                let copied = [24_000, 6000][pick as usize];

                let out = seeded(&queue, &scoring, seed);
                assert_eq!(
                    (out.activations, out.skill),
                    (2 + hits, u64::from(hits)),
                    "seed {seed}"
                );
                assert_eq!(out.tap, tally(|_| 72_000 + copied), "seed {seed}");
                if choice {
                    seen[pick as usize] += 1;
                }
            }
        }
        assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
    }

    /// can't trigger more duration skills when sync is active.
    #[test]
    fn check_sync_blocks_lane() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        sync(&mut scoring, 4, TriggerType::Perfects, 5, ALWAYS, 550, 1 << 1);
        let out = play(&queue, &scoring);
        assert_eq!(out.activations, 9);

        let mut paying_nothing = scoring;
        psu(&mut paying_nothing, 4, TriggerType::Perfects, 5, ALWAYS, 0, 550);
        let unsynced = play(&queue, &paying_nothing);
        assert_eq!(Outcome { tap: unsynced.tap, ..out }, unsynced);
        assert!(out.tap > unsynced.tap);

        // Nobody to copy, still blocks. Very good skill.
        scoring.targets[4][0] = 0;
        assert_eq!(play(&queue, &scoring), unsynced);
    }

    /// Card copies lane 2 and its accessory lane 1. Only the accessory rolls.
    #[test]
    fn check_sync_acc() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        sync(&mut scoring, 6, TriggerType::Time, 1, 0, 60_000, 1 << 2);
        scoring.accessories[6] = AccessorySkill { effect: SkillEffect::Sync, ..worn(ALWAYS, 0) };
        (scoring.durations[6][1], scoring.targets[6][1]) = (60_000, 1 << 1);
        assert_eq!(Triggers::build(&queue, &scoring).sync(), 1 << 6);

        let out = play(&queue, &scoring);
        assert_eq!((out.activations, out.accessory_activations, out.skill), (0, 1, 0));
        assert_eq!(out.tap, tally(|_| 96_000));
    }

    /// The 1.08x PL Perfect and 1.33x Trick SIS work on the copied team
    /// they're additive and separate.
    #[test]
    fn check_sync_under_a_lock() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = Scoring { trick: 8000, ..synced() };
        sync(&mut scoring, 0, TriggerType::Time, 1, ALWAYS, 60_000, 1 << 1);
        lock(&mut scoring, 4, TriggerType::Time, 1, ALWAYS, 60_000);
        assert_eq!(play(&queue, &scoring).tap, tally(|_| 112_320), "104,000 at 1.08");
        assert_eq!(at(&queue, &scoring, &sloppy(0, 0)).tap, tally(|_| 104_000), "converted");

        let mut tricked = scoring;
        trick(&mut tricked, 4, TriggerType::Time, 1, ALWAYS, 60_000);
        for accuracy in [ALWAYS, FLIP, 0] {
            assert_eq!(at(&queue, &tricked, &sloppy(accuracy, 0)).tap, tally(|_| 104_000));
        }
    }

    #[test]
    #[expect(clippy::integer_division, reason = "it's a test.")]
    fn check_sync_with_hearts() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut scoring = synced();
        overhealing(&mut scoring, 30);
        healer(&mut scoring, 4, TriggerType::RhythmIcons, 10, ALWAYS, 30);
        // Lane 8 takes lane 1's 24,000 from note 26.
        sync(&mut scoring, 8, TriggerType::Combo, 25, ALWAYS, 60_000, 1 << 1);
        let want: u64 = (1..=51_u64)
            .map(|note| {
                let heart = 10 + (note + 5) / 10;
                let strength = if note > 25 { 96 } else { 80 };
                let combo = if note > 50 { 110 } else { 100 };
                strength * heart * combo / 80
            })
            .sum();
        assert_eq!(play(&queue, &scoring).tap, want);
    }

    /// For when sync ends mid-hold.
    #[test]
    fn check_sync_ends_through_releases() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        let mut locked = synced();
        lock(&mut locked, 4, TriggerType::RhythmIcons, 1, ALWAYS, 250);
        let mut copying = locked;
        sync(&mut copying, 0, TriggerType::Time, 3, ALWAYS, 500, 1 << 1);

        let (with, without) = (play(&queue, &copying), play(&queue, &locked));
        // Notes 11 to 15 and 41 to 45, each 16,000 * 1.08.
        assert_eq!(with.tap, without.tap + 10 * 216);
        assert_eq!(
            Outcome {
                tap: with.tap,
                activations: without.activations + 2,
                ..without
            },
            with
        );
    }

    #[test]
    fn check_sync_window_copies() {
        let arena = fifty_one();
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");
        // [3000, 3500) and [6000, 6500), lane 1 down to lane 0's 8000.
        let mut bare = synced();
        sync(&mut bare, 1, TriggerType::Time, 3, ALWAYS, 500, 1);
        let window = |n| (11..=15).contains(&n) || (41..=45).contains(&n);
        assert_eq!(play(&queue, &bare).tap, tally(|n| if window(n) { 64_000 } else { 80_000 }));

        let mut copies = 0;
        for shape in 0..128_u32 {
            for lasting in 0..3 {
                let mut team = bare;
                if shape & 1 != 0 {
                    team.skills[3] = clocked(TriggerType::Score, 1500, 0, 50);
                }
                if shape & 2 != 0 {
                    team.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), 0, 70);
                    (team.members[0], team.members[2], team.members[5]) = (1, 1 << 2, 1 << 5);
                }
                if shape & 4 != 0 {
                    team.skills[8] = clocked(TriggerType::StarNotes, 1, 0, 5);
                }
                if shape & 8 != 0 {
                    overhealing(&mut team, 30);
                    healer(&mut team, 6, TriggerType::RhythmIcons, 4, 0, 12);
                }
                if shape & 16 != 0 {
                    team.skills[7] = clocked(TriggerType::RhythmIcons, u16::MAX, 0, 3);
                    team.accessories[7] =
                        AccessorySkill { effect: SkillEffect::ComboFever, ..worn(ALWAYS, 30) };
                    team.durations[7][1] = 400;
                }
                if shape & 32 != 0 {
                    team.skills[2] = scorer(u16::MAX, 0, 100);
                    team.accessories[2] =
                        AccessorySkill { effect: SkillEffect::PerfectScoreUp, ..worn(ALWAYS, 50) };
                    team.durations[2][1] = 500;
                }
                if shape & 64 != 0 {
                    team.skills[5] = scorer(u16::MAX, 0, 10);
                    team.accessories[5] =
                        AccessorySkill { effect: SkillEffect::SkillRateUp, ..worn(ALWAYS, 50) };
                    team.durations[5][1] = 300;
                }
                match lasting {
                    0 => {}
                    1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, 0, 600),
                    _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, 0, 600),
                }
                for accuracy in [ALWAYS, 8000] {
                    for seed in 0..2 {
                        let config = sloppy(accuracy, seed);
                        assert_eq!(
                            at(&queue, &team, &config),
                            at(&queue, &bare, &config),
                            "shape {shape}, lasting {lasting}, seed {seed}"
                        );
                    }
                }
                copies += 1;
            }
        }
        assert_eq!(copies, 384);
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "agreed`")]
    fn check_sync_copies() {
        let stream: Vec<_> = (0..51_u32)
            .map(|i| {
                (Time(2000 + i * 100), Event::new((i % 9) as u8, EventKind::Tap, i % 3 == 0, false))
            })
            .collect();
        let arena = arena(&stream);
        let queue = Queue::build(&arena, &only(&arena)).expect("terminated");

        let mut base = Scoring::flat(TEAM);
        base.stats = [8000, 8000, 6000, 6000, 6000, 6000, 8000, 8000, 24_000];
        base.trick = 9_999;
        base.skills[2] = scorer(4, FLIP, 100);
        base.skills[5] = clocked(TriggerType::Combo, 3, FLIP, 10);
        base.accessories[5] = worn(FLIP, 20);
        base.skills[6] = clocked(TriggerType::Time, 1, FLIP, 7);
        base.skills[7] = clocked(TriggerType::Perfects, 2, FLIP, 3);

        let mut copies = 0;
        for shape in 0..128_u32 {
            for lasting in 0..3 {
                let mut team = base;
                if shape & 1 != 0 {
                    team.skills[3] = clocked(TriggerType::Score, 1500, FLIP, 50);
                }
                if shape & 2 != 0 {
                    team.skills[0] = clocked(TriggerType::YearGroup, (1 << 2) | (1 << 5), FLIP, 70);
                    (team.members[0], team.members[2], team.members[5]) = (1, 1 << 2, 1 << 5);
                }
                if shape & 4 != 0 {
                    team.skills[8] = clocked(TriggerType::StarNotes, 1, FLIP, 5);
                }
                if shape & 8 != 0 {
                    overhealing(&mut team, 30);
                    team.accessories[6] =
                        AccessorySkill { effect: SkillEffect::Healer, ..worn(FLIP, 0) };
                    team.heals[6][1] = 12;
                }
                if shape & 16 != 0 {
                    team.accessories[7] =
                        AccessorySkill { effect: SkillEffect::ComboFever, ..worn(FLIP, 30) };
                    team.durations[7][1] = 400;
                }
                if shape & 32 != 0 {
                    team.accessories[2] =
                        AccessorySkill { effect: SkillEffect::PerfectScoreUp, ..worn(FLIP, 50) };
                    team.durations[2][1] = 500;
                }
                if shape & 64 != 0 {
                    team.accessories[5] =
                        AccessorySkill { effect: SkillEffect::SkillRateUp, ..worn(FLIP, 50) };
                    team.durations[5][1] = 300;
                }
                match lasting {
                    0 => {}
                    1 => lock(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                    _ => trick(&mut team, 4, TriggerType::RhythmIcons, 5, FLIP, 600),
                }

                let mut never = team;
                sync(&mut never, 1, TriggerType::RhythmIcons, 3, 0, 700, 1 << 8);
                let mut cold = team;
                cold.skills[1] = scorer(3, 0, 0);
                let mut nothing = team;
                sync(&mut nothing, 1, TriggerType::RhythmIcons, 3, FLIP, 700, 1);
                let mut feverish = team;
                fever(&mut feverish, 1, TriggerType::RhythmIcons, 3, FLIP, 0, 700);
                let mut copying = team;
                sync(&mut copying, 1, TriggerType::Time, 1, ALWAYS, 60_000, 1 << 8);
                let mut strong = Scoring { team_strength: TEAM + 16_000, ..team };
                fever(&mut strong, 1, TriggerType::Time, 1, ALWAYS, 0, 60_000);

                let triggers = Triggers::build(&queue, &never);
                assert_eq!((triggers.sync(), triggers.param()), (1 << 1, 0));
                assert_eq!(triggers.score() != 0, shape & 1 != 0);
                assert_eq!(triggers.year() != 0, shape & 2 != 0);
                assert_eq!(!triggers.stars().is_empty(), shape & 4 != 0);
                assert_eq!(triggers.heals() != 0, shape & 8 != 0);
                assert_eq!(triggers.fever() != 0, shape & 16 != 0);
                assert_eq!(triggers.psu() != 0, shape & 32 != 0);
                assert_eq!(triggers.sru() != 0, shape & 64 != 0);
                assert_eq!(triggers.tricks() != 0, lasting == 2);

                for accuracy in [ALWAYS, 8000, 0] {
                    for seed in 0..4 {
                        let config = sloppy(accuracy, seed);
                        let what = format!("shape {shape}, lasting {lasting}, seed {seed}");
                        assert_eq!(
                            at(&queue, &never, &config),
                            at(&queue, &cold, &config),
                            "{what}"
                        );
                        assert_eq!(
                            at(&queue, &nothing, &config),
                            at(&queue, &feverish, &config),
                            "{what}"
                        );
                        assert_eq!(
                            at(&queue, &copying, &config),
                            at(&queue, &strong, &config),
                            "{what}"
                        );
                    }
                }
                copies += 1;
            }
        }
        assert_eq!(copies, 384, "every Sync copy");
    }
}
