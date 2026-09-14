pub const MIN_LEVEL: u8 = 1;
pub const MAX_LEVEL: u8 = simulator::skill::MAX_BASE_LEVEL;

#[must_use]
pub const fn in_range(skill_level: u8) -> bool {
    MIN_LEVEL <= skill_level && skill_level <= MAX_LEVEL
}

#[must_use]
pub const fn default() -> u8 {
    MIN_LEVEL
}
