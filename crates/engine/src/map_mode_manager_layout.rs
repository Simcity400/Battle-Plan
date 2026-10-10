//! Map-mode switch receiver and presentation selectors.
//! Gradient-border state belongs to the separate object reached through
//! `GRADIENT_BORDER_MANAGER`. These fields do not enable an hourly state walk.

/// Map-mode switch entry.
pub const SET_MAP_MODE_RVA: usize = 0x00e0_3630;
/// Current map-mode i32 ordinal.
pub const CURRENT_MODE: usize = 0;
/// Gradient-border manager pointer used for layer selection.
pub const GRADIENT_BORDER_MANAGER: usize = 0x30;
/// First signed i32 map selector.
pub const STATE_50: usize = 0x50;
/// Second signed i32 map selector.
pub const STATE_54: usize = 0x54;
