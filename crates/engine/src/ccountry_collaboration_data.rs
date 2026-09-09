//! CCountryCollaborationData, confirmed by the pinned executable's RTTI.

/// CCountryCollaborationData vtable; old D52 collector_module.rs:45 and pinned RTTI agree.
pub const VTABLE_RVA: usize = 0x029a_5d08;
/// CCountryCollaborationData target country link, u32; old read_country_collaboration_relations reads +8.
pub const TARGET_COUNTRY: usize = 0x08;
/// CCountryCollaborationData progress, i64/100000; old collector_module.rs:676 reads the full native qword without narrowing.
pub const PROGRESS: usize = 0x10;
/// CCountryCollaborationData progress denominator; old D52 fixed-point contract and read_country_collaboration_relations use 100000.
pub const FIXED_POINT_SCALE: i64 = 100_000;
