//! Skill effects. What a skill does once it activates.

use super::Outcome;
use crate::scalar::Centi;
use crate::skill::SkillEffect;

/// TODO: every other effect.
#[inline]
pub(super) fn pay(effect: SkillEffect, value: Centi, out: &mut Outcome) {
    if effect == SkillEffect::Scorer {
        out.skill = out.skill.saturating_add(u64::from(value));
    }
}
