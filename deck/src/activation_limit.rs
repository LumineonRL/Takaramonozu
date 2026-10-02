//! Per-live activation limits on the four Score-triggered cards.
//! Finding these values was a nightmare. Documentation was weak towards the end
//! SIF's life.

use core::num::NonZeroU8;

/// 90 `Animal Nico`, every 15,000.
const PANDA_NICO: u8 = 10;

/// 107 `Animal Honoka`, every 12,000.
const BEAR_HONOKA: u8 = 12;

/// 162 `October (Sports / Cheerleader) Umi`, every 13,500.
const CHEER_UMI: u8 = 13;

/// 182 `Health Committee (Nurse) Kotori`, every 18,000.
const NURSE_KOTORI: u8 = 8;

#[must_use]
pub const fn of(card_id: u16) -> Option<NonZeroU8> {
    NonZeroU8::new(match card_id {
        90 => PANDA_NICO,
        107 => BEAR_HONOKA,
        162 => CHEER_UMI,
        182 => NURSE_KOTORI,
        _ => 0,
    })
}

#[cfg(test)]
mod tests {
    use core::num::NonZeroU8;

    use super::of;

    #[test]
    fn check_the_four_limits() {
        assert_eq!(of(90).map(NonZeroU8::get), Some(10), "Animal Nico");
        assert_eq!(of(107).map(NonZeroU8::get), Some(12), "Animal Honoka");
        assert_eq!(of(162).map(NonZeroU8::get), Some(13), "Cheerleader Umi");
        assert_eq!(of(182).map(NonZeroU8::get), Some(8), "Nurse Kotori");
        for card_id in [0, 1, 89, 91, 106, 108, 161, 163, 181, 183, 206, 3911] {
            assert_eq!(of(card_id), None, "card {card_id}");
        }
    }

    /// "they are capped to only 8~13 procs"
    #[test]
    fn check_the_limits_span_the_quoted_range() {
        let limits = [90, 107, 162, 182].map(|id| of(id).map_or(0, NonZeroU8::get));
        assert_eq!(limits.iter().min(), Some(&8));
        assert_eq!(limits.iter().max(), Some(&13));
    }
}
