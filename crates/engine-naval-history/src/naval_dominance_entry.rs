//! Country-keyed entry containing a dominance-values subobject.

/// Naval-dominance array-entry stride.
pub const STRIDE: usize = 0x50;
/// Naval-dominance entry u32 country key.
pub const COUNTRY_KEY: usize = 0x00;
/// Naval-dominance values subobject.
pub const VALUES: usize = 0x08;
