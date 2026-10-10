//! Sorted province/node-index pair in a supply companion route vector.

/// Route-pair stride.
pub const STRIDE: usize = 8;
/// Province ID, i32; positive IDs are required.
pub const PROVINCE_ID: usize = 0;
/// Supply-node array index, u32.
pub const NODE_INDEX: usize = 4;
