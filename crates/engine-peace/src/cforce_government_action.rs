//! CForceGovernmentAction layout for the hash-pinned executable.

use engine_sites::Rva;
/// Force-government action primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a4_cb10);
/// Raw country operand. The 0x58-byte action has no puppet auxiliary after it.
pub const COUNTRY: usize = 0x50;
