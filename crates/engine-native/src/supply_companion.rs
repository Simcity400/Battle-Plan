//! Supply calculation data joined through the country supply system
//! (`ccountry_supply_system::COMPANION`) and the supply-system array. Offsets are object-relative.

/// Companion array element stride.
pub const STRIDE: usize = 800;
/// Supply-node array data pointer.
pub const NODES: usize = 0xb8;
/// Supply-node array capacity, u32.
pub const NODE_CAPACITY: usize = 0xc0;
/// Related-controller byte bitmap pointer, indexed by country slot.
pub const RELATED_CONTROLLERS: usize = 0xd0;
/// Required trucks, signed i64 fixed5. Hourly reset clears it; its nonzero
/// writer remains unestablished.
pub const REQUIRED_TRUCKS_RAW: usize = 0x130;
/// Required trains, signed i32, reset and accumulated during hourly supply update.
pub const REQUIRED_TRAINS: usize = 0x140;
/// Subtype-zero sorted province/node route array.
pub const ROUTES_SUBTYPE_ZERO: usize = 0x2c0;
