//! Hub motorization settings in the country supply system
//! (`ccountry_supply_system::HUB_SETTINGS_DATA`). This 0x28-byte record is distinct from the
//! 0xe8-byte simulation node.

/// Settings record stride.
pub const STRIDE: usize = 0x28;
/// Province ID, i32.
pub const PROVINCE: usize = 0x00;
/// Supply-node type, i32.
pub const NODE_TYPE: usize = 0x04;
/// Disabled-for-allies byte, serialized separately from the actor map.
pub const DISABLED_FOR_ALLIES: usize = 0x08;
/// Motorization map data pointer.
pub const MOTORIZATION_DATA: usize = 0x10;
/// Motorization map capacity; insertion stores raw actor/level pairs.
pub const MOTORIZATION_CAPACITY: usize = 0x18;
/// Motorization map count, u32.
pub const MOTORIZATION_COUNT: usize = 0x1c;
