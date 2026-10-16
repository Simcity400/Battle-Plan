//! Robin-Hood hash nodes in variable storage.

/// Node stride.
pub const STRIDE: usize = 0x30;
/// Probe distance; zero is empty, occupied distances are one-based.
pub const DISTANCE: usize = 0x04;
/// Stored string key.
pub const KEY: usize = 0x08;
/// Signed i64 value.
pub const VALUE: usize = 0x28;
/// Fixed-point value divisor.
pub const VALUE_SCALE: i64 = 100_000;
