//! Directional attache link writers; their concrete receiver type is unknown.
//! These directional links are separate from captured attache relation rows.

/// Applies both directional attache slots reciprocally.
pub const APPLY_LINK_RVA: usize = 0x00d1_2da0;
/// Clears both directional attache slots.
pub const CLEAR_LINK_RVA: usize = 0x00d1_1890;
