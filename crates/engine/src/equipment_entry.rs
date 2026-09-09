//! Native equipment entry shared by division banks, production stockpiles,
//! market stockpiles, convoy pools, air-wing banks, deployment and upgrade delivery pools.
//!
//! The concrete C++ pair/record name has not been established. Signed amounts
//! are retained before any consumer's clamping, filtering or display rounding.

pub const STRIDE: usize = 0x10;
/// Equipment-entry CEquipmentVariant pointer; old division reader at attach_log.rs:131655 and market-stockpile reader :138169 read the first qword.
pub const VARIANT: usize = 0x00;
/// Equipment-entry amount, i64 native fixed point; old division, market stockpile and D93 readers read the second qword.
pub const AMOUNT: usize = 0x08;
/// Equipment-entry native fixed-point denominator; old market serializer interpretation and D93 lib.rs:65 use 100,000.
pub const FIXED_POINT_SCALE: i64 = 100_000;
