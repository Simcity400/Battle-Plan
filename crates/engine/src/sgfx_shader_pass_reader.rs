//! Shader-pass declaration reader for presentation.

/// Reader vtable.
pub const VTABLE_RVA: usize = 0x02b3_b128;
/// Pass-name string following the vtable pointer.
pub const NAME: usize = 0x08;
/// Vertex shader-name string copied to pass row +0x20.
pub const VERTEX_SHADER_NAME: usize = 0x28;
/// Pixel shader-name string copied to pass row +0x40.
pub const PIXEL_SHADER_NAME: usize = 0x48;
/// Blend declaration selector string at +0x68.
pub const BLEND_STATE_NAME: usize = 0x68;
/// Depth/stencil declaration selector string at +0x88.
pub const DEPTH_STENCIL_STATE_NAME: usize = 0x88;
/// Rasterizer declaration selector string at +0xa8.
pub const RASTERIZER_STATE_NAME: usize = 0xa8;
