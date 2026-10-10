//! Separate badge source owner holding raw specialization tokens.
//! Shared offsets do not establish identity with research state.

/// Specialization-token array data pointer.
pub const SPECIALIZATION_DATA: usize = 0x140;
/// Specialization-token count, u32.
pub const SPECIALIZATION_COUNT: usize = 0x14c;
/// Token element width for little-endian u32 values.
pub const SPECIALIZATION_STRIDE: usize = 4;
