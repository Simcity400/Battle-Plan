//! CEquipmentDesignerView presentation layout, excluded from hourly recording.

/// Equipment-designer refresh entry.
pub const REFRESH_RVA: usize = 0x0177_97a0;
/// Exact equipment-designer refresh entry bytes.
pub const REFRESH_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0x89, 0xa0, 0xf3, 0x00, 0x00,
];
/// Ordered equipment-designer grid producer.
pub const GRID_PRODUCER_RVA: usize = 0x0177_d780;
/// Full equipment-designer refresh; receiver is view+0x28.
pub const FULL_REFRESH_RVA: usize = 0x0177_a720;
/// Full-refresh receiver's offset within CEquipmentDesignerView.
pub const FULL_REFRESH_SUBOBJECT: usize = 0x28;
/// Equipment-designer selection entry.
pub const SELECTION_RVA: usize = 0x0177_15a0;
/// Exact equipment-designer selection entry bytes.
pub const SELECTION_SENTINEL: [u8; 16] = [
    0x48, 0x85, 0xd2, 0x0f, 0x84, 0x14, 0x03, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56,
];
/// View +0x3788 handler pointer.
pub const HANDLER: usize = 0x3788;
/// Designer signed i32 mode: 0 equipment/ship, 1 tank, 2 plane.
pub const MODE: usize = 0x3790;
/// Designer intelligence-container pointer; visible flag gates presentation.
pub const INTEL_CONTAINER: usize = 0xf3a0;
/// View +0xF3A8 enclosing scroll container.
pub const SCROLL_CONTAINER: usize = 0xf3a8;
/// View +0xF3B0 variant grid.
pub const VARIANT_GRID: usize = 0xf3b0;
/// Designer row-pointer array with eight-byte elements.
pub const ROW_POOL_DATA: usize = 0xf3c0;
/// Designer allocated row count, u32; used count must not exceed it.
pub const ROW_POOL_COUNT: usize = 0xf3cc;
/// Designer used row count, u32.
pub const ROW_POOL_USED: usize = 0xf3d8;
/// View +0xF3E8 country-filter grid.
pub const COUNTRY_FILTER_GRID: usize = 0xf3e8;
/// Designer excluded-country id array with u32 elements.
pub const EXCLUDED_COUNTRIES_DATA: usize = 0xf418;
/// Designer excluded-country count, u32.
pub const EXCLUDED_COUNTRIES_COUNT: usize = 0xf424;
/// Designer excluded-country element stride for four-byte u32 ids.
pub const EXCLUDED_COUNTRY_STRIDE: usize = 4;
/// View +0x10D98 own-country filter byte.
pub const OUR_FILTER: usize = 0x10d98;
/// View +0x10D99 foreign-country filter byte.
pub const FOREIGN_FILTER: usize = 0x10d99;
/// View +0x10D9A obsolete-design filter byte.
pub const OBSOLETE_FILTER: usize = 0x10d9a;
