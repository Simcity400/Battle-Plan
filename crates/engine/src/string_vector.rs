//! Vector operations for map-application string queues. The header
//! initializer is shared with other element types.

/// Header initializer clearing two qwords and storing the allocator at +0x10.
pub const INITIALIZE_RVA: usize = 0x0011_da30;
/// String-vector append using 0x20-byte entries.
pub const PUSH_STRING_RVA: usize = 0x0011_d230;
/// String-range teardown for zero through count, followed by clearing count;
/// also used by the runtime DLC vector.
pub const DESTROY_RANGE_RVA: usize = 0x0012_07d0;
