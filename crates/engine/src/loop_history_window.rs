//! The window reached through CLoopHistory +0x20; its engine class name is unknown.

/// History-window capacity u32 at +0x20; old air_loss_stats_live derive_country_row_result checks five.
pub const CAPACITY: usize = 0x20;
/// Air-statistics yearly window capacity five; old air_loss_stats_live validates this observed layout.
pub const AIR_STATS_YEARLY_CAPACITY: u32 = 5;
/// History-window record-fields pointer at +0x30; old air_loss_stats_live follows it to the i64 field array.
pub const RECORD_FIELDS: usize = 0x30;
