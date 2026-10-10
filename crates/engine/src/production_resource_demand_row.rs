//! Resource-demand entry retaining two signed fixed-point integers before display summation.

/// Resource-demand entry stride.
pub const STRIDE: usize = 0x20;
/// Strategic-resource pointer, authenticated by index.
pub const RESOURCE: usize = 0x08;
/// First signed i64 resource amount.
pub const FIRST_AMOUNT: usize = 0x10;
/// Second signed i64 resource amount.
pub const SECOND_AMOUNT: usize = 0x18;
