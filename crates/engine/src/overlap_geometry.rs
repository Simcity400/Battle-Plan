//! Geometry interface used by the old named-overlap observer; exact class unproven.
//!
//! Its +0x78 position slot must not be conflated with the CGuiObject base table,
//! whose +0x78 entry is a show operation. The old receiver's derived layout needs
//! revalidation before these calls can be used.

/// Old overlap geometry position byte slot, output `[i32;2]`; a729_named_overlap_hook.rs:700,720 invokes +0x78 on its child/owner receiver.
pub const POSITION_VSLOT_CANDIDATE: usize = 0x78;
/// Old overlap geometry extent byte slot, output `[i32;2]`; a729_named_overlap_hook.rs:701 invokes +0x28 on the same unproven interface.
pub const EXTENT_VSLOT_CANDIDATE: usize = 0x28;
