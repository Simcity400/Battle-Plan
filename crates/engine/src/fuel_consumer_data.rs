//! Anonymous fuel consumer entry in CCountryFuelStatus's consumer array (+0x48).

/// Consumer entry stride.
pub const STRIDE: usize = 0x18;
/// Consumer priority i32.
pub const PRIORITY: usize = 0x00;
/// Requested fuel i64 /100000.
pub const REQUESTED: usize = 0x08;
/// Received fuel i64 /100000.
pub const RECEIVED: usize = 0x10;
