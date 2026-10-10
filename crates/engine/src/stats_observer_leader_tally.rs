//! Combat-observer leader tally at +0x20. Selecting a cached last-nonnull
//! entry does not establish current-leader freshness.

/// Entry stride.
pub const STRIDE: usize = 0x0c;
/// Leader handle, u64; pointer identity is not inferred.
pub const HANDLE: usize = 0;
/// Tally count, u32.
pub const TALLY: usize = 8;
