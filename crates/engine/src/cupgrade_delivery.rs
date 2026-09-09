//! CUpgradeDelivery with an embedded CEquipmentVariantPool.
//!
//! The old D93 reader exposes progress only when status is 1. Other status
//! meanings and consistency at the hourly boundary remain unproven.

/// CUpgradeDelivery primary vtable; pinned-exe RTTI confirms old D93 lib.rs:52.
pub const VTABLE_RVA: usize = 0x29bb2e0;
/// CUpgradeDelivery status, u32; old D93 read_deliveries reads +0x08.
pub const STATUS: usize = 0x08;
/// CUpgradeDelivery status whose progress is read; old D93 read_deliveries admits +0x10 only when status equals 1.
pub const PROGRESS_AVAILABLE_STATUS: u32 = 1;
/// CUpgradeDelivery progress, i64 native fixed point; old D93 read_deliveries reads +0x10 when status is 1.
pub const PROGRESS: usize = 0x10;
/// CUpgradeDelivery total progress, i64 native fixed point; old D93 read_deliveries reads +0x18 for every status.
pub const TOTAL_PROGRESS: usize = 0x18;
/// CUpgradeDelivery embedded CEquipmentVariantPool; old D93 read_deliveries validates its vtable at +0x20.
pub const PRODUCED_EQUIPMENT_POOL: usize = 0x20;
/// CUpgradeDelivery numeric scale; old D93 lib.rs:65 records native fixed point with denominator 100,000.
pub const FIXED_POINT_SCALE: i64 = 100_000;
