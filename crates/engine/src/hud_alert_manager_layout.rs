//! Active-alert manager reached from the UI root; exact C++ class unproven.
//! This is presentation state, not the country's complete alert causes.

/// Alert manager's per-type CPdxArray base.
pub const TYPE_VECTORS: usize = 0x16950;
/// Alert manager per-type CPdxArray stride.
pub const TYPE_VECTOR_STRIDE: usize = 0x18;
/// Alert manager's engine alert type count.
pub const ALERT_TYPE_COUNT: usize = 0x52;
