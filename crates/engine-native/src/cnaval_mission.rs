//! CNavalMission embedded in CTaskForce. Counter predicates consume these operands; offsets use the
//! mission base.

/// CNavalMission vtable.
pub const VTABLE_RVA: usize = 0x029f_b4b8;
/// Owning CTaskForce primary pointer.
pub const TASK_FORCE: usize = 0x08;
/// Mission kind i32 (HOLD..NAVAL_INVASION_SUPPORT).
pub const KIND: usize = 0x14;
/// In-regions byte; active mission predicates require it.
pub const IS_IN_REGIONS: usize = 0x30;
/// Unsaved running-away byte, set by repair/refit movement and cleared
/// by cancellation. While set, engagement value is zero and the task force
/// is considered nonengaging.
#[cfg(test)]
pub const RUNNING_AWAY: usize = 0x31;
/// Engagement rule i32, constructed as 2.
pub const ENGAGEMENT_RULE: usize = 0x34;
/// Bombardment-region pointer, required for mission kind 9; save uses its id.
pub const BOMBARDMENT_REGION: usize = 0x60;
/// Accessible CStrategicRegion pointer array.
pub const ACCESSIBLE_REGIONS: usize = 0xc8;
/// Mission constructor for the task-force embedded object.
pub const CONSTRUCTOR_RVA: usize = 0x0102_df20;
