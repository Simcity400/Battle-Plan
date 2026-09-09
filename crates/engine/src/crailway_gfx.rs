//! CRailwayGfx presentation state and its route builder in a729.
//! td_rail_change_signal/DOSSIER.md:128 identifies RefreshProvinceRailway;

/// CRailwayGfx native f32 alpha; rail_files.rs:2032 records the +0xcc camera-cutoff recurrence and its draw threshold.
pub const ALPHA: usize = 0xcc;
/// CRailwayGfx route/ribbon builder; rail_files.rs:2011 and td_rail_change_signal/excerpts/func_141c53520.decompile.c:173 agree on the receiver call.
pub const BUILD_ROUTE_RIBBON_RVA: usize = 0x01c5_1b40;
