//! MIO detail controller, excluded from hourly recording.
//! Its owner field points to an organisation list item; that item's reference
//! resolves the organisation.

/// Detail-window primary vtable.
pub const VTABLE_RVA: usize = 0x02a7_ce30;
/// Detail-window secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02a7_ce58;
/// Detail-window secondary interface.
pub const SECONDARY_INTERFACE: usize = 0x28;
/// Owning organisation-list-item pointer.
pub const ORGANISATION_LIST_ITEM: usize = 0x30;
/// Window pointer used for visibility gating.
pub const DETAIL_WINDOW: usize = 0x38;
/// Signed i32 active tab.
pub const TAB: usize = 0x40;
/// History-tab discriminator.
pub const HISTORY_TAB: i32 = 1;
/// Separator pointer vector.
pub const SEPARATOR_VECTOR: usize = 0x2078;
/// Fixed-bonus pointer vector.
pub const FIXED_VECTOR: usize = 0x2090;
/// Equipment-row pointer vector.
pub const EQUIPMENT_VECTOR: usize = 0x20a8;
/// Optional bonus-row pointer.
pub const OPTIONAL_BONUS_ROW: usize = 0x20c0;
/// Variant-row pool with a signed i32 count.
pub const VARIANT_POOL: usize = 0x20c8;
/// Production-row pool with a signed i32 count.
pub const PRODUCTION_POOL: usize = 0x20e0;
/// Variant standard-list pointer.
pub const VARIANT_LIST: usize = 0x3850;
/// Production standard-list pointer.
pub const PRODUCTION_LIST: usize = 0x3858;
/// Bonus standard-list pointer.
pub const BONUS_LIST: usize = 0x3860;
/// History-refresh entry.
pub const HISTORY_REFRESH_RVA: usize = 0x01f0_ac90;
/// History-refresh entry sentinel.
pub const HISTORY_REFRESH_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Bonus rebuild and section-predicate entry.
pub const BONUS_REBUILD_RVA: usize = 0x01f0_5960;
/// Bonus rebuild entry sentinel.
pub const BONUS_REBUILD_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x54, 0x41, 0x56,
];
/// Variant-history predicate.
pub const VARIANT_PREDICATE_RVA: usize = 0x00da_1420;
/// Production-history predicate.
pub const PRODUCTION_PREDICATE_RVA: usize = 0x00da_1680;
/// Fixed-bonus predicate.
pub const FIXED_PREDICATE_RVA: usize = 0x01f0_6630;
/// Equipment predicate.
pub const EQUIPMENT_PREDICATE_RVA: usize = 0x01f0_5cd0;
/// First shared history-producer entry.
pub const HISTORY_PRODUCER_RVA: usize = 0x01f0_6360;
/// Second shared history-producer entry.
pub const HISTORY_PRODUCER_5390_RVA: usize = 0x01fa_5390;
/// Third shared history-producer entry.
pub const HISTORY_PRODUCER_6630_RVA: usize = 0x01fa_6630;
/// Bonus-producer entry.
pub const BONUS_PRODUCER_RVA: usize = 0x01fa_4b90;
/// Fixed-section presentation branch.
pub const FIXED_SECTION_SITE_RVA: usize = 0x01f0_5a3c;
/// Equipment-section presentation branch.
pub const EQUIPMENT_SECTION_SITE_RVA: usize = 0x01f0_5baa;
/// Optional-bonus presentation branch.
pub const OPTIONAL_BONUS_SITE_RVA: usize = 0x01f0_6032;
/// Bonus UI u8 global for modifier 0x26b.
pub const FIXED_BONUS_GLOBAL_RVA: usize = 0x0331_8b84;
/// Bonus UI usize feature flag for modifier 0x268.
pub const FIXED_FLAG_268_RVA: usize = 0x0331_b570;
/// Bonus UI u32 feature flag for modifier 0x269.
pub const FIXED_FLAG_269_RVA: usize = 0x0331_b9b8;
/// Bonus UI usize feature flag for modifier 0x26a.
pub const FIXED_FLAG_26A_RVA: usize = 0x0331_b600;
