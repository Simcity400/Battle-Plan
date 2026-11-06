//! CIdeologyGroup definition shared by country identity and politics.

use engine_sites::Rva;
/// Ideology-group primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029b_a288);
/// CIdeologyGroup native u32 identity.
pub const ID: usize = 0x08;
/// CIdeologyGroup token std::string.
pub const TOKEN: usize = 0x18;
