//! `CGameGui`, the game-specific CGui manager.
//!
//! The ordinary container factory and registration operations are inherited
//! from CGui; this identity does not prove arbitrary root-graph ancestry.

/// CGameGui primary vtable.
pub const VTABLE_RVA: usize = 0x0294_a688;
/// CGameGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0294_a760;
/// CGameGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00b4_29a0;
