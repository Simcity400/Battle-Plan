//! Carrier-airbase host handle payload consumed by the old oil-capacity walk.
//!
//! This is expected to be a CShip, but the old resolver neither authenticates
//! its class nor documents a primary/secondary-base adjustment. These offsets
//! remain relative to that resolved payload until its identity is proved.

/// Carrier host equipment-entry array; compact_market_trade_oil_airbase_cache_value reads payload+60.
pub const EQUIPMENT_ENTRIES: usize = 0x60;
/// Carrier host equipment-entry count, i32; compact_market_trade_oil_airbase_cache_value reads payload+6C.
pub const EQUIPMENT_ENTRY_COUNT: usize = 0x6c;
/// Carrier host capacity classifier byte; compact_market_trade_oil_capacity_divisor reads payload+628.
pub const CAPACITY_CLASSIFIER: usize = 0x628;
/// Carrier host capacity-class bit; compact_market_trade_oil_capacity_divisor uses byte bit 80 to select class 3.
pub const CAPACITY_CLASSIFIER_MASK: u8 = 0x80;
/// Carrier host cached module-property bool byte; compact_market_trade_oil_airbase_cache_value reads payload+801.
pub const MODULE_PROPERTY_CACHE: usize = 0x801;
/// Carrier host cache-valid bool byte; compact_market_trade_oil_airbase_cache_value tests payload+802 first.
pub const MODULE_PROPERTY_CACHE_VALID: usize = 0x802;
