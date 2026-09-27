//! NIndustrialOrganisation::COrganisationDetailWindow, the MIO detail controller.
//! Knowledge only: this presentation path is excluded from hourly recording.
//! Sources are preserved in archive/pre-rewrite-2026-09-09 under
//! extractor/native/lanes/dynamic_collection_trace_live/src/.
//! Source: a729_mio_history_hook.rs (H), a729_mio_bonus_hook.rs (B),
//! and a729_air_mio_controller.rs (A).
//! Exact-build Ghidra job 20260722T073034Z-a28-mio-controller-org-field-confirm-02849eb3
//! identifies the class in FUN141f04ea0 and its owning COrganisationListItem in FUN141f0bb10.
//! The old history hook's direct-organisation interpretation of +0x30 is wrong:
//! FUN141f0ac90 instructions at 141f0aceb-141f0ad04 resolve the item's +0x84 reference.

/// COrganisationDetailWindow primary vtable; retained FUN141f04ea0 at 141f04f90 installs this RTTI identity.
pub const VTABLE_RVA: usize = 0x02a7_ce30;
/// COrganisationDetailWindow secondary vtable; retained FUN141f04ea0 at 141f04f9a installs it at +0x28.
pub const SECONDARY_VTABLE_RVA: usize = 0x02a7_ce58;
/// COrganisationDetailWindow secondary interface at +0x28; retained FUN141f04ea0 at 141f04fa1 initializes it.
pub const SECONDARY_INTERFACE: usize = 0x28;
/// Detail controller +0x30 COrganisationListItem pointer; FUN141f0ac90 resolves item +0x84, correcting H:344.
pub const ORGANISATION_LIST_ITEM: usize = 0x30;
/// Detail controller +0x38 window pointer; H:335 reads it before visibility gating.
pub const DETAIL_WINDOW: usize = 0x38;
/// Detail controller +0x40 i32 active tab; H:325 accepts value 1 for history.
pub const TAB: usize = 0x40;
/// Detail controller history-tab discriminant; H:325 compares the native i32 to 1.
pub const HISTORY_TAB: i32 = 1;
/// Detail controller +0x2078 separator pointer vector; B:416 walks data and i32 count.
pub const SEPARATOR_VECTOR: usize = 0x2078;
/// Detail controller +0x2090 fixed-bonus pointer vector; B:369 walks data and i32 count.
pub const FIXED_VECTOR: usize = 0x2090;
/// Detail controller +0x20a8 equipment-row pointer vector; B:403 walks data and i32 count.
pub const EQUIPMENT_VECTOR: usize = 0x20a8;
/// Detail controller +0x20c0 optional bonus row pointer; B:386 checks source/row presence.
pub const OPTIONAL_BONUS_ROW: usize = 0x20c0;
/// Detail controller +0x20c8 variant row pool; H:428/690 reads its i32 count at +0x0c.
pub const VARIANT_POOL: usize = 0x20c8;
/// Detail controller +0x20e0 production row pool; H:429/690 reads its i32 count at +0x0c.
pub const PRODUCTION_POOL: usize = 0x20e0;
/// Detail controller +0x3850 variant standard-list pointer; H:435/692 resolves its rows.
pub const VARIANT_LIST: usize = 0x3850;
/// Detail controller +0x3858 production standard-list pointer; H:436/692 resolves its rows.
pub const PRODUCTION_LIST: usize = 0x3858;
/// Detail controller +0x3860 bonus standard-list pointer; B:612 resolves its rows.
pub const BONUS_LIST: usize = 0x3860;
/// FUN141f0ac90 history refresh entry; H:28/156 compares the retained entry bytes before hooking.
pub const HISTORY_REFRESH_RVA: usize = 0x01f0_ac90;
/// FUN141f0ac90 first 16 bytes retained by H:85; no new hook or validation is installed here.
pub const HISTORY_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// FUN141f05960 bonus rebuild/section predicate; B:28/84/173 retains this hook site.
pub const BONUS_REBUILD_RVA: usize = 0x01f0_5960;
/// FUN141f05960 first 16 bytes retained by B:100; no new hook or validation is installed here.
pub const BONUS_REBUILD_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x54, 0x41, 0x56,
];
/// FUN140da1420 variant predicate; H:81 and A:100 identify this history producer.
pub const VARIANT_PREDICATE_RVA: usize = 0x00da_1420;
/// FUN140da1680 production predicate; H:82 and A:101 identify this history producer.
pub const PRODUCTION_PREDICATE_RVA: usize = 0x00da_1680;
/// FUN141f06630 fixed-bonus predicate; B:84 and A:99 list this producer.
pub const FIXED_PREDICATE_RVA: usize = 0x01f0_6630;
/// FUN141f05cd0 equipment predicate; B:85 and A:99 list this producer.
pub const EQUIPMENT_PREDICATE_RVA: usize = 0x01f0_5cd0;
/// FUN141f06360 shared history producer; H:449/451 and A:100/101 retain its RVA, ownership unknown.
pub const HISTORY_PRODUCER_RVA: usize = 0x01f0_6360;
/// FUN141fa5390 shared history producer; H:449/451 and A:100/101 retain its RVA, ownership unknown.
pub const HISTORY_PRODUCER_5390_RVA: usize = 0x01fa_5390;
/// FUN141fa6630 shared history producer; H:449/451 and A:100/101 retain its RVA, ownership unknown.
pub const HISTORY_PRODUCER_6630_RVA: usize = 0x01fa_6630;
/// FUN141fa4b90 bonus producer; B:712 and A:99 retain its RVA, ownership unknown.
pub const BONUS_PRODUCER_RVA: usize = 0x01fa_4b90;
/// FUN141f05960 fixed-section branch; B:93 uses its RVA as a presentation identity key.
pub const FIXED_SECTION_SITE_RVA: usize = 0x01f0_5a3c;
/// FUN141f05960 equipment-section branch; B:94 uses its RVA as a presentation identity key.
pub const EQUIPMENT_SECTION_SITE_RVA: usize = 0x01f0_5baa;
/// FUN141f05960 optional-bonus branch; B:95 uses its RVA as a presentation identity key.
pub const OPTIONAL_BONUS_SITE_RVA: usize = 0x01f0_6032;
/// Bonus UI global for modifier 0x26b, read as u8 by B:737; owning engine struct unknown.
pub const FIXED_BONUS_GLOBAL_RVA: usize = 0x0331_8b84;
/// Bonus UI feature flag for modifier 0x268, read as usize by B:739; owning engine struct unknown.
pub const FIXED_FLAG_268_RVA: usize = 0x0331_b570;
/// Bonus UI feature flag for modifier 0x269, read as u32 by B:748; owning engine struct unknown.
pub const FIXED_FLAG_269_RVA: usize = 0x0331_b9b8;
/// Bonus UI feature flag for modifier 0x26a, read as usize by B:754; owning engine struct unknown.
pub const FIXED_FLAG_26A_RVA: usize = 0x0331_b600;
