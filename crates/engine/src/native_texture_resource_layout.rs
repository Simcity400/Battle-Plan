//! File-texture wrapper storing a shader-resource view followed by a texture.
//! The opaque render-target wrapper gives its first field a different meaning.

/// File-texture resource allocation size.
pub const SIZE: usize = 0x58;
/// File-texture wrapper allocation entry.
pub const ALLOCATE_RVA: usize = 0x023f_d6d0;
/// File-texture creation entry copying image metadata, then creating texture and view.
pub const CREATE_RVA: usize = 0x023f_fc90;
/// Shader-resource-view pointer.
pub const SHADER_RESOURCE_VIEW: usize = 0;
/// D3D texture pointer.
pub const TEXTURE: usize = 8;
/// Optional replacement-wrapper pointer, followed before selecting the effective view.
pub const BINDING_REDIRECT: usize = 0x18;
/// Copied image metadata prefix: dimensions, mips, format, cubemap and creation flags.
pub const IMAGE_METADATA: usize = 0x20;
/// Cached texture u32 width.
pub const WIDTH: usize = 0x38;
/// Cached texture u32 height.
pub const HEIGHT: usize = 0x3c;
/// Cached texture u32 format.
pub const FORMAT: usize = 0x40;
/// Cached u32 bytes per pixel, converted from format bit count.
pub const BYTES_PER_PIXEL: usize = 0x44;
