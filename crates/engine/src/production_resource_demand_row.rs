//! Native resource-demand entry in CEquipmentProductionLine's demand vector.
//!
//! The old oil-production walk did not establish a C++ class name for this row.
//! Values remain the two signed fixed-point integers before their display sum.

/// Resource-demand entry stride; old compact_market_trade_oil_production_need at attach_log.rs:114197 subscripts by 0x20.
pub const STRIDE: usize = 0x20;
/// Resource-demand CStrategicResource pointer; old oil-production walk follows entry+0x08 and validates its index.
pub const RESOURCE: usize = 0x08;
/// Resource-demand first native i64 amount; old oil-production walk reads entry+0x10 before adding the second amount.
pub const FIRST_AMOUNT: usize = 0x10;
/// Resource-demand second native i64 amount; old oil-production walk reads entry+0x18 before fixed-point rounding.
pub const SECOND_AMOUNT: usize = 0x18;
