//! CNavalMission embedded in CTaskForce. Counter predicates consume these operands; offsets use the
//! mission base.

/// CNavalMission vtable.
pub const VTABLE_RVA: usize = 0x0296_8408;
/// Owning CTaskForce primary pointer.
pub const TASK_FORCE: usize = 0x08;
/// Mission kind i32 (HOLD..NAVAL_INVASION_SUPPORT).
pub const KIND: usize = 0x14;
/// In-regions byte; active mission predicates require it.
pub const IS_IN_REGIONS: usize = 0x30;
/// Unsaved running-away byte, set by repair/refit movement and cleared
/// by cancellation. While set, engagement value is zero and the task force
/// is considered nonengaging.
pub const RUNNING_AWAY: usize = 0x31;
/// Loaded engagement-rule array of signed i64 fixed5 values. Header capacity and count follow the
/// data pointer; the stored engagement-rule ordinal indexes it.
pub const AGGRESSION_SETTINGS_VALUES_RVA: usize = 0x0332_08f0;
/// Engagement-value getter returns zero while running away, otherwise the loaded rule value.
pub const ENGAGEMENT_VALUE_RVA: usize = 0x00f9_f210;
/// Engagement rule i32, constructed as 2.
pub const ENGAGEMENT_RULE: usize = 0x34;
/// Bombardment-region pointer, required for mission kind 9; save uses its id.
pub const BOMBARDMENT_REGION: usize = 0x60;
/// Accessible CStrategicRegion pointer array.
pub const ACCESSIBLE_REGIONS: usize = 0xc8;
/// Signed accessible-region count; the counter tests it for zero.
pub const ACCESSIBLE_REGION_COUNT: usize = 0xd4;
/// Mission constructor for the task-force embedded object.
pub const CONSTRUCTOR_RVA: usize = 0x00f9_4710;
/// Mission save writer.
pub const SERIALIZE_RVA: usize = 0x00fa_9900;
/// Mission save-token reader.
pub const LOAD_RVA: usize = 0x00fa_49b0;
/// Counter engagement predicate uses running-away state, loaded rule value and mission kinds 2/3.
pub const ENGAGING_PREDICATE_RVA: usize = 0x00fa_2780;
/// Counter region-requiring predicate admits mission kinds 1..6 and 9.
pub const REGION_KIND_PREDICATE_RVA: usize = 0x00fa_2c20;
/// Counter active predicate uses active byte and kind; kind 9 also requires `BOMBARDMENT_REGION`.
pub const ACTIVE_PREDICATE_RVA: usize = 0x00fa_2d70;
