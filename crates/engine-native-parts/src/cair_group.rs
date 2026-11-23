
use engine_sites::Rva;
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub name_start: usize,
    pub name_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberRow {
    pub wing: u64,
    pub group: u64,
    pub ordinal: u32,
}

pub const VTABLE_RVA: Rva = Rva(0x02a4_6a20);
pub const TYPE: u32 = 78;
pub const HANDLE: usize = 0x08;
pub const NAME: usize = 0x50;
pub const THEATRE_HANDLE: usize = 0x70;
pub const WINGS: usize = 0x78;
