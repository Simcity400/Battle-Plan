use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029f_7748);

pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029f_7780);

pub const SECONDARY: usize = 0x10;

pub const REFERENCE_TYPE: usize = 0x18;

pub const POOL: usize = 0x38;
pub const TYPE: u32 = 69;
pub const GROUP_HANDLE: usize = 0x40;
