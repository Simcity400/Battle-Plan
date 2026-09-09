//! Native request-vector entry in CArmyUpgradesRequests; engine type name unknown.

/// CArmyUpgradesRequests request-row stride; old D93 lib.rs:29 and read_request_rows use 0x18 bytes.
pub const STRIDE: usize = 0x18;
/// Upgrade request row's CEquipmentVariant pointer; old D93 read_request_rows reads the first qword and checks its vtable.
pub const EQUIPMENT_VARIANT: usize = 0x00;
/// Upgrade request amount, i64 in engine fixed point; old D93 read_request_rows reads +0x08 without narrowing.
pub const REQUEST: usize = 0x08;
/// Upgrade request total, i64 in engine fixed point; old D93 read_request_rows reads +0x10 without narrowing.
pub const TOTAL: usize = 0x10;
/// Upgrade request numeric scale; old D93 lib.rs:65 records native fixed point with denominator 100,000.
pub const FIXED_POINT_SCALE: i64 = 100_000;
