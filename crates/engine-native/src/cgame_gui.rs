//! `CGameGui`, the game-specific CGui manager.
//!
//! The ordinary container factory and registration operations are inherited
//! from CGui; this identity does not prove arbitrary root-graph ancestry.

use engine_sites::Rva;
/// CGameGui primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029c_7580);
/// CGameGui secondary vtable.
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029c_7658);
/// CGameGui constructor.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00b9_49a0);
