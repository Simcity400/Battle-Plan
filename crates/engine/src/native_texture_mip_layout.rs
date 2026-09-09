//! Anonymous decoded DDS mip record retained by native_texture_image_layout.
//! These are engine source records, distinct from the D3D subresource descriptors
//! that FUN1423FC160 builds from them. Unidentified padding is not treated as data.

/// Decoded mip-record stride; retained FUN142410030 allocates and copies rows in 0x20-byte units and FUN1423FC160 advances by eight u32 words.
pub const STRIDE: usize = 0x20;
/// Mip level u32; FUN142410030 writes the inner mip index at +0 and FUN1423FC160 uses it to choose the destination subresource.
pub const LEVEL: usize = 0;
/// Cube face u32; FUN142410030 writes zero for ordinary 2D rows and its outer cube-face index at +4 for cubemaps.
pub const FACE: usize = 4;
/// Mip width u32; FUN142410030 stores each shrinking width at +8, consumed by FUN1423FC160's row-pitch calculation.
pub const WIDTH: usize = 8;
/// Mip height u32; FUN142410030 stores each shrinking height at +C, consumed by FUN1423FC160's slice-pitch calculation.
pub const HEIGHT: usize = 0x0c;
/// Encoded mip byte length u32; FUN142410030 stores +10 and FUN1423F8E90 reads it before expanding BGR triples.
pub const BYTE_LENGTH: usize = 0x10;
/// Encoded mip bytes pointer; FUN142410030 stores +18 and FUN1423FC160 uses that data unless BGR expansion is required.
pub const BYTES: usize = 0x18;
