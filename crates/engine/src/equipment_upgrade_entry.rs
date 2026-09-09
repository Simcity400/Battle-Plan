//! Equipment upgrade entry; the concrete C++ type was not named by the reviewed reader.
//! The old wire retained only the low byte of the qword at +8; preserve the full native storage until rederived.

/// Equipment-upgrade entry stride; read_compact_air_variant_upgrade_entries copies 0x10-byte records.
pub const STRIDE: usize = 0x10;
/// Equipment-upgrade level storage candidate; legacy reader loads u64 at +8 then masks FF, so full field semantics are unproven.
pub const LEVEL_STORAGE: usize = 0x08;
