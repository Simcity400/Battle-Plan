
use engine_sites::Rva;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub priority: i32,
    pub name_start: usize,
    pub name_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub orders_group: u64,
    pub theater_group: u64,
    pub ordinal: u32,
}

pub const VTABLE_RVA: Rva = Rva(0x02a5_f240);
pub const REFERENCE_TYPE: u32 = 68;
pub const HANDLE: usize = 0x08;
pub const THEATRE: usize = 0x18;
pub const PRIORITY: usize = 0x20;
pub const NAME: usize = 0x28;
pub const ORDERS_GROUP_DATA: usize = 0x48;
pub const SIZE: usize = 0x60;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x016d_8ed0);
