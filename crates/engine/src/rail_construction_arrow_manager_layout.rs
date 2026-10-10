//! Construction-arrow visual manager.
//! Its layout is distinct from the simulation railway manager.

/// Construction-arrow update routine.
pub const UPDATE_RVA: usize = 0x0159_b650;
/// Exact update-entry instruction sentinel.
pub const UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20, 0x55,
];
/// Edge-type-2 entity definition pointer.
pub const TYPE_2_PROTOTYPE: usize = 0x20;
/// Edge-type-3 entity definition pointer.
pub const TYPE_3_PROTOTYPE: usize = 0x28;
/// Endpoint bucket array pointer.
pub const ENDPOINT_BUCKETS: usize = 0x90;
/// Endpoint hash mask, u32.
pub const ENDPOINT_BUCKET_MASK: usize = 0x9c;
/// Endpoint-pair FNV-1a initial value.
pub const ENDPOINT_HASH_OFFSET_BASIS: u32 = 0x811c_9dc5;
/// Endpoint-pair FNV-1a multiplier.
pub const ENDPOINT_HASH_PRIME: u32 = 0x0100_0193;
