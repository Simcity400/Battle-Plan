//! Command-ability definition. These constants do not establish an hourly
//! world walk or a complete availability evaluator.

/// Primary definition vtable.
pub const VTABLE_RVA: usize = 0x02a0_13e8;
/// Secondary definition vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02a0_1438;
/// Secondary subobject offset.
pub const SECONDARY_BASE: usize = 0x08;
/// Valid-definition byte tested before setting the subunit unlock marker.
pub const VALID: usize = 0x10;
/// Token string with inline or heap storage.
pub const TOKEN: usize = 0x70;
/// Token length, u64.
pub const TOKEN_LENGTH: usize = 0x80;
/// Token capacity, u64, selecting inline or heap storage.
pub const TOKEN_CAPACITY: usize = 0x88;
/// Target kind, i32; the country-wide value uses no caster or formation match.
pub const TARGET_KIND: usize = 0x90;
/// Allocated-cost flag byte selecting persistent cost allocation.
pub const ALLOCATE_COST: usize = 0x94;
/// Duration hours, i32; the indefinite value retains an active record.
pub const DURATION_HOURS: usize = 0xa8;
/// Cooldown hours, i32; values below one remove the record.
#[cfg(test)]
pub const COOLDOWN_HOURS: usize = 0xac;
/// Fixed-point cost scale for i64 products and divisions.
pub const COST_SCALE: i64 = 100_000;
/// Constructor installing both vtables and the allowed-condition trigger.
pub const CONSTRUCTOR_RVA: usize = 0x0108_f390;
