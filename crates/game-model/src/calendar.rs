//! `CDate`, the raw date base inherited by both Gregorian and game dates.

/// Raw date hour epoch subtracted from the signed stored hour.
pub const HOUR_EPOCH: i32 = 43_800_000;
/// Gregorian month lengths.
pub const MONTH_LENGTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
