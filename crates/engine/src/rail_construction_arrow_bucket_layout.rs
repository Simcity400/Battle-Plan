//! Endpoint hash bucket used by the FUN14159b650 arrow manager; C++ type is unconfirmed.

/// Endpoint bucket stride used by rail_construction_arrow.rs:12,182,204.
pub const STRIDE: usize = 0x30;
/// Endpoint bucket occupancy i8, read by rail_construction_arrow.rs:184.
pub const OCCUPANCY: usize = 4;
/// Endpoint bucket empty marker checked by rail_construction_arrow.rs:184.
pub const EMPTY: i8 = -1;
/// Endpoint bucket canonical lower province id u32, read by rail_construction_arrow.rs:189.
pub const LOWER_PROVINCE: usize = 8;
/// Endpoint bucket canonical higher province id u32, read by rail_construction_arrow.rs:190.
pub const HIGHER_PROVINCE: usize = 0x0c;
/// Endpoint bucket selected entity pointer, read by rail_construction_arrow.rs:192.
pub const ENTITY: usize = 0x10;
/// Old lookup reads chain length u8 at current+0x34 (next slot+4), rail_construction_arrow.rs:200; retain this unusual access uncorrected.
pub const NEXT_SLOT_CHAIN_LENGTH: usize = 0x34;
