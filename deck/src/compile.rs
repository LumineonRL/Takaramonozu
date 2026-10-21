//! The stat calculation pipeline for a Team.
//!
//! Special thanks to none other than me from 10 years ago, coming in clutch
//! with a spreadsheet I built to calculate team stats, as it helped present me
//! fix a bug in the calculation order.
//! See <https://docs.google.com/spreadsheets/d/1ecVUlSYMU5oBAvSbhgmbUjPlJZTklf2OF6_s3MY4dTM>
//! for a historic relic of just how long I've been wanting to "solve" SIF.
//! Edit: Turns out I think I might've had in oopsie in my 10-year old spreadsheet.
//! Thank you sifcord Discord server logs.

use core::num::NonZeroU8;

use simulator::accessory::AccessoryRows;
use simulator::card::{Card, CardDb, LeaderSkill, LeaderTargets};
use simulator::play::Scoring;
use simulator::play::activation::{AMP_LEVELS, AccessorySkill, Payload, Skill};
use simulator::scalar::{
    Attribute, Centi, CharacterId, CharacterSet, Milli, Ms, Pct, RATE_UNIT, Rate,
};
use simulator::sis::SisEffect;
use simulator::skill::{SkillEffect, SkillRows, TriggerType};
use simulator::song::Matching;

use crate::activation_limit;
use crate::card::DeckEntry;
use crate::level_cap::LevelCaps;
use crate::manager::{EntryId, Manager};
use crate::team::{CENTER, Context, LANES, Team};

/// The tap bonus the heart count is capped one step past.
const OVERHEAL_TARGET: u32 = 20_000;

/// A given lane's tap score multiplier.
///
/// 1.1x if a the card attribute or card's group matches the song's attribute group
/// 1.1 * 1.1 = 1.21x if both match
/// 1.0x if none of them match.
///
/// Putting this here since it stays constant for the same team+song if we're running many sims on the same
/// configurations. Pre-compute at the compilation step instead of re-deriving later.
const NO_MATCH: u16 = 100;
const ONE_MATCH: u16 = 110;
const BOTH_MATCH: u16 = 121;
const MILLI_UNIT: u32 = 1000;
const PAYOUT_DIVISOR: u64 = MILLI_UNIT as u64 * 100;
const CHARM: u32 = 2500;
const HEAL: u32 = 480_000;

/// 9 cards w/ 1 HP,
const MIN_TABLE_HP: u8 = 9;

/// 9 cards w/ 7 HP, which is the highest.
const MAX_TABLE_HP: u8 = 63;

/// Overheal heart table. Thank you sifcord scientists.
const HEART_BONUS: [u16; 55] = [
    26, 29, 31, 34, 37, 40, 43, 46, 49, 51, // 9..=18
    59, 63, 66, 70, 73, 77, 80, 84, 88, 91, // 19..=28
    95, 99, 102, 106, 110, 114, 118, 121, // 29..=36
    176, 183, 190, 197, 204, 211, 218, 225, 233, // 37..=45
    264, 273, 282, 291, 300, 309, 319, 328, 338, 347, // 46..=55
    357, 367, 377, 387, 398, 408, 419, 429, // 56..=63
];

/// Divergence alert!
/// In the real game, you must field a full team to play, thus the
/// minimum possible HP is 9.
/// In this simulator I'm making it so that partial teams can be constructed
/// and scored to allow for dense RL reward.
/// However, no entries in the `HEART_BONUS` table exist for scenarios with
/// less than 9 max HP. So we're just defaulting to 0 :). In practice this
/// should not matter at all.
#[must_use]
pub fn heart_bonus(max_hp: u8) -> u16 {
    if max_hp < MIN_TABLE_HP {
        return 0;
    }
    let row = max_hp.min(MAX_TABLE_HP).saturating_sub(MIN_TABLE_HP);
    HEART_BONUS.get(usize::from(row)).copied().unwrap_or(0)
}

