//! CUpgradeEntry for the intelligence-agency window; class identified by FUN141a00a50 vtable writes.

/// CUpgradeEntry constructor RVA; old dynamic_collection_trace_live/src/a729_named_overlap_hook.rs:20 gates FUN141a00a50.
pub const CONSTRUCTOR_RVA: usize = 0x01a0_0a50;
/// CUpgradeEntry constructor bytes; old a729_named_overlap_hook.rs:64-66 verifies this sentinel before observation.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// CUpgradeEntry overlap widget pointer +0x1068; old a729_named_overlap_hook.rs:525 reads it after the constructor returns.
pub const OVERLAP: usize = 0x1068;
