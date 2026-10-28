//! `CGameGui`, the game-specific CGui manager.
//!
//! The ordinary container factory and registration operations are inherited
//! from CGui; this identity does not prove arbitrary root-graph ancestry.

/// CGameGui primary vtable.
pub const VTABLE_RVA: usize = 0x029c_7550;
/// CGameGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x029c_7628;
/// CGameGui constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00b9_4930;
