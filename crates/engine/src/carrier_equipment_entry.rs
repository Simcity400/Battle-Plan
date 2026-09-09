//! Unnamed equipment entry in the carrier-host oil-capacity walk.
//!
//! Its pointee is consistent with CEquipmentVariant's module container, but
//! the old walk did not authenticate that class; no amount field is inferred.

/// Carrier equipment entry stride; compact_market_trade_oil_airbase_cache_value steps 0x10 bytes.
pub const STRIDE: usize = 0x10;
/// Carrier equipment entry module-bearing object pointer; compact_market_trade_oil_airbase_cache_value reads entry+0.
pub const OBJECT: usize = 0x00;
