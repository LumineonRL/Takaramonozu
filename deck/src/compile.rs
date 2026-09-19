//! The stat calculation pipeline for a Team.
//!
//! Special thanks to none other than me from 10 years ago, coming in clutch
//! with a spreadsheet I built to calculate team stats, as it helped present me
//! fix a bug in the calculation order.
//! See https://docs.google.com/spreadsheets/d/1ecVUlSYMU5oBAvSbhgmbUjPlJZTklf2OF6_s3MY4dTM
//! for a historic relic of just how long I've been wanting to "solve" SIF.

use core::num::NonZeroU8;

use simulator::card::{LeaderSkill, LeaderTargets};
use simulator::scalar::{Attribute, CharacterId, CharacterSet, Milli, Pct};
use simulator::sis::SisEffect;

use crate::level_cap::LevelCaps;
use crate::manager::Manager;
use crate::team::{CENTER, Context, LANES, Team};

/// The tap-score total the heart count is capped one step past.
const OVERHEAL_TARGET: u32 = 20_000;

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

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
#[repr(C)]
pub struct Slot {
    pub stats: [u32; 3],
    pub sis: SisFlags,
    _pad: [u8; 3],
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Compiled {
    pub slots: [Slot; LANES],
    pub totals: [u32; 3],
    pub max_hp: u8,
    pub per_heart: u16,
    pub max_hearts: u16,
    pub characters: CharacterSet,
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
}

#[must_use]
pub fn compile(team: &Team, ctx: Context, caps: &LevelCaps) -> Compiled {
    let mut out = Compiled::default();
    let mut sums = [Sums::default(); LANES];
    let mut chars = [None; LANES];
    let mut all = [0_u32; 3];
    let mut grouped = false;
    let mut center = None;

    for (lane, ((dst, lane_sums), who)) in
        out.slots.iter_mut().zip(&mut sums).zip(&mut chars).enumerate()
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
                SisEffect::Trick => {}
            }
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
    }

    for slot in &out.slots {
        for (t, s) in out.totals.iter_mut().zip(slot.stats) {
            *t = t.saturating_add(s);
        }
    }
    out.per_heart = heart_bonus(out.max_hp);
    out.max_hearts = max_hearts(out.per_heart);
    out
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
        Compiled, HEART_BONUS, MAX_TABLE_HP, MIN_TABLE_HP, Slot, compile, heart_bonus, max_hearts,
    };
    use crate::accessory::{Accessories, tests::accessories};
    use crate::card::{Deck, Gallery, tests::cards};
    use crate::level_cap::{LevelCaps, tests::corpus, tests::meta};
    use crate::manager::EntryId;
    use crate::sis::{SisInventory, tests::sis};
    use crate::team::{CENTER, Context, Team};
    use simulator::accessory::AccessoryDb;
    use simulator::card::{Card, CardDb, LeaderId, LeaderSkill};
    use simulator::scalar::{Attribute, CharacterId, Stats};
    use simulator::sis::SisDb;

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
            compile(team, self.ctx(), &self.caps)
        }

        fn card(&mut self, card_id: u16, idolized: bool, slots: u8) -> EntryId {
            self.deck.add(&self.cards, card_id, idolized, 1, slots, 100).unwrap().0
        }

        fn owns(&mut self, sis_id: u16) -> EntryId {
            self.inventory.add(&self.sis_db, sis_id).unwrap().0
        }
    }

    fn nine_characters() -> CardDb {
        let mut db = cards();
        db.cards = (0..9)
            .map(|i| Card {
                stats: [Stats([100, 0, 0]); 2],
                skill: None,
                leader: LeaderId(0),
                character: CharacterId(i),
                attribute: Attribute::Smile,
            })
            .collect();
        db.meta = (1..=9).map(|id| meta(id, false)).collect();
        db
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

    #[test]
    fn check_heart_cap() {
        let bonus = heart_bonus(54);
        let cap = max_hearts(bonus);
        assert_eq!((bonus, cap), (338, 60), "54 max hp, +3.38% per heart");
        assert_eq!(u32::from(cap) * u32::from(bonus), 20_280, "+202.8%");

        for hp in MIN_TABLE_HP..=MAX_TABLE_HP {
            let (b, n) = (u32::from(heart_bonus(hp)), u32::from(max_hearts(heart_bonus(hp))));
            assert!((n - 1) * b < 20_000, "{n} hearts at {hp} hp is more than one step past");
            assert!(n * b >= 20_000, "{n} hearts at {hp} hp does not reach +200%");
        }
        assert_eq!(max_hearts(heart_bonus(0)), 0, "empty team = no hearts4u");
    }
}
