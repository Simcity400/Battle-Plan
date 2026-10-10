//! CFacilitiesTabView presentation layout.
//!
//! The setup callback is excluded from hourly state sampling.

/// Facilities setup entry.
pub const SETUP_DERIVED_RVA: usize = 0x01c9_d840;
/// Exact facilities setup entry bytes.
pub const SETUP_DERIVED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac, 0x24,
];
/// CFacilitiesTabView specialization list pointer.
pub const SPECIALIZATION_LIST: usize = 0x0b58;
/// Facilities source-token snapshot site; RBX holds the owner.
pub const SOURCE_SNAPSHOT_RVA: usize = 0x01c9_dd9b;
/// Facilities setup source-vector pointer at RSP+0x30 of the snapshot site.
pub const SOURCE_SNAPSHOT_STACK_DATA: usize = 0x30;
/// Facilities setup source-vector u32 count at RSP+0x3C of the snapshot site.
pub const SOURCE_SNAPSHOT_STACK_COUNT: usize = 0x3c;
/// CFacilitiesTabView per-row append completion site; facility hook contract records the return from list virtual +0x20.
pub const ROW_APPEND_COMPLETED_RVA: usize = 0x01c9_ddf6;
/// CFacilitiesTabView retained-list commit site; facility hook contract records [RBX+0xB58] before the final tail dispatch.
pub const LIST_COMMITTED_RVA: usize = 0x01c9_de44;
