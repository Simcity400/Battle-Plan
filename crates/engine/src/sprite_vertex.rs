//! Ordinary sprite vertex emitted for CSpriteType drawing; native C++ type unnamed.
//! This is presentation geometry, not a recorded state row or a D3D SDK struct.

/// Sprite vertex stride; retained FUN14222CB90 builds four stack records 20 bytes apart, matched by game_ui/src/container_sprite_draw.rs:347-374.
pub const STRIDE: usize = 20;
/// Sprite position X, native f32; retained FUN14222CB90 fills the first word of each 20-byte stack record, matching container_sprite_pipeline.rs:152-174.
pub const X: usize = 0;
/// Sprite position Y, native f32; container_sprite_draw.rs:347-355 preserves the second position component in the native packed order.
pub const Y: usize = 4;
/// Sprite position Z, native f32; container_sprite_draw.rs:347-355 preserves the third position component in the native packed order.
pub const Z: usize = 8;
/// Sprite texture U, native f32; container_sprite_pipeline.rs:165-174 places TEXCOORD after the three position floats.
pub const U: usize = 12;
/// Sprite texture V, native f32; container_sprite_draw.rs:347-355 packs the second texture component after U.
pub const V: usize = 16;
/// Ordinary sprite strip vertex count; FUN14222D5A0 ends with the four-vertex draw dispatch, retained by container_sprite_draw.rs:371-374.
pub const VERTICES_PER_STRIP: usize = 4;
