
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029d_5030);
pub const SIZE: usize = 0xf8;
pub const SUPPLY_RATIO_RAW: usize = 0x30;
pub const REFERENCE_CANDIDATE: usize = 0xa8;
pub const COUNTRY: usize = 0xb0;
pub const POOLS: usize = 0xb8;
pub const POOLS_COUNT: usize = 0xc4;
pub const CAPACITY_RAW: usize = 0xd0;
#[cfg(any(test, feature = "test-support"))]
pub const INITIAL_CAPACITY_RAW: i64 = 100_000;
pub const SORTIE_CAPACITY_FACTOR_RAW: usize = 0xd8;
pub const FUEL_DEMAND_RAW: usize = 0xe0;
pub const FUEL_ALLOCATED_RAW: usize = 0xf0;
