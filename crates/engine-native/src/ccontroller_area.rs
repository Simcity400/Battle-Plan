
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029d_b198);
pub const REPRESENTATIVE_PROVINCE: usize = 0x28;
pub const PROVINCE_DATA: usize = 0x30;
pub const PROVINCE_COUNT: usize = 0x3c;
pub const SIZE: usize = 0x118;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00d5_1d20);
pub const CLEAR_RVA: Rva = Rva(0x00d5_4570);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub theatre: u64,
    pub ordinal: u32,
    pub representative: Option<u32>,
    pub provinces_start: usize,
    pub provinces_len: usize,
}
