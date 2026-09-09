//! CRailwayProductionLine; inherited fields live in cgeneral_production_line and cproduction_line.

/// CRailwayProductionLine primary vtable; construction lane lib.rs:105 and pinned executable RTTI agree.
pub const VTABLE_RVA: usize = 0x02a1_5130;
/// CRailwayProductionLine observed CReferenceObject type; construction lane LINE_TYPE_RAILWAY_CONSTRUCTION.
pub const REFERENCE_TYPE_ID: u32 = 4_713;
/// CRailwayProductionLine initial railway level u32; read_railway_construction_queue_from_manager.
pub const BASE_LEVEL: usize = 0x70;
/// CRailwayProductionLine requested railway level u32; construction lane lib.rs RAILWAY_LINE_TARGET_LEVEL.
pub const TARGET_LEVEL: usize = 0x74;
/// CRailwayProductionLine completion flag u8; read_railway_construction_queue_from_manager.
pub const COMPLETION_FLAG: usize = 0x78;
/// CRailwayProductionLine province-id route array; construction lane lib.rs RAILWAY_LINE_ROUTE_PTR.
pub const ROUTE: usize = 0x80;
/// CRailwayProductionLine route array u32 capacity; construction lane lib.rs RAILWAY_LINE_ROUTE_CAP.
pub const ROUTE_CAPACITY: usize = 0x88;
/// CRailwayProductionLine route array u32 count; construction lane lib.rs RAILWAY_LINE_ROUTE_SIZE.
pub const ROUTE_COUNT: usize = 0x8c;
/// CRailwayProductionLine route entries are u32 province ids; old queue reader multiplies index by 4.
pub const ROUTE_PROVINCE_STRIDE: usize = 0x04;
/// CRailwayProductionLine current segment i32, with -1 allowed; old railway queue reader.
pub const CURRENT_SEGMENT: usize = 0x98;
/// CRailwayProductionLine remaining hours i64 raw; old queue reader preserves width without conversion.
pub const REMAINING_HOURS: usize = 0xa0;
