//! Decoded DDS mip record retained by `native_texture_image_layout`.
//! These source records are distinct from D3D subresource descriptors.
//! Unidentified padding is not treated as data.

/// Decoded mip-record stride.
pub const STRIDE: usize = 0x20;
/// Mip-level u32 index selecting the destination subresource.
pub const LEVEL: usize = 0;
/// Cube-face u32 index, zero for ordinary 2D rows.
pub const FACE: usize = 4;
/// Mip u32 width used for row pitch.
pub const WIDTH: usize = 8;
/// Mip u32 height used for slice pitch.
pub const HEIGHT: usize = 0x0c;
/// Encoded mip u32 byte length.
pub const BYTE_LENGTH: usize = 0x10;
/// Encoded mip-byte pointer, with BGR expansion applied separately.
pub const BYTES: usize = 0x18;
