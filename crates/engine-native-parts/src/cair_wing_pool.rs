
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_77d8);
pub const SIZE: usize = 0x48;
pub const AIR_BASE_REFERENCE: usize = 0x18;
pub const DEFINITION: usize = 0x20;
pub const WINGS: usize = 0x28;
#[cfg(any(test, feature = "test-support"))]
pub const WINGS_COUNT: usize = 0x34;
pub const OWNER: usize = 0x40;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00fd_e990);
pub const DESTRUCTOR_RVA: Rva = Rva(0x00fd_ef50);
