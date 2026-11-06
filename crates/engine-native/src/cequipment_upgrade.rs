//! Authored CEquipmentUpgrade definitions referenced by variant upgrade entries.

use engine_sites::Rva;
/// CEquipmentUpgrade primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a4_e598);
/// CEquipmentUpgrade signed token.
pub const TOKEN: usize = 0x08;
/// CEquipmentUpgrade MSVC name.
pub const NAME: usize = 0x18;
/// Immutable authored XP currency: land, naval or air.
pub const COST_CURRENCY: usize = 0xe0;
