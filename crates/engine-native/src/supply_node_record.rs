//! Hub motorization settings in the country supply system
//! (`ccountry_supply_system::HUB_SETTINGS_DATA`). This 0x28-byte record is distinct from the
//! 0xe8-byte simulation node.

/// Settings record stride.
pub const STRIDE: usize = 0x28;
/// Province ID, i32.
pub const PROVINCE: usize = 0x00;
/// Supply-node type, i32.
#[cfg(test)]
pub const NODE_TYPE: usize = 0x04;
/// Motorization map data pointer.
pub const MOTORIZATION_DATA: usize = 0x10;
/// Motorization map count, u32.
pub const MOTORIZATION_COUNT: usize = 0x1c;
