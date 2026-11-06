//! Ship-refit production line; inherited fields are in
//! `cequipment_production_line` and `cproduction_line`.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029f_8c60);

/// Registry identity kind for ship-refit work orders.
pub const REFERENCE_TYPE: u32 = 71;