/// I'll do my best to summarize the SIFcord findings here.
/// This tracks how many hearts a team can bank before the tap bonus stops growing.
///
/// It's the count that's capped. The game stops one step past +200%.
/// 54 max HP means a heart is worth 3.38%. The cap is 60, so 202.8%, then
/// clamp that down to 200%.
///
/// Oh, another assumption. We're only modeling GREAT/PERFECT taps, no misses
/// or anything that will cost stamina, so no modeling the mechanic where you
/// lose partial overheal gauge.
#[expect(clippy::doc_markdown, reason = "Are we really doing this again, Clippy?")]
#[must_use]
pub fn max_hearts(per_heart: u16) -> u16 {
    if per_heart == 0 {
        return 0;
    }
    let cap = OVERHEAL_TARGET.div_ceil(u32::from(per_heart));
    u16::try_from(cap).unwrap_or(u16::MAX)
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct SisFlags(pub u8);

impl SisFlags {
    const CHARM: u8 = 1 << 0;
    const HEAL: u8 = 1 << 1;

    #[must_use]
    pub const fn charm(self) -> bool {
        (self.0 & Self::CHARM) != 0
    }

    #[must_use]
    pub const fn heal(self) -> bool {
        (self.0 & Self::HEAL) != 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(C)]
pub struct Slot {
    pub stats: [u32; 3],
    pub trick: [u32; 3],
    pub skill: Skill,
    pub accessory: AccessorySkill,
    pub bonus: u16,
    /// See `Scoring::members`.
    pub member: u16,
    /// See `Scoring::durations`.
    pub durations: [Ms; 2],
    /// See `Scoring::heals`.
    pub heals: [u8; 2],
    pub sis: SisFlags,
    /// See `Scoring::sparks`.
    pub spark: u8,
    /// See `Scoring::stats`.
    pub strength: u32,
    /// See `Scoring::targets`.
    pub targets: [u16; 2],
    /// See `Scoring::boosts`.
    pub boosts: [u16; 2],
    /// See `Scoring::amplified`.
    pub amplified: [[Payload; AMP_LEVELS]; 2],
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            stats: [0; 3],
            trick: [0; 3],
            skill: Skill::NONE,
            accessory: AccessorySkill::NONE,
            bonus: NO_MATCH,
            member: 0,
            durations: [0; 2],
            heals: [0; 2],
            sis: SisFlags(0),
            spark: 0,
            strength: 0,
            targets: [0; 2],
            boosts: [0; 2],
            amplified: [[Payload::default(); AMP_LEVELS]; 2],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Compiled {
    pub slots: [Slot; LANES],
    pub totals: [u32; 3],
    pub team_strength: u32,
    pub trick: u32,
    pub max_hp: u8,
    pub per_heart: u16,
    pub max_hearts: u16,
    pub characters: CharacterSet,
}

impl Compiled {
    #[must_use]
    pub const fn strength(&self, attribute: Attribute) -> u32 {
        pick(self.totals, attribute)
    }

    #[must_use]
    pub fn scoring(&self) -> Scoring {
        Scoring {
            skills: self.slots.map(|s| s.skill),
            accessories: self.slots.map(|s| s.accessory),
            team_strength: self.team_strength,
            trick: self.trick,
            durations: self.slots.map(|s| s.durations),
            lanes: self.slots.map(|s| s.bonus),
            members: self.slots.map(|s| s.member),
            heals: self.slots.map(|s| s.heals),
            per_heart: self.per_heart,
            max_hearts: self.max_hearts,
            max_hp: self.max_hp,
            stats: self.slots.map(|s| s.strength),
            targets: self.slots.map(|s| s.targets),
            boosts: self.slots.map(|s| s.boosts),
            amplified: self.slots.map(|s| s.amplified),
            sparks: self.slots.map(|s| s.spark),
        }
    }
}

/// Oh apparently you add everything together here.
#[derive(Clone, Copy, Default)]
struct Sums {
    /// Self-percent SIS
    percent: [u32; 3],
    /// Self-flat SIS
    flat: [u32; 3],
    /// Raw card stat
    base: [u32; 3],
    /// Trick SIS
    trick: [u32; 3],
}

/// Each lane's initial tap multiplier.
fn lane_bonus(card: &Card, song: Matching) -> u16 {
    let attribute = card.attribute == song.attribute;
    let group = song.group.is_some_and(|set| set.contains(card.character));
    match (attribute, group) {
        (true, true) => BOTH_MATCH,
        (true, false) | (false, true) => ONE_MATCH,
        (false, false) => NO_MATCH,
    }
}

#[must_use]
pub fn compile(team: &Team, ctx: Context, caps: &LevelCaps, song: Matching) -> Compiled {
    let mut out = Compiled::default();
    let mut sums = [Sums::default(); LANES];
    let mut chars = [None; LANES];
    let mut years = [None; LANES];
    let mut all = [0_u32; 3];
    let mut grouped = false;
    let mut center = None;

    for (lane, (((dst, lane_sums), who), year)) in
        out.slots.iter_mut().zip(&mut sums).zip(&mut chars).zip(&mut years).enumerate()
    {
        let Some(slot) = team.slot(lane) else {
            continue;
        };
        let (Some(card), Some(entry)) = (ctx.card(slot.card()), ctx.deck.get(slot.card())) else {
            continue;
        };
        let Some(base) = ctx.deck.base_stats(slot.card(), ctx.cards, caps) else {
            continue;
        };

        // Base stats from cards and accs
        dst.stats = base;
        lane_sums.base = base;
        if let Some(acc) = slot.accessory().and_then(|a| ctx.accessories.stats(a, ctx.acc_db)) {
            for (s, a) in dst.stats.iter_mut().zip(acc.0) {
                *s = s.saturating_add(u32::from(a));
            }
        }

        *who = Some(card.character);
        dst.bonus = lane_bonus(card, song);
        out.characters.0 |= bit(card.character);
        out.max_hp = out.max_hp.saturating_add(entry.hp().map_or(0, NonZeroU8::get));
        if lane == CENTER {
            center = Some(card.leader);
        }

        for sis in slot.sis().filter_map(|id| ctx.inventory.sis(id, ctx.sis_db)) {
            match sis.effect {
                SisEffect::SelfFlat => add(&mut lane_sums.flat, sis.attribute, natural(sis.value)),
                SisEffect::SelfPercent => add(&mut lane_sums.percent, sis.attribute, sis.value),
                SisEffect::AllPercent if sis.group.is_some() => grouped = true,
                SisEffect::AllPercent => add(&mut all, sis.attribute, sis.value),
                SisEffect::Charm => dst.sis.0 |= SisFlags::CHARM,
                SisEffect::Heal => dst.sis.0 |= SisFlags::HEAL,
                SisEffect::Trick => add(&mut lane_sums.trick, sis.attribute, sis.value),
            }
        }

        let (skill, duration, hp, amped) = resolve(card, entry, ctx.cards, dst.sis);
        let (accessory, worn_duration, worn_hp, worn_amped) = slot
            .accessory()
            .map_or(NO_ACCESSORY, |worn| resolve_accessory(worn, ctx, dst.sis));
        (dst.skill, dst.accessory) = (skill, accessory);
        (dst.durations, dst.heals) = ([duration, worn_duration], [hp, worn_hp]);
        dst.amplified = [amped, worn_amped];
        dst.spark = slot.accessory().map_or(0, |worn| spark(worn, ctx));
        if dst.skill.trigger == TriggerType::YearGroup {
            *year = skill_target(card, ctx.cards);
        }
    }

    // I hate the name seats but slots is taken and year group cards (and other skill effects in the future)
    // need to track "Which characters are on this team".
    let seats = seats(&chars);
    for (((dst, &seat), who), year) in out.slots.iter_mut().zip(&seats).zip(chars).zip(years) {
        dst.member = seat;
        if let (Some(who), Some(year)) = (who, year) {
            dst.skill.threshold = waits_for(year, who, &chars, &seats);
        }
    }

    if grouped && out.characters.0.count_ones() == u32::try_from(LANES).unwrap_or(u32::MAX) {
        nonet(team, ctx, out.characters, &mut all);
    }

    let guest = team.guest();
    for ((dst, lane_sums), who) in out.slots.iter_mut().zip(&sums).zip(&chars) {
        pipeline(&mut dst.stats, all, lane_sums);

        // Leader Skill and Guest bonuses
        let Some(who) = *who else { continue };
        let base = dst.stats;
        let mut bonus = [0_u32; 3];
        for id in [center, guest].into_iter().flatten() {
            if let Some(ls) = ctx.cards.leaders.get(usize::from(id.0)) {
                leader(*ls, base, who, &ctx.cards.leader_targets, &mut bonus);
            }
        }
        for (s, b) in dst.stats.iter_mut().zip(bonus) {
            *s = s.saturating_add(b);
        }

        // Trick SIS
        for ((t, s), sum) in dst.trick.iter_mut().zip(dst.stats).zip(lane_sums.trick) {
            *t = portion(s, sum);
        }
    }

    for slot in &out.slots {
        for (t, s) in out.totals.iter_mut().zip(slot.stats) {
            *t = t.saturating_add(s);
        }
        out.trick = out.trick.saturating_add(pick(slot.trick, song.attribute));
    }
    out.team_strength = pick(out.totals, song.attribute);
    reaches(&mut out.slots, team, ctx, &chars, song.attribute);
    out.per_heart = heart_bonus(out.max_hp);
    out.max_hearts = max_hearts(out.per_heart);
    out
}

/// A lane skill (value, duration) when Amp is active.
type Resolved<S> = (S, Ms, u8, [Payload; AMP_LEVELS]);

const NO_ACCESSORY: Resolved<AccessorySkill> =
    (AccessorySkill::NONE, 0, 0, [Payload { value: 0, duration: 0, extra: 0 }; AMP_LEVELS]);

/// Trigger at the card's skill level
/// Payload at the value of the amp's skill level
/// clamped to 1..16.
///
/// For the SIF nerds out there, there was a modeling decision to pad every skill to 16 levels
/// even lower rarities that might cap at only 11 like SRs. For performance it's easier to just
/// pre-pad them all to 16 so in the hot loop we only worry about clamping to one value.
fn resolve(card: &Card, entry: DeckEntry, cards: &CardDb, sis: SisFlags) -> Resolved<Skill> {
    let Some(def) = card.skill.and_then(|id| cards.skills.def(id)) else {
        return (Skill::NONE, 0, 0, NO_ACCESSORY.3);
    };
    let level = entry.skill_level();
    let trigger = cards.skills.trigger(def, level);
    let (value, duration) = cards.skills.payload(def, level);
    let amped = amplify(def.effect, level, sis, |at| cards.skills.payload(def, at));

    // TODO: Every other effect
    let skill = Skill {
        value: points(def.effect, value, sis),
        chance: rate(trigger.chance),
        threshold: trigger.threshold,
        effect: def.effect,
        trigger: def.trigger_type,
        limit: activation_limit::of(entry.card_id()),
    };
    (skill, duration, heal(def.effect, value), amped)
}

fn resolve_accessory(worn: EntryId, ctx: Context, sis: SisFlags) -> Resolved<AccessorySkill> {
    let Some((accessory, level)) = ctx.accessory(worn) else {
        return NO_ACCESSORY;
    };
    let Some(def) = ctx.acc_db.skills.def(accessory.skill) else {
        return NO_ACCESSORY;
    };
    let (value, duration) = ctx.acc_db.skills.payload(def, level);
    let amped = amplify(def.effect, level, sis, |at| ctx.acc_db.skills.payload(def, at));

    let skill = AccessorySkill {
        value: points(def.effect, value, sis),
        chance: rate(ctx.acc_db.skills.chance(def, level)),
        effect: def.effect,
    };
    (skill, duration, heal(def.effect, value), amped)
}

/// Spark's threshold (or just 0 for every other acc)
fn spark(worn: EntryId, ctx: Context) -> u8 {
    let Some((accessory, _)) = ctx.accessory(worn) else {
        return 0;
    };
    let def = ctx.acc_db.skills.def(accessory.skill);
    def.filter(|def| def.effect == SkillEffect::Spark)
        .map_or(0, |def| def.threshold)
}

/// The number of skill levels to increase the card's skill level by, 1-16 inclusive.
/// Clipped to 16 if that number comes out to more than 16.
/// An Appeal Boost's rows become points in `reaches`.
fn amplify(
    effect: SkillEffect,
    level: u8,
    sis: SisFlags,
    payload: impl Fn(u8) -> (Centi, Ms),
) -> [Payload; AMP_LEVELS] {
    let mut above = level;
    [(); AMP_LEVELS].map(|()| {
        above = above.saturating_add(1);
        let (value, duration) = payload(above);
        Payload {
            value: points(effect, value, sis),
            duration,
            extra: u16::from(heal(effect, value)),
        }
    })
}

fn skill_target(card: &Card, cards: &CardDb) -> Option<CharacterSet> {
    let def = cards.skills.def(card.skill?)?;
    cards.skill_targets.set(def.target?)
}

/// The lanes each Param Boost and Sync can affect.
/// Converts Param Boost's percents into points
/// pre-computed from each lane's final stat, so this is the very end of stat calc.
fn reaches(
    slots: &mut [Slot; LANES],
    team: &Team,
    ctx: Context,
    chars: &[Option<CharacterId>; LANES],
    attribute: Attribute,
) {
    let strengths = slots.map(|s| pick(s.stats, attribute));
    for (lane, (dst, &strength)) in (0_u32..).zip(slots.iter_mut().zip(&strengths)) {
        dst.strength = strength;
        let Some(slot) = team.slot(lane as usize) else {
            continue;
        };
        let own = 1_u16.checked_shl(lane).unwrap_or(0);
        let card = ctx.card(slot.card()).and_then(|card| skill_target(card, ctx.cards));
        let worn = slot.accessory().and_then(|worn| accessory_target(worn, ctx));
        let [(card_reach, card_boost), (worn_reach, worn_boost)] = [
            reach(dst.skill.effect, &mut dst.skill.value, card, own, chars, &strengths),
            reach(dst.accessory.effect, &mut dst.accessory.value, worn, own, chars, &strengths),
        ];
        dst.targets = [card_reach, worn_reach];
        dst.boosts = [card_boost, worn_boost];
        let [card_rows, worn_rows] = &mut dst.amplified;
        for (effect, rows, set) in [
            (dst.skill.effect, card_rows, card),
            (dst.accessory.effect, worn_rows, worn),
        ] {
            for row in rows.iter_mut() {
                let (_, percent) = reach(effect, &mut row.value, set, own, chars, &strengths);
                if effect == SkillEffect::AppealBoost {
                    row.extra = percent;
                }
            }
        }
    }
}

/// Target lanes and Param Boost percent.
/// Turns the boost's percent into points.
/// Syncs can't copy their own lane.
fn reach(
    effect: SkillEffect,
    value: &mut Centi,
    set: Option<CharacterSet>,
    own: u16,
    chars: &[Option<CharacterId>; LANES],
    strengths: &[u32; LANES],
) -> (u16, u16) {
    match effect {
        SkillEffect::AppealBoost => {
            let percent = u16::try_from(*value).unwrap_or(u16::MAX);
            *value = appeal(*value, set, chars, strengths);
            (lanes(set, chars), percent)
        }
        SkillEffect::Sync => (lanes(set, chars) & !own, 0),
        SkillEffect::Scorer
        | SkillEffect::Healer
        | SkillEffect::PerfectLock
        | SkillEffect::Encore
        | SkillEffect::Amplify
        | SkillEffect::TotalTrick
        | SkillEffect::PerfectScoreUp
        | SkillEffect::SkillRateUp
        | SkillEffect::ComboFever
        | SkillEffect::Spark => (0, 0),
    }
}

/// Filters who can affect whom for param boost and sync activations that are tied
/// to groups e.g. Aqours second years.
fn lanes(target: Option<CharacterSet>, chars: &[Option<CharacterId>; LANES]) -> u16 {
    let Some(set) = target else { return 0 };
    (0_u32..)
        .zip(chars)
        .filter(|(_, who)| who.is_some_and(|c| set.contains(c)))
        .fold(0, |mask, (lane, _)| mask | 1_u16.checked_shl(lane).unwrap_or(0))
}

fn accessory_target(worn: EntryId, ctx: Context) -> Option<CharacterSet> {
    let (accessory, _) = ctx.accessory(worn)?;
    let def = ctx.acc_db.skills.def(accessory.skill)?;
    ctx.acc_db.targets.set(def.target?)
}

/// `ceil(stat * value%)` on every lane that has a character that targets it.
fn appeal(
    value: Centi,
    target: Option<CharacterSet>,
    chars: &[Option<CharacterId>; LANES],
    strengths: &[u32; LANES],
) -> u32 {
    let Some(set) = target else { return 0 };
    chars
        .iter()
        .zip(strengths)
        .filter(|(who, _)| who.is_some_and(|c| set.contains(c)))
        .fold(0, |sum, (_, &stat)| sum.saturating_add(percent(stat, value)))
}

/// `ceil(stat * value/100)`
fn percent(stat: u32, value: Centi) -> u32 {
    let scaled = u64::from(stat).saturating_mul(u64::from(value)).div_ceil(100);
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// Each lane's character as the bit of the first lane that character is in.
fn seats(chars: &[Option<CharacterId>; LANES]) -> [u16; LANES] {
    chars.map(|who| {
        let first = (0_u32..).zip(chars).find(|&(_, &o)| who.is_some() && o == who);
        first.map_or(0, |(lane, _)| 1_u16.checked_shl(lane).unwrap_or(0))
    })
}

/// The seats aka lanes a year group card needs to "wait for" to activate.
/// 0 if the team lacks the characters for the skill to roll at all.
fn waits_for(
    year: CharacterSet,
    own: CharacterId,
    chars: &[Option<CharacterId>; LANES],
    seats: &[u16; LANES],
) -> u16 {
    let others = year.0 & !bit(own);
    let mut fielded = 0;
    let mut need = 0;
    for (who, &seat) in chars.iter().zip(seats) {
        if let Some(c) = *who
            && others & bit(c) != 0
        {
            fielded |= bit(c);
            need |= seat;
        }
    }
    if fielded == others { need } else { 0 }
}

#[expect(clippy::integer_division, reason = "they're both constants")]
const fn rate(chance: Pct) -> Rate {
    (chance as Rate).saturating_mul(RATE_UNIT / 100)
}

/// Scorer's points with Charm, Healer's points with Heal, or Combo Fever's, PSU's and Spark's points.
/// And Amplify's levels.
/// Appeal Boost stays a percent here and becomes points once every lane's stat is known.
fn points(effect: SkillEffect, value: Centi, sis: SisFlags) -> Centi {
    match effect {
        SkillEffect::Scorer => payout(value, if sis.charm() { CHARM } else { MILLI_UNIT }),
        SkillEffect::Healer => payout(value, if sis.heal() { HEAL } else { 0 }),
        SkillEffect::ComboFever | SkillEffect::PerfectScoreUp | SkillEffect::Spark => {
            payout(value, MILLI_UNIT)
        }
        SkillEffect::Amplify => levels(value),
        SkillEffect::PerfectLock
        | SkillEffect::Encore
        | SkillEffect::TotalTrick
        | SkillEffect::SkillRateUp
        | SkillEffect::AppealBoost
        | SkillEffect::Sync => value,
    }
}

#[expect(clippy::integer_division, reason = "exact")]
const fn levels(value: Centi) -> Centi {
    value / 100
}

#[expect(clippy::integer_division, reason = "exact")]
fn heal(effect: SkillEffect, value: Centi) -> u8 {
    if effect == SkillEffect::Healer {
        u8::try_from(value / 100).unwrap_or(u8::MAX)
    } else {
        0
    }
}

#[expect(clippy::integer_division, reason = "Game mechanic.")]
fn payout(value: Centi, factor: Milli) -> Centi {
    let scaled = u64::from(value).saturating_mul(u64::from(factor)) / PAYOUT_DIVISOR;
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// Nonet can't be active unless all 9 slots are filled, so it doesn't even
/// bother to check otherwise.
fn nonet(team: &Team, ctx: Context, team_mask: CharacterSet, all: &mut [u32; 3]) {
    for (_, slot) in team.lanes() {
        for sis in slot.sis().filter_map(|id| ctx.inventory.sis(id, ctx.sis_db)) {
            if sis.effect == SisEffect::AllPercent
                && let Some(group) = sis.group
                && let Some(set) = ctx.sis_db.targets.set(group)
                && (team_mask.0 & !set.0) == 0
            {
                add(all, sis.attribute, sis.value);
            }
        }
    }
}

/// team-wide percent SIS multiplies by the base stat, before accessory and
/// gets added right along with the flat SIS.
fn pipeline(stats: &mut [u32; 3], all: [u32; 3], sums: &Sums) {
    let lanes = stats.iter_mut().zip(all).zip(sums.percent).zip(sums.flat).zip(sums.base);
    for ((((s, a), p), f), b) in lanes {
        *s = boost(*s, p).saturating_add(f).saturating_add(portion(b, a));
    }
}

fn leader(
    ls: LeaderSkill,
    base: [u32; 3],
    who: CharacterId,
    targets: &LeaderTargets,
    out: &mut [u32; 3],
) {
    add(out, ls.attribute, share(pick(base, ls.secondary.unwrap_or(ls.attribute)), ls.value));

    if let Some(extra) = ls.extra
        && targets.set(extra.target).is_some_and(|set| set.contains(who))
    {
        add(out, extra.attribute, share(pick(base, extra.attribute), extra.value));
    }
}

/// `ceil(stat * sum/1000)`
#[expect(
    clippy::arithmetic_side_effects,
    reason = "I don't think it's possible to get anywhere close to the bound. Okay yeah I did the math
    u32 is a very big number"
)]
const fn portion(stat: u32, sum: Milli) -> u32 {
    (stat * sum).div_ceil(1000)
}

/// `ceil(stat * (1 + sum/1000))`.
const fn boost(stat: u32, sum: Milli) -> u32 {
    stat.saturating_add(portion(stat, sum))
}

/// `ceil(stat * pct/100)`
#[expect(clippy::arithmetic_side_effects, reason = "`u32 bige.")]
const fn share(stat: u32, pct: Pct) -> u32 {
    (stat * pct as u32).div_ceil(100)
}

#[expect(
    clippy::integer_division,
    reason = "You got a better way to do this, clippy!? idk should be fine."
)]
const fn natural(value: Milli) -> u32 {
    value / 1000
}

#[expect(clippy::indexing_slicing, reason = "`Attribute` has exactly these 3 variants")]
fn add(v: &mut [u32; 3], a: Attribute, n: u32) {
    v[a as usize] = v[a as usize].saturating_add(n);
}

#[expect(clippy::indexing_slicing, reason = "`Attribute` has exactly these 3 variants")]
const fn pick(v: [u32; 3], a: Attribute) -> u32 {
    v[a as usize]
}

/// A `CharacterId`'s bit. `Roster::build` caps the roster at 128 names. Which means we can always
/// shift within range.
fn bit(c: CharacterId) -> u128 {
    1_u128.checked_shl(u32::from(c.0)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        BOTH_MATCH, Compiled, MAX_TABLE_HP, MIN_TABLE_HP, NO_MATCH, ONE_MATCH, appeal, compile,
        heart_bonus, lanes, max_hearts, percent, reach, seats, waits_for,
    };
    use crate::accessory::{Accessories, tests::accessories};
    use crate::card::{Deck, Gallery, tests::cards};
    use crate::level_cap::{LevelCaps, tests::corpus};
    use crate::manager::EntryId;
    use crate::sis::{SisInventory, tests::sis};
    use crate::team::{CENTER, Context, Team};
    use simulator::accessory::AccessoryDb;
    use simulator::card::{CardDb, LeaderId, LeaderSkill};
    use simulator::scalar::{Attribute, CharacterId, CharacterSet};
    use simulator::sis::SisDb;
    use simulator::skill::SkillEffect;
    use simulator::song::Matching;

    struct Fixture {
        cards: CardDb,
        acc_db: AccessoryDb,
        sis_db: SisDb,
        caps: LevelCaps,
        deck: Deck,
        accessories: Accessories,
        inventory: SisInventory,
    }

    fn fixture() -> Fixture {
        Fixture {
            cards: cards(),
            acc_db: accessories(),
            sis_db: sis(),
            caps: corpus(),
            deck: Deck::new("d".to_owned(), Gallery::default()),
            accessories: Accessories::new("a".to_owned()),
            inventory: SisInventory::new("s".to_owned()),
        }
    }

    impl Fixture {
        fn ctx(&self) -> Context<'_> {
            Context {
                deck: &self.deck,
                accessories: &self.accessories,
                inventory: &self.inventory,
                cards: &self.cards,
                acc_db: &self.acc_db,
                sis_db: &self.sis_db,
            }
        }

        fn run(&self, team: &Team) -> Compiled {
            self.against(team, Matching { attribute: Attribute::Cool, group: None })
        }

        fn against(&self, team: &Team, song: Matching) -> Compiled {
            compile(team, self.ctx(), &self.caps, song)
        }

        fn card(&mut self, card_id: u16, idolized: bool, slots: u8) -> EntryId {
            self.deck.add(&self.cards, card_id, idolized, 1, slots, 100).unwrap().0
        }

        fn owns(&mut self, sis_id: u16) -> EntryId {
            self.inventory.add(&self.sis_db, sis_id).unwrap().0
        }
    }

    #[test]
    fn check_ceilings() {
        let mut f = fixture();
        let card = f.card(1, false, 4); // unidolized, 1 Smile
        let (petit, aura) = (f.owns(11), f.owns(19)); // 13% self, 1.5% team
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_sis(0, petit, f.ctx()).unwrap();
        team.equip_sis(0, aura, f.ctx()).unwrap();

        assert_eq!(f.run(&team).slots[0].stats[0], 3);
    }

    #[test]
    fn check_stat_pipeline_for_real_this_time() {
        let mut f = fixture();
        let card = f.card(1, true, 8); // idolized, 1021 Smile
        let (petit, aura) = (f.owns(11), f.owns(19)); // 13% self, 1.5% team
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_sis(0, aura, f.ctx()).unwrap();
        assert_eq!(f.run(&team).slots[0].stats[0], 1021 + 16, "ceil(1021 * .015)");

        team.equip_sis(0, petit, f.ctx()).unwrap();
        assert_eq!(
            f.run(&team).slots[0].stats[0],
            1154 + 16,
            "ceil(1021 * 1.13) + ceil(1021 * .015), and not ceil(ceil(1021 * 1.015) * 1.13)"
        );
    }

    #[test]
    fn check_stat_pipeline_with_acc() {
        let mut f = fixture();
        let (worn, bare) = (f.card(1, true, 8), f.card(1, true, 8));
        let acc = f.accessories.add(&f.acc_db, 7, 8).unwrap().0;
        let (mine, theirs) = (f.owns(19), f.owns(19)); // 1.5% team each, so 3%
        let mut team = Team::default();
        team.equip_card(0, worn, f.ctx()).unwrap();
        team.equip_card(1, bare, f.ctx()).unwrap();
        team.equip_accessory(0, acc, f.ctx()).unwrap();
        team.equip_sis(0, mine, f.ctx()).unwrap();
        team.equip_sis(1, theirs, f.ctx()).unwrap();

        let stats = f.accessories.stats(acc, &f.acc_db).unwrap().0[0];
        assert_eq!(stats, 17, "the fixture's level-8 accessory, which the margin depends on");

        let out = f.run(&team);
        assert_eq!(out.slots[1].stats[0], 1021 + 31, "ceil(1021 * .03), no accessory in play");
        assert_eq!(
            out.slots[0].stats[0],
            1021 + 17 + 31,
            "the 3% is ceil(1021 * .03) = 31 off the bare card, not ceil(1038 * .03) = 32"
        );
    }

    #[test]
    fn check_flat_sis_calc() {
        let mut f = fixture();
        f.cards.leaders.push(LeaderSkill {
            attribute: Attribute::Smile,
            secondary: None,
            value: 10,
            extra: None,
        });
        f.cards.cards[0].leader = LeaderId(1);

        let card = f.card(1, true, 8); // idolized, 1021 Smile
        let kiss = f.owns(7); // 200 flat Smile
        let mut team = Team::default();
        team.equip_card(CENTER, card, f.ctx()).unwrap();
        let bare = f.run(&team).slots[CENTER].stats[0];
        team.equip_sis(CENTER, kiss, f.ctx()).unwrap();
        let with = f.run(&team).slots[CENTER].stats[0];

        assert_eq!(bare, 1021 + 103, "ceil(1021 *.10");
        assert_eq!(with, 1221 + 123, "should read post-percent vals");
        assert_eq!(with - bare, 220, "flat applies more than its value");
    }

    #[test]
    fn check_trick_sis() {
        let mut f = fixture();
        f.cards.leaders.push(LeaderSkill {
            attribute: Attribute::Smile,
            secondary: None,
            value: 10,
            extra: None,
        });
        f.cards.cards[0].leader = LeaderId(1);
        let card = f.card(1, true, 8); // idolized, 1021 Smile
        let (kiss, trick) = (f.owns(7), f.owns(23)); // 200 flat Smile, +33% Smile
        let mut team = Team::default();
        team.equip_card(CENTER, card, f.ctx()).unwrap();
        team.equip_sis(CENTER, kiss, f.ctx()).unwrap();
        let smile = Matching { attribute: Attribute::Smile, group: None };
        let bare = f.against(&team, smile);
        assert_eq!(bare.trick, 0);

        team.equip_sis(CENTER, trick, f.ctx()).unwrap();
        let out = f.against(&team, smile);
        assert_eq!(out.slots[CENTER].stats, bare.slots[CENTER].stats);
        assert_eq!(out.slots[CENTER].stats[0], 1221 + 123);
        assert_eq!(out.slots[CENTER].trick, [444, 0, 0], "ceil(1344 * .33)");
        assert_eq!(out.trick, 444);
        assert_eq!(out.scoring().trick, out.trick);
        assert_eq!(out.team_strength, bare.team_strength);

        let cool = f.run(&team);
        assert_eq!(cool.slots[CENTER].trick, [444, 0, 0]);
        assert_eq!(cool.trick, 0, "Smile Trick does nothing on cool song.");
    }

    #[test]
    fn check_leader_skill_uses_center_lane() {
        let mut f = fixture();
        f.cards.leaders.push(LeaderSkill {
            attribute: Attribute::Smile,
            secondary: Some(Attribute::Cool),
            value: 10,
            extra: None,
        });
        f.cards.cards[0].leader = LeaderId(1);
        let card = f.card(1, true, 8);
        let mut team = Team::default();

        team.equip_card(0, card, f.ctx()).unwrap();
        assert_eq!(f.run(&team).slots[0].stats[0], 1021, "not the center, so no leader skill");

        team.clear(0).unwrap();
        team.equip_card(CENTER, card, f.ctx()).unwrap();
        assert_eq!(f.run(&team).slots[CENTER].stats[0], 1021 + 32, "ceil(315 * .10)");
    }

    /// The fixture card is `CharacterId(0)`, Smile.
    #[test]
    fn check_lane_bonus() {
        let mut f = fixture();
        let card = f.card(1, true, 8);
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();

        let hers = CharacterSet(1 << CharacterId(0).0);
        let cases = [
            (Attribute::Cool, None, NO_MATCH),
            (Attribute::Smile, None, ONE_MATCH),
            (Attribute::Cool, Some(hers), ONE_MATCH),
            (Attribute::Smile, Some(hers), BOTH_MATCH),
            (Attribute::Smile, Some(CharacterSet(0)), ONE_MATCH),
        ];
        for (attribute, group, want) in cases {
            let out = f.against(&team, Matching { attribute, group });
            assert_eq!(out.slots[0].bonus, want, "{attribute:?} {group:?}");
            assert_eq!(out.slots[1].bonus, NO_MATCH, "Empty lane is trivially 1.00x");
            assert_eq!(out.scoring().lanes[0], want);
        }
    }

    #[test]
    fn check_team_strength_matches_song_attribute() {
        let mut f = fixture();
        let card = f.card(1, true, 8); // 1021 Smile, 2 Pure, 315 Cool
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();

        let smile = f.against(&team, Matching { attribute: Attribute::Smile, group: None });
        let cool = f.against(&team, Matching { attribute: Attribute::Cool, group: None });
        assert_eq!(smile.team_strength, 1021, "raw team total.");
        assert_eq!(cool.team_strength, 315);
        assert_eq!(smile.scoring().team_strength, smile.team_strength);
        assert_eq!(smile.totals, cool.totals, "only the attribute read differs");
        assert_eq!(smile.scoring().stats.iter().sum::<u32>(), smile.team_strength);
        assert_eq!(cool.scoring().stats[0], 315);
    }

    #[test]
    fn check_scorer_accessory_gets_charm_sis() {
        let mut f = fixture();
        let card = f.card(1, true, 8);
        let pin = f.accessories.add(&f.acc_db, 11, 8).unwrap().0; // Scorer
        let charm = f.owns(15); // Smile Charm
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_accessory(0, pin, f.ctx()).unwrap();

        let bare = f.run(&team).slots[0].accessory;
        assert_eq!(bare.value, 8000,);
        assert_eq!(bare.chance, 3100);
        assert_eq!(bare.effect, SkillEffect::Scorer);

        team.equip_sis(0, charm, f.ctx()).unwrap();
        let charmed = f.run(&team).slots[0].accessory;
        assert_eq!(charmed.value, 20_000);
        assert_eq!(charmed.chance, bare.chance);
        assert_eq!(f.run(&team).scoring().accessories[0], charmed);
        assert_eq!(f.run(&team).slots[0].skill.value, 2500, "and the card's own 1000");
    }

    /// The fixture has a scorer acc with a value of 1000 a level for 8 level
    /// It also has a Healer that heals 1 per level.
    /// Amp only goes to level 8 here.
    #[test]
    fn check_amplified_rows() {
        let mut f = fixture();
        let card = f.card(1, true, 8);
        let pin = f.accessories.add(&f.acc_db, 11, 4).unwrap().0;
        let charm = f.owns(15);
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_accessory(0, pin, f.ctx()).unwrap();
        team.equip_sis(0, charm, f.ctx()).unwrap();

        let slot = f.run(&team).slots[0];
        assert_eq!(slot.accessory.value, 10_000);
        let worn: Vec<u32> = slot.amplified[1].iter().map(|row| row.value).collect();
        assert_eq!(worn[..5], [12_500, 15_000, 17_500, 20_000, 20_000]);
        assert!(worn[4..].iter().all(|&v| v == 20_000), "only up to level 8");
        assert!(slot.amplified[0].iter().all(|row| row.value == slot.skill.value), "one level");
        assert_eq!(f.run(&team).scoring().amplified[0], slot.amplified);
    }

    /// Healer acc at sl 3 heals 4 at +1 and 8 from +5.
    #[test]
    fn check_amplified_heals() {
        let mut f = fixture();
        let healer = f.card(4, true, 8);
        let clef = f.accessories.add(&f.acc_db, 15, 3).unwrap().0;
        let heal = f.owns(27);
        let mut team = Team::default();
        team.equip_card(0, healer, f.ctx()).unwrap();
        team.equip_accessory(0, clef, f.ctx()).unwrap();
        team.equip_sis(0, heal, f.ctx()).unwrap();
        let slot = f.run(&team).slots[0];
        let hp: Vec<u16> = slot.amplified[1].iter().map(|row| row.extra).collect();
        assert_eq!(hp[..6], [4, 5, 6, 7, 8, 8]);
        assert_eq!(slot.amplified[1][0].value, 4 * 480);
    }

    /// card heals 4, acc 8.
    /// So 1920 and 3840 points respectively with Heal SIS.
    #[test]
    fn check_heal_sis() {
        let mut f = fixture();
        let card = f.card(4, true, 8);
        let clef = f.accessories.add(&f.acc_db, 15, 8).unwrap().0;
        let heal = f.owns(27);
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_accessory(0, clef, f.ctx()).unwrap();

        let bare = f.run(&team).slots[0];
        assert_eq!((bare.skill.effect, bare.skill.value), (SkillEffect::Healer, 0));
        assert_eq!((bare.accessory.effect, bare.accessory.value), (SkillEffect::Healer, 0));
        assert_eq!((bare.skill.chance, bare.accessory.chance), (3000, 3000), "both still roll");

        team.equip_sis(0, heal, f.ctx()).unwrap();
        let healed = f.run(&team).slots[0];
        assert!(healed.sis.heal() && !healed.sis.charm());
        assert_eq!(healed.skill.value, 4 * 480);
        assert_eq!(healed.accessory.value, 8 * 480, "acc inherits Heal SIS");

        team.unequip_accessory(0).unwrap();
        let pin = f.accessories.add(&f.acc_db, 11, 8).unwrap().0;
        team.equip_accessory(0, pin, f.ctx()).unwrap();
        assert_eq!(f.run(&team).slots[0].accessory.value, 8000, "Heal does not reach a Scorer");
    }

    /// The fixture's card 4 heals 4 HP and accessory 15 heals 8 at level 8.
    /// Two idolized fixture cards are 6 HP each, summing to 12. 0.34% per heart.
    #[test]
    fn check_overheal_inputs() {
        let mut f = fixture();
        let (healer, scorer) = (f.card(4, true, 8), f.card(1, true, 8));
        let clef = f.accessories.add(&f.acc_db, 15, 8).unwrap().0;
        let mut team = Team::default();
        team.equip_card(0, healer, f.ctx()).unwrap();
        team.equip_accessory(0, clef, f.ctx()).unwrap();
        assert_eq!(f.run(&team).max_hearts, 0, "6 HP is not on heal table");

        team.equip_card(1, scorer, f.ctx()).unwrap();
        let out = f.run(&team);
        assert_eq!(out.slots[0].heals, [4, 8]);
        assert_eq!(out.slots[1].heals, [0, 0], "Scorers do not heal, silly");
        assert_eq!((out.max_hp, out.per_heart, out.max_hearts), (12, 34, 589));

        let scoring = out.scoring();
        assert_eq!(scoring.heals[0], [4, 8]);
        assert_eq!(scoring.heals[1], [0, 0]);
        assert_eq!((scoring.max_hp, scoring.per_heart, scoring.max_hearts), (12, 34, 589));
    }

    /// The fixture's Spark is threshold 5 at 11% for 1000 points per level.
    /// Note that the raw data uses smaller values than what actually get applied, but the preprocessor
    /// crate has actually already multiplied by the threshold, so this is just conversion to points.
    #[test]
    fn check_spark() {
        let mut f = fixture();
        let card = f.card(1, true, 8);
        let medal = f.accessories.add(&f.acc_db, 13, 3).unwrap().0;
        let charm = f.owns(15);
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_accessory(0, medal, f.ctx()).unwrap();
        team.equip_sis(0, charm, f.ctx()).unwrap();

        let out = f.run(&team);
        let worn = out.slots[0].accessory;
        assert_eq!(
            (worn.effect, worn.value, worn.chance),
            (SkillEffect::Spark, 3000, 1100),
            "no Charm"
        );
        let rows: Vec<u32> = out.slots[0].amplified[1].iter().map(|row| row.value).collect();
        assert_eq!(rows[..6], [4000, 5000, 6000, 7000, 8000, 8000]);
        assert_eq!(out.scoring().sparks, [5, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// It would be bad if accessories used the card's skill level because someone made an oopsie.
    #[test]
    fn check_an_accessory_skill_level() {
        let mut f = fixture();
        let card = f.card(1, true, 8);
        let low = f.accessories.add(&f.acc_db, 11, 3).unwrap().0;
        let mut team = Team::default();
        team.equip_card(0, card, f.ctx()).unwrap();
        team.equip_accessory(0, low, f.ctx()).unwrap();

        assert_eq!(f.run(&team).slots[0].accessory.value, 3000, "300_000 Centi at level 3");
    }

    #[test]
    fn check_seats() {
        let (riko, you) = (Some(CharacterId(7)), Some(CharacterId(3)));
        let chars = [None, riko, you, riko, None, you, None, None, riko];
        assert_eq!(seats(&chars), [0, 1 << 1, 1 << 2, 1 << 1, 0, 1 << 2, 0, 0, 1 << 1]);
    }

    /// Riko waits for Chika and You. Team without You means Riko can't proc.
    #[test]
    fn check_chain_needs_the_rest_of_its_year() {
        let (chika, you, riko, dia) =
            (CharacterId(1), CharacterId(2), CharacterId(3), CharacterId(9));
        let year = CharacterSet((1 << 1) | (1 << 2) | (1 << 3));
        let chars = [
            Some(riko),
            Some(dia),
            Some(chika),
            Some(riko),
            Some(you),
            Some(chika),
            None,
            None,
            None,
        ];
        let seated = seats(&chars);
        assert_eq!(waits_for(year, riko, &chars, &seated), (1 << 2) | (1 << 4), "first Chika, You");
        assert_eq!(waits_for(year, you, &chars, &seated), (1 << 2) | 1, "first Chika, first Riko");

        let no_you = chars.map(|c| c.filter(|&c| c != you));
        assert_eq!(waits_for(year, riko, &no_you, &seats(&no_you)), 0);
        let only_riko = CharacterSet(1 << 3);
        assert_eq!(waits_for(only_riko, riko, &chars, &seated), 0, "誰もいない");
    }

    #[test]
    fn check_heart_cap() {
        let bonus = heart_bonus(54);
        let cap = max_hearts(bonus);
        assert_eq!((bonus, cap), (338, 60), "54 max hp, +3.38% per heart");
        assert_eq!(u32::from(cap) * u32::from(bonus), 20_280, "+202.8%, but clamped to +200%");

        for hp in MIN_TABLE_HP..=MAX_TABLE_HP {
            let (b, n) = (u32::from(heart_bonus(hp)), u32::from(max_hearts(heart_bonus(hp))));
            assert!((n - 1) * b < 20_000, "{n} hearts at {hp} hp is more than one step past");
            assert!(n * b >= 20_000, "{n} hearts at {hp} hp does not reach +200%");
        }
        assert_eq!(max_hearts(heart_bonus(0)), 0, "empty team = no hearts4u");
    }

    /// Transcription of real recording.
    /// Smile 541, 311, 759, 2024, 9754, 1614 and 1321*3
    /// 18,966 displayed team strength
    /// +215% on all nine lanes from kotoeli's skill
    #[test]
    fn check_appeal_boost() {
        assert_eq!(percent(541, 215), 1164, "1163.15, up");
        assert_eq!(percent(1000, 215), 2150, "exact");
        assert_eq!(percent(1000, 0), 0);

        let strengths = [541, 311, 759, 2024, 9754, 1614, 1321, 1321, 1321];
        assert_eq!(strengths.iter().sum::<u32>(), 18_966);
        let (minami, honoka, eli, kotori) =
            (CharacterId(40), CharacterId(1), CharacterId(2), CharacterId(3));
        let chars = [
            Some(minami),
            Some(CharacterId(41)),
            Some(CharacterId(42)),
            Some(honoka),
            Some(kotori),
            Some(eli),
            Some(kotori),
            Some(kotori),
            Some(kotori),
        ];
        let all = CharacterSet((1 << 43) - 1);
        let boost = appeal(215, Some(all), &chars, &strengths);
        assert_eq!(boost, 40_783, "a ceiling a lane");
        assert!((59_716..=59_760).contains(&(18_966 + boost)));

        let muse = CharacterSet((1 << 1) | (1 << 2) | (1 << 3));
        assert_eq!(appeal(215, Some(muse), &chars, &strengths), 4352 + 20_972 + 3471 + 3 * 2841);
        let only_kotori = CharacterSet(1 << 3);
        assert_eq!(appeal(100, Some(only_kotori), &chars, &strengths), 9754 + 3 * 1321);
        assert_eq!(appeal(215, None, &chars, &strengths), 0, "no target");
        let empty = chars.map(|c| c.filter(|&c| c != kotori));
        assert_eq!(appeal(100, Some(only_kotori), &empty, &strengths), 0, "empty lanes");
    }

    /// Two Planetarium Nozomis in lanes 3 and 5 with Cheer Girl Umi (the UR. This isn't the infamous cheer umi score-based scorer.
    /// it's a different cheer umi with a lily white-boosting param skill) in lane 6, a
    /// Nico in lane 0, an N card in lane 2, and three empty lanes.
    #[test]
    fn check_sync_targets() {
        let strengths = [4000, 0, 2000, 3270, 0, 2990, 4968, 0, 0];
        let (nico, nozomi, umi) = (CharacterId(7), CharacterId(5), CharacterId(8));
        let chars = [
            Some(nico),
            None,
            Some(CharacterId(40)),
            Some(nozomi),
            None,
            Some(nozomi),
            Some(umi),
            None,
            None,
        ];
        let third_years = Some(CharacterSet((1 << 5) | (1 << 6) | (1 << 7)));
        let all = Some(CharacterSet((1 << 43) - 1));

        let mut none = 0;
        let card = reach(SkillEffect::Sync, &mut none, third_years, 1 << 3, &chars, &strengths);
        assert_eq!(card, ((1 << 0) | (1 << 5), 0));
        let worn = reach(SkillEffect::Sync, &mut none, all, 1 << 3, &chars, &strengths);
        assert_eq!(worn, ((1 << 0) | (1 << 2) | (1 << 5) | (1 << 6), 0));
        assert_eq!(none, 0, "Sync has no value field");

        // Param Boost boosts its own lane and keeps its percent beside its points.
        let lily_white = Some(CharacterSet((1 << 5) | (1 << 8)));
        let mut points = 32;
        let boost =
            reach(SkillEffect::AppealBoost, &mut points, lily_white, 1 << 6, &chars, &strengths);
        assert_eq!(boost, ((1 << 3) | (1 << 5) | (1 << 6), 32));
        assert_eq!(points, 1047 + 957 + 1590);

        let mut scored = 500;
        assert_eq!(reach(SkillEffect::Scorer, &mut scored, all, 1, &chars, &strengths), (0, 0));
        assert_eq!(scored, 500);
        assert_eq!(lanes(None, &chars), 0, "no target");
    }
}
