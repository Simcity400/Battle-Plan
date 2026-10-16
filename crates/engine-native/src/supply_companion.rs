//! Supply calculation data joined through the country supply system
//! (`ccountry_supply_system::COMPANION`) and the supply-system array. Offsets are object-relative.

/// Companion array element stride.
pub const STRIDE: usize = 800;
/// Province supply hash bucket pointer.
pub const SUPPLY_BUCKETS: usize = 0x08;
/// Hash occupied count, u32.
pub const SUPPLY_COUNT: usize = 0x10;
/// Hash mask, u32.
pub const SUPPLY_MASK: usize = 0x14;
/// Maximum probe distance, u8.
pub const SUPPLY_MAX_PROBE: usize = 0x18;
/// Supply-node array data pointer.
pub const NODES: usize = 0xb8;
/// Supply-node array capacity, u32.
pub const NODE_CAPACITY: usize = 0xc0;
/// Supply-node array count, u32.
pub const NODE_COUNT: usize = 0xc4;
/// Related-controller byte bitmap pointer, indexed by country slot.
pub const RELATED_CONTROLLERS: usize = 0xd0;
/// Related-controller count bounding canonical controller indices.
pub const RELATED_CONTROLLER_COUNT: usize = 0xdc;
/// Required trucks, signed i64 fixed5. Hourly reset clears it; its nonzero
/// writer remains unestablished.
pub const REQUIRED_TRUCKS_RAW: usize = 0x130;
/// Required trains, signed i32, reset and accumulated during hourly supply update.
pub const REQUIRED_TRAINS: usize = 0x140;
/// Subtype-zero sorted province/node route array.
pub const ROUTES_SUBTYPE_ZERO: usize = 0x2c0;
/// Subtype-one sorted province/node route array.
pub const ROUTES_SUBTYPE_ONE: usize = 0x2d8;
/// Fallback sorted province/node route array.
pub const ROUTES_FALLBACK: usize = 0x2f0;
/// Province supply hash lookup by exact province key.
pub const SUPPLY_LOOKUP_RVA: usize = 0x0121_3f80;
