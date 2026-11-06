//! Force-government peace participant identity.
use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a3_c9d8);
/// Country reference used by government and stackable operations after country selection.
pub const RESULT_COUNTRY: usize = 0x60;
