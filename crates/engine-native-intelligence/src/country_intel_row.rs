
use engine_sites::Rva;
pub const STRIDE: usize = 0x20;
pub const CIVILIAN: usize = 0x00;
pub const ARMY: usize = 0x08;
pub const NAVY: usize = 0x10;
pub const AIR: usize = 0x18;

pub const WRITER_RVA: Rva = Rva(0x0112_25b0);
pub const LANE_DENOMINATOR: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner_country: u32,
    pub target_index: u32,
    pub army: i64,
    pub air: i64,
    pub civilian: i64,
    pub navy: i64,
}

impl Row {
    pub fn from_bytes(owner_country: u32, target_index: u32, bytes: &[u8; STRIDE]) -> Self {
        let lane =
            |offset: usize| i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        Self {
            owner_country,
            target_index,
            army: lane(ARMY),
            air: lane(AIR),
            civilian: lane(CIVILIAN),
            navy: lane(NAVY),
        }
    }

    pub fn is_zero(&self) -> bool {
        self.army == 0 && self.air == 0 && self.civilian == 0 && self.navy == 0
    }
}
