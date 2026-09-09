//! Arrow manager passed to FUN14159b650; exact class name is unconfirmed.
//! This visual manager must not be confused with the simulation CRailwayManager.

/// Construction arrow update FUN14159b650, retained from a729_hook/rail_construction_arrow.rs:8.
pub const UPDATE_RVA: usize = 0x0159_b650;
/// FUN14159b650 entry bytes checked by the old hook, rail_construction_arrow.rs:9-11.
pub const UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Arrow manager edge-type-2 entity definition pointer, read by rail_construction_arrow.rs:261.
pub const TYPE_2_PROTOTYPE: usize = 0x20;
/// Arrow manager edge-type-3 entity definition pointer, read by rail_construction_arrow.rs:262.
pub const TYPE_3_PROTOTYPE: usize = 0x28;
/// Arrow manager endpoint bucket array pointer, read by rail_construction_arrow.rs:175.
pub const ENDPOINT_BUCKETS: usize = 0x90;
/// Arrow manager endpoint hash mask u32, read by rail_construction_arrow.rs:176.
pub const ENDPOINT_BUCKET_MASK: usize = 0x9c;
/// Endpoint pair FNV-1a initial value used by the engine lookup mirrored in rail_construction_arrow.rs:165.
pub const ENDPOINT_HASH_OFFSET_BASIS: u32 = 0x811c_9dc5;
/// Endpoint pair FNV-1a multiplier used by the engine lookup mirrored in rail_construction_arrow.rs:166.
pub const ENDPOINT_HASH_PRIME: u32 = 0x0100_0193;
