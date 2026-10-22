//! Required-equipment row for garrison alerts. Its pointer identifies an
//! equipment type. Equal geometry with `equipment_entry` does not establish
//! that both rows share a type.

/// Required-equipment row stride.
pub const STRIDE: usize = 0x10;
/// Equipment-type pointer, followed through its archetype link (`cequipment_type::ARCHETYPE`).
#[cfg(test)]
pub const EQUIPMENT_TYPE: usize = 0x00;
/// Signed i64 fixed-point amount.
pub const AMOUNT: usize = 0x08;
/// Fixed-point denominator.
pub const FIXED_POINT_SCALE: i64 = 100_000;
