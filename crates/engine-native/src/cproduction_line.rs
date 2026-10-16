//! Production-line base shared by equipment and general lines at offset zero.

/// Inherited reference-object type, u32.
pub const REFERENCE_TYPE: usize = 0x08;
/// Inherited reference-object instance ID, u32.
pub const ID: usize = 0x0c;
/// Active factory count, i32, consumed by production and construction reads.
pub const ACTIVE_FACTORIES: usize = 0x18;
/// Production-status owner pointer supplied by the third constructor argument.
pub const PRODUCTION_STATUS: usize = 0x20;
/// Cost, signed i64 fixed5, storing the full calculation.
pub const COST: usize = 0x28;
/// Speed, signed i64 fixed5, stored only after the nonnegative check.
pub const SPEED: usize = 0x30;
/// Produced amount, signed i64 fixed5, accumulating and subtracting completed costs.
pub const PRODUCED: usize = 0x38;
/// Production-line priority, i32.
pub const PRIORITY: usize = 0x44;
/// Requested output amount, i32.
pub const AMOUNT: usize = 0x48;
