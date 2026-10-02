//! The things that can change between each run aka accuracy and approach rate.
//! Seed also lives here.

use crate::scalar::{RATE_UNIT, Rate};
use crate::song::NOTE_SPEEDS;

/// The in-game approach rate, 1 (slowest) to 10 (fastest).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct NoteSpeed(u8);

/// See `data/note_speed_map.json`.
impl NoteSpeed {
    pub const SLOWEST: Self = Self(1);
    pub const FASTEST: Self = Self(10);

    /// # Errors
    /// A setting outside 1..=10, which is not a speed the game offers.
    #[must_use]
    pub const fn new(setting: u8) -> Option<Self> {
        if setting < Self::SLOWEST.0 || setting > Self::FASTEST.0 {
            return None;
        }
        Some(Self(setting))
    }

    #[must_use]
    pub const fn setting(self) -> u8 {
        self.0
    }

    /// Travel time is in ms for the record.
    #[must_use]
    #[expect(clippy::indexing_slicing, reason = "1..=10")]
    pub const fn offset(self) -> u32 {
        NOTE_SPEEDS[self.0.saturating_sub(1) as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayConfig {
    pub speed: NoteSpeed,

    /// The chance a judgement comes out PERFECT rather than GREAT.
    ///
    /// Note that GOOD, BAD, and MISS are not being modeled. I see no reason to
    /// model it. I mean it's possible to model but I also don't see much value in it.
    /// I mean that's not a good argument. There's not much value in this project regardless.
    /// The game is dead.
    pub accuracy: Rate,
    pub seed: u64,
}

impl PlayConfig {
    /// aka accuracy = 1.0.
    /// Assumes nothing about perfect locks. Obviously an AP is possible with sub-perfect
    /// accuracy with perfect locks existing. This is just saying to not roll accuracy at all,
    /// just call it a perfect.
    pub const ALL_PERFECT: Rate = RATE_UNIT;

    #[must_use]
    #[expect(clippy::similar_names, reason = "Are `speed` and `seed` even really that similar?")]
    pub const fn new(speed: NoteSpeed, accuracy: Rate, seed: u64) -> Self {
        Self { speed, accuracy, seed }
    }
}

#[cfg(test)]
mod tests {
    use super::{NoteSpeed, PlayConfig};

    #[test]
    fn check_speed_setting() {
        for setting in 1..=10 {
            assert!(NoteSpeed::new(setting).is_some());
        }
        assert_eq!(NoteSpeed::new(0), None);
        assert_eq!(NoteSpeed::new(11), None);
    }

    #[test]
    fn check_the_earliest_spawn_is_positive() {
        // It would be bad if a note spawned before the song started after applying
        // the note speed modifier.
        let config = PlayConfig::new(NoteSpeed::SLOWEST, PlayConfig::ALL_PERFECT, 0);
        assert_eq!(crate::song::Time(2047).spawn(config.speed.offset()).0, 297);
    }
}
