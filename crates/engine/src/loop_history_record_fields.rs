//! Record-fields storage reached through the CLoopHistory window at +0x30.
//! The source walk does not identify an engine class name for this storage.

/// Record-fields first qword points to channel values; old air_loss_stats_live dereferences recflds directly.
pub const VALUES: usize = 0;
/// Record-fields channel stride eight bytes; old air_loss_stats_live reads each channel as native i64.
pub const VALUE_STRIDE: usize = 8;
