
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029e_f400);

pub const SIZE: usize = 0x70;
pub const PROVINCE_RECORD: usize = 0x08;
pub const BUILDING_STATUS: usize = 0x10;
pub const BUILDING: usize = 0x18;
pub const LEVELS_BY_NEIGHBOR_INDEX: usize = 0x20;
pub const LEVEL_COUNT: usize = 0x2c;
pub const NEIGHBOR_PAIRS: usize = 0x38;
pub const NEIGHBOR_PAIR_COUNT: usize = 0x44;
pub const PENDING_CONSTRUCTION: usize = 0x50;
pub const COOLDOWN_TICKS: usize = 0x68;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00f0_4f70);
pub const SET_RAILWAY_RVA: Rva = Rva(0x00f0_8540);
pub const LOAD_TOKEN_RVA: Rva = Rva(0x00f0_7ed0);
pub const DESTRUCTOR_RVA: Rva = Rva(0x00f0_5420);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: u32,
    pub cooldown_ticks: i32,
    pub building_bound: bool,
}

pub fn building_bound(info: &[u8]) -> bool {
    info[BUILDING..BUILDING + 8] != [0; 8]
}
