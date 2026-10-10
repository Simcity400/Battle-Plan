//! CArmyManpowerValues embedded in divisions, deployment and garrisons.
//! Rows use manpower_value_entry's layout. Deployment totals sum positive i32
//! values into i64 and omit the aggregate for an empty array; those are consumer
//! choices, independent of the stored representation.

/// Embedded manpower-values vtable.
pub const VTABLE_RVA: usize = 0x2943900;
/// Data pointer for eight-byte entries.
pub const DATA: usize = 0x08;
/// Capacity word; negative headers are refused.
pub const CAPACITY: usize = 0x10;
/// Signed i32 count. Serialization keeps positive amounts in occurrence order
/// with each raw tag, independently of its alias target.
pub const COUNT: usize = 0x14;
