//! Equipment module slot/definition entry; concrete C++ type name unproven.
//! Preserve the collection's stored occurrence order.

/// Equipment-module entry stride.
pub const STRIDE: usize = 0x10;
/// Equipment-module entry u32 slot token.
pub const SLOT: usize = 0x00;
/// Equipment-module entry CEquipmentModule*.
pub const DEFINITION: usize = 0x08;
