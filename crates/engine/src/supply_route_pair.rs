//! Anonymous sorted province/node-index pair in a supply companion route vector.

/// Route pair stride; old d81_decode_route_pairs, attach_log.rs:140455, decodes chunks_exact(8).
pub const STRIDE: usize = 8;
/// Province id, native i32; old d81_decode_route_pairs reads the first four bytes and requires positive ids.
pub const PROVINCE_ID: usize = 0;
/// Supply-node array index, native u32; old d81_decode_route_pairs reads the second four bytes.
pub const NODE_INDEX: usize = 4;
