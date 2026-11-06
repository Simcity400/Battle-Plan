//! Null unit-leader skill sentinel with the skill layout.

use engine_sites::Rva;
/// Null skill-sentinel vtable.
pub const VTABLE_RVA: Rva = Rva(0x029c_29e0);
/// Null skill-sentinel pointer global, distinct from the database.
pub const SINGLETON_RVA: Rva = Rva(0x033e_7c58);
