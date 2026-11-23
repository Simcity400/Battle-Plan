
use engine_sites::Rva;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    pub name_start: usize,
    pub name_len: usize,
}

pub const VTABLE_RVA: Rva = Rva(0x0286_4790);
pub const TYPE: u32 = 77;
pub const HANDLE: usize = 0x08;
pub const COUNTRY: usize = 0x18;
pub const NAME: usize = 0x20;
pub const GROUPS: usize = 0x40;
pub const GROUPS_COUNT: usize = 0x4c;
