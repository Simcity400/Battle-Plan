//! Entity-description presentation layout; no live table is enabled.

/// Entity-description initializer.
pub const INITIALIZER_RVA: usize = 0x0228_13b0;
/// Initialized description size.
pub const SIZE: usize = 0x460;
/// Unidentified byte, initialized to true.
pub const DEFAULT_TRUE_FLAG: usize = 0x40c;
/// Authored gate for rate updates; creation does not use it.
pub const ANIMATION_RATE_AUTHORED: usize = 0x450;
/// Animation rate f32, default 1.0.
pub const ANIMATION_RATE: usize = 0x454;
/// Unidentified field assigned native f32 bits for 1.0 by the initializer.
pub const DEFAULT_FLOAT_440: usize = 0x440;
/// Unidentified field assigned native f32 bits for 1.0 by the initializer.
pub const DEFAULT_FLOAT_44C: usize = 0x44c;
/// Freshness field compared with the definition; width is unspecified.
pub const VERSION_COMPARE_FIELD: usize = 0x444;
