//! CProductionStatus reached through CCountry+0xf68 in the old production and trade walks.
//! Constants preserve those reads; whole-hour consistency has not yet been assessed.

/// CProductionStatus production-line pointer array; collector offsets.rs LINE_BEGIN and read_production_lines.
pub const LINES: usize = 0x58;
/// CProductionStatus production-line array u32 capacity; collector offsets.rs LINE_CAP.
pub const LINES_CAPACITY: usize = 0x60;
/// CProductionStatus production-line array u32 count; collector offsets.rs LINE_SIZE.
pub const LINES_COUNT: usize = 0x64;
/// CProductionStatus production-line array stores 8-byte pointers; read_production_line_vector_snapshot.
pub const LINE_POINTER_STRIDE: usize = 0x08;
/// CProductionStatus construction-line pointer array; compact build_profile.rs CONSTRUCTION_BEGIN.
pub const CONSTRUCTION_LINES: usize = 0x70;
/// CProductionStatus construction-line array u32 capacity; compact build_profile.rs CONSTRUCTION_CAP.
pub const CONSTRUCTION_LINES_CAPACITY: usize = 0x78;
/// CProductionStatus construction-line array u32 count; compact build_profile.rs CONSTRUCTION_SIZE.
pub const CONSTRUCTION_LINES_COUNT: usize = 0x7c;
/// CProductionStatus construction array stores 8-byte pointers; construction lane queue walk.
pub const CONSTRUCTION_POINTER_STRIDE: usize = 0x08;
/// CProductionStatus licensed CEquipmentVariant pointer array; collector read_production_licenses.
pub const LICENSES: usize = 0x198;
/// CProductionStatus license array u32 capacity; collector offsets.rs LICENSE_CAP.
pub const LICENSES_CAPACITY: usize = 0x1a0;
/// CProductionStatus license array u32 count; collector offsets.rs LICENSE_SIZE.
pub const LICENSES_COUNT: usize = 0x1a4;
/// CProductionStatus license array stores 8-byte pointers; collector offsets.rs LICENSE_STRIDE.
pub const LICENSE_POINTER_STRIDE: usize = 0x08;
/// CProductionStatus stockpile entry array; collector read_production_stockpile_raw.
pub const STOCKPILE: usize = 0x220;
/// CProductionStatus stockpile array u32 capacity; collector offsets.rs STOCK_CAP.
pub const STOCKPILE_CAPACITY: usize = 0x228;
/// CProductionStatus stockpile array u32 count; collector offsets.rs STOCK_SIZE.
pub const STOCKPILE_COUNT: usize = 0x22c;
/// CProductionStatus owning CCountry pointer; trade factory-header reader validates this backlink.
pub const COUNTRY: usize = 0x290;
/// CProductionStatus factory total i64 fixed /100000; compact_market_trade_factory_header_rows.
pub const FACTORY_TOTAL: usize = 0x378;
/// CProductionStatus unavailable factory component i64 fixed /100000; trade factory-header reader.
pub const FACTORY_AVAILABLE_SUBTRACTION: usize = 0x390;
/// CProductionStatus additional unavailable factory count i32; trade factory-header reader.
pub const FACTORY_AVAILABLE_SUBTRACTION_COUNT: usize = 0x3b0;
/// CProductionStatus total factory subtraction i32, component 0; trade factory-header reader.
pub const FACTORY_TOTAL_SUBTRACTION_0: usize = 0x3b4;
/// CProductionStatus total factory subtraction i32, component 1; trade factory-header reader.
pub const FACTORY_TOTAL_SUBTRACTION_1: usize = 0x3b8;
/// CProductionStatus total factory subtraction i32, component 2; trade factory-header reader.
pub const FACTORY_TOTAL_SUBTRACTION_2: usize = 0x3bc;
/// CProductionStatus total factory subtraction i32, component 3; trade factory-header reader.
pub const FACTORY_TOTAL_SUBTRACTION_3: usize = 0x3c0;
/// CProductionStatus used factory count i32; compact_market_trade_factory_header_rows.
pub const FACTORIES_USED: usize = 0x440;
