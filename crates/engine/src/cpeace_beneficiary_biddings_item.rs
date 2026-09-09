//! CPeaceBeneficiaryBiddingsItem, identified by constructor vtable writes in FUN141e34b30.

/// CPeaceBeneficiaryBiddingsItem constructor RVA; old dynamic_collection_trace_live/src/a729_named_overlap_hook.rs:19 gates FUN141e34b30.
pub const CONSTRUCTOR_RVA: usize = 0x01e3_4b30;
/// CPeaceBeneficiaryBiddingsItem constructor bytes; old a729_named_overlap_hook.rs:61-63 verifies this sentinel before observation.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// CPeaceBeneficiaryBiddingsItem overlap widget pointer +0x5d0; old a729_named_overlap_hook.rs:511 reads it after the constructor returns.
pub const OVERLAP: usize = 0x5d0;
