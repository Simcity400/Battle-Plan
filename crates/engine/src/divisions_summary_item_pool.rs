//! Divisions-summary item pool used by FUN141ce6b40; its concrete template name is not retained.

/// Divisions-summary pool population function; old dynamic_collection_trace_live/src/a729_named_overlap_hook.rs:22 gates FUN141ce6b40.
pub const POPULATE_RVA: usize = 0x01ce_6b40;
/// Divisions-summary pool population bytes; old a729_named_overlap_hook.rs:70-72 verifies this exact-build sentinel.
pub const POPULATE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57,
];
/// Divisions-summary pool item pointer array +0x18; old a729_named_overlap_hook.rs:612 reads each pointer after population.
pub const ITEMS: usize = 0x18;
/// Divisions-summary pool count u32 +0x24; old a729_named_overlap_hook.rs:607 bounds the item pointer walk.
pub const COUNT: usize = 0x24;
