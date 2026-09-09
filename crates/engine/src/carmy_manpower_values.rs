//! CArmyManpowerValues embedded in deployment objects.
//!
//! Rows use manpower_value_entry's eight-byte layout. The old deployment
//! walk summed positive i32 values into i64 and omitted the aggregate for
//! an empty array; those are consumer choices, not engine representations.

/// CArmyManpowerValues primary vtable; offsets.rs:451 and pinned RTTI confirm the actual container class.
pub const VTABLE_RVA: usize = 0x2943900;
/// CArmyManpowerValues row pointer; offsets.rs:448 and read_deployment_manpower_container read +8.
pub const DATA: usize = 0x08;
/// CArmyManpowerValues array capacity, u32; offsets.rs:449 and read_deployment_manpower_container read +0x10.
pub const CAPACITY: usize = 0x10;
/// CArmyManpowerValues array count, u32; offsets.rs:450 and read_deployment_manpower_container read +0x14.
pub const COUNT: usize = 0x14;
