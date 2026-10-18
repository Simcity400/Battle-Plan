//! `CDate`, the raw date base inherited by both Gregorian and game dates.

/// CDate native signed hour i32.
pub const RAW_HOUR: usize = 0;
/// Raw date hour epoch subtracted from the signed stored hour.
pub const HOUR_EPOCH: i32 = 43_800_000;
/// Gregorian month-length table, twelve i32s.
pub const MONTH_LENGTHS_RVA: usize = 0x0306_d210;
/// Gregorian month lengths.
pub const MONTH_LENGTHS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
