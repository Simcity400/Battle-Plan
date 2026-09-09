//! CStrategicRegion, pinned a729; all offsets use its primary object.

/// CStrategicRegion primary vtable; executable RTTI confirms old AIR_WING_REGION_VTABLE_RVA.
pub const VTABLE_RVA: usize = 0x0295_69a8;
/// CStrategicRegion secondary vtable; executable RTTI and read_air_wing_mission_region confirm +8 displacement.
pub const SECONDARY_VTABLE_RVA: usize = 0x0295_69f8;
/// CStrategicRegion secondary subobject; collector read_air_wing_mission_region authenticates both vtables.
pub const SECONDARY: usize = 0x08;
/// CStrategicRegion u32 id; collector read_air_wing_mission_region reads primary+58.
pub const ID: usize = 0x58;
/// CStrategicRegion CProvince* vector; compact_market_trade_oil_context_average follows primary+18 for each hourly context sample.
pub const PROVINCES: usize = 0x18;
/// CStrategicRegion i32 province count; compact_market_trade_oil_context_average reads primary+24 before weather aggregation.
pub const PROVINCES_COUNT: usize = 0x24;
/// CStrategicRegion naval-dominance container pointer; naval_missions_status_live::read_naval_supremacy_rows authenticates the pointee.
pub const NAVAL_DOMINANCE: usize = 0xe8;
