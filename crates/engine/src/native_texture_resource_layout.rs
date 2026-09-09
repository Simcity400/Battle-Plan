//! Anonymous texture wrapper allocated by FUN1423FD6D0 and populated by FUN1423FFC90.
//! The file-texture path stores an SRV first and a texture second; this is not the
//! opaque render-target wrapper, whose first field has a different meaning.

/// File-texture resource allocation size; retained FUN1423FD6D0 requests 0x58 before invoking the creation dispatch.
pub const SIZE: usize = 0x58;
/// File-texture wrapper allocation entry; container_sprite_resources_gpu.rs:26 records the FUN1423FD6D0 -> FUN1423FFC90 -> FUN1423FB4F0 chain.
pub const ALLOCATE_RVA: usize = 0x023f_d6d0;
/// File-texture wrapper creation entry; retained FUN1423FFC90 copies the image metadata, creates the texture, then its shader-resource view.
pub const CREATE_RVA: usize = 0x023f_fc90;
/// Shader-resource-view pointer; retained FUN1423FFC90 stores the successful CreateShaderResourceView result in wrapper+0.
pub const SHADER_RESOURCE_VIEW: usize = 0;
/// D3D texture pointer; retained FUN1423FFC90 stores FUN1423FB4F0's nonzero result at wrapper+8.
pub const TEXTURE: usize = 8;
/// Optional replacement wrapper pointer; FUN1423FD6D0 zeroes +18 and retained FUN1424005A0 follows it before reading the effective wrapper's SRV.
pub const BINDING_REDIRECT: usize = 0x18;
/// Embedded copied image metadata prefix; FUN1423FFC90 copies width/height/mips/format/cubemap/creation flags to wrapper+20..37.
pub const IMAGE_METADATA: usize = 0x20;
/// Cached texture width u32; retained FUN1423FFC90 writes input image+0 to wrapper+38.
pub const WIDTH: usize = 0x38;
/// Cached texture height u32; retained FUN1423FFC90 writes input image+4 to wrapper+3C.
pub const HEIGHT: usize = 0x3c;
/// Cached native format u32; retained FUN1423FFC90 writes input image+C to wrapper+40.
pub const FORMAT: usize = 0x40;
/// Cached bytes-per-pixel u32; retained FUN1423FFC90 shifts FUN1423F9E10's format-bit count right by three before storing wrapper+44.
pub const BYTES_PER_PIXEL: usize = 0x44;
