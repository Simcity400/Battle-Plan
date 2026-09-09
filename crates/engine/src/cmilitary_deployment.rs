//! CMilitaryDeployment, using the primary object as every offset's base.
//!
//! Training values are native i64 fixed point. The old deployment lane
//! converted them to f64 and omitted zero values; neither conversion nor
//! omission establishes engine absence. The equipment pool's array begins
//! at deployment +0x40 and its allow-zero byte is at deployment +0x58;
//! their pool-relative constants live in cequipment_variant_pool.

/// CMilitaryDeployment primary vtable; offsets.rs:439 and pinned RTTI confirm the primary subobject.
pub const VTABLE_RVA: usize = 0x29a9d38;
/// CMilitaryDeployment secondary vtable; offsets.rs:440 and pinned RTTI locate this subobject at +0x18.
pub const SECONDARY_VTABLE_RVA: usize = 0x29a9d70;
/// CMilitaryDeployment secondary vptr offset; offsets.rs:441 and read_deployment_line check +0x18.
pub const SECONDARY_VTABLE: usize = 0x18;
/// CMilitaryDeployment embedded CEquipmentVariantPool; offsets.rs:435 and read_deployment_equipment check +0x20.
pub const EQUIPMENT_POOL: usize = 0x20;
/// CMilitaryDeployment training, i64 fixed point; offsets.rs:442 and read_deployment_line read +0x60 before f64 conversion.
pub const TRAINING: usize = 0x60;
/// CMilitaryDeployment maximum training, i64 fixed point; offsets.rs:443 and read_deployment_line read +0x68.
pub const MAX_TRAINING: usize = 0x68;
/// CMilitaryDeployment available CArmyManpowerValues; offsets.rs:444 and read_deployment_line walk embedded +0x70.
pub const MANPOWER_HAVE: usize = 0x70;
/// CMilitaryDeployment needed CArmyManpowerValues; offsets.rs:445 and read_deployment_line walk embedded +0x90.
pub const MANPOWER_NEED: usize = 0x90;
/// CMilitaryDeployment maximum manpower, i32; offsets.rs:446 and read_deployment_line read +0xB0 before i64 widening.
pub const MAX_MANPOWER: usize = 0xb0;
/// CMilitaryDeployment CReferencedDivisionTemplate pointer; offsets.rs:447 and read_deployment_line check +0xB8.
pub const TEMPLATE: usize = 0xb8;
/// CMilitaryDeployment native training denominator; read_deployment_line applies offsets.rs:462's 100,000 scale.
pub const FIXED_POINT_SCALE: i64 = 100_000;
