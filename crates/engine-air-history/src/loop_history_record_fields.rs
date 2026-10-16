//! Record-fields storage reached through the history window.

/// Record-fields pointer to channel values.
pub const VALUES: usize = 0;
/// Record-fields i64 channel stride.
pub const VALUE_STRIDE: usize = 8;
/// Record-fields CPdxArray u32 capacity for i64 fields.
pub const CAPACITY: usize = 8;
/// Record-fields CPdxArray u32 count, copied from the owning history.
pub const COUNT: usize = 0xc;
/// Record-fields owning history pointer.
pub const HISTORY: usize = 0x18;
