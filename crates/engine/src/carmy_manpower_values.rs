//! CArmyManpowerValues embedded in divisions, deployment and garrisons.
//!
//! Rows use manpower_value_entry's eight-byte layout. The old deployment
//! walk summed positive i32 values into i64 and omitted the aggregate for
//! an empty array; those are consumer choices, not engine representations.

/// 140C58490 installs this vtable at CArmyManpower+8; CArmy ctor 140C5DD30 embeds that object at primary +3D0.
pub const VTABLE_RVA: usize = 0x2943900;
/// 140C5B350 losses and writer 140C5B0D0 use data pointer+8, entries stride 8.
pub const DATA: usize = 0x08;
/// CPdxArray capacity word+10, allocator/growth 140C5B0D0; negative headers refused.
pub const CAPACITY: usize = 0x10;
/// 140C5B350 uses signed i32 iteration over count+14; 140C5B610 serializes positive
/// amounts in this order, calling 140BA5C20 for each raw tag (not its alias target).
pub const COUNT: usize = 0x14;
