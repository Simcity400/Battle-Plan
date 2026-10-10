//! Parsed shader-pass declaration for presentation.

/// Pass row stride.
pub const STRIDE: usize = 0xd8;
/// Pass-name string compared with the requested effect name.
pub const NAME: usize = 0;
/// Vertex shader-name string at +0x20.
pub const VERTEX_SHADER_NAME: usize = 0x20;
/// Pixel shader-name string at +0x40.
pub const PIXEL_SHADER_NAME: usize = 0x40;
/// Blend declaration selector string at +0x60.
pub const BLEND_STATE_NAME: usize = 0x60;
/// Depth/stencil declaration selector string at +0x80.
pub const DEPTH_STENCIL_STATE_NAME: usize = 0x80;
/// Rasterizer declaration selector string at +0xa0.
pub const RASTERIZER_STATE_NAME: usize = 0xa0;
/// Pass parser and append routine copying the reader payload.
pub const READ_DECLARATION_RVA: usize = 0x0234_8f80;
/// Pass selection and program construction resolving named states and vertex/pixel shaders.
pub const RESOLVE_RVA: usize = 0x0234_4df0;
