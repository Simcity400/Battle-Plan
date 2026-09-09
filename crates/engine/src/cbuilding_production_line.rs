//! CBuildingProductionLine; exact-build A14 construction report identifies its CBuilding pointer.

/// CBuildingProductionLine CReferenceObject type 59; compact build_profile.rs LINE_TYPE_CONSTRUCTION.
pub const REFERENCE_TYPE_ID: u32 = 59;
/// CBuildingProductionLine CBuilding pointer; A14_RE_EVIDENCE.json and old construction queue walker agree.
pub const BUILDING: usize = 0x70;
/// CBuildingProductionLine repair levels remaining i32; A14 report: level minus healthy_level, zero for new construction.
pub const REPAIR_LEVELS_REMAINING: usize = 0x78;
