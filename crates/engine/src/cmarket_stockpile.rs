//! CMarketStockpile layout.

/// CMarketStockpile owner raw country link, u32.
pub const OWNER: usize = 0x20;
/// CMarketStockpile embedded CEquipmentVariantPool.
pub const EQUIPMENT_POOL: usize = 0x38;
/// CMarketStockpile constructor.
pub const CONSTRUCTOR_RVA: usize = 0x19c3190;
