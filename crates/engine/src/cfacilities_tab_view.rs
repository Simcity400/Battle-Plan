//! NProject::NUi::CFacilitiesTabView presentation layout for a729.
//!
//! Preserved from the excluded dynamic-collection presentation lane; this
//! controller's setup callback is not an hourly state sampling boundary.

/// CFacilitiesTabView::SetupDerived; a729_facility_smooth_list_hook.rs:18/117 pins and checks this entry.
pub const SETUP_DERIVED_RVA: usize = 0x01c9_d840;
/// CFacilitiesTabView::SetupDerived entry bytes; a729_facility_smooth_list_hook.rs:48-50 checks all 16 bytes.
pub const SETUP_DERIVED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac, 0x24,
];
/// CFacilitiesTabView specialization list pointer; old callback reads +0xB58 after SetupDerived, a729_facility_smooth_list_hook.rs:350-355.
pub const SPECIALIZATION_LIST: usize = 0x0b58;
pub const SOURCE_SNAPSHOT_RVA: usize = 0x01c9_dd9b;
/// CFacilitiesTabView source vector pointer on the setup stack; facility hook contract records [RSP+0x30] at SOURCE_SNAPSHOT_RVA.
pub const SOURCE_SNAPSHOT_STACK_DATA: usize = 0x30;
/// CFacilitiesTabView source vector u32 count on the setup stack; facility hook contract records [RSP+0x3C] at SOURCE_SNAPSHOT_RVA.
pub const SOURCE_SNAPSHOT_STACK_COUNT: usize = 0x3c;
/// CFacilitiesTabView per-row append completion site; facility hook contract records the return from list virtual +0x20.
pub const ROW_APPEND_COMPLETED_RVA: usize = 0x01c9_ddf6;
/// CFacilitiesTabView retained-list commit site; facility hook contract records [RBX+0xB58] before the final tail dispatch.
pub const LIST_COMMITTED_RVA: usize = 0x01c9_de44;
