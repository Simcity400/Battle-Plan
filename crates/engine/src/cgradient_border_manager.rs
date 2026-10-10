//! CGradientBorderManager presentation layout.
//!
//! The +0x49 flag belongs to this manager. The outer map-mode receiver
//! separately owns the signed +0x50/+0x54 state.

/// Gradient-manager diplomacy flag byte.
pub const DIPLOMACY_FLAG: usize = 0x49;
/// Selected layer resolver through the type-indexed layer pointer array.
pub const SELECTED_LAYER_RVA: usize = 0x00f1_c3a0;
