//! Texture conversion and upload metadata for presentation. Resource
//! cache lifetime and execution order are unestablished. Format mappings
//! describe game texture codes rather than viewer fallback choices.

/// D3D11 texture creator lowering descriptors and uploading source rows.
pub const CREATE_TEXTURE_RVA: usize = 0x023f_b4f0;
/// Mip-to-upload conversion with compressed pitches and BGR expansion.
pub const BUILD_UPLOAD_ROWS_RVA: usize = 0x023f_c160;
/// BGR expansion to B,G,R,0 without alpha repair.
pub const EXPAND_BGR_RVA: usize = 0x023f_8e90;
/// Pixel-format bit count before pitch calculation; formats 11/12 use 32 upload bits.
pub const FORMAT_BITS_PER_PIXEL_RVA: usize = 0x023f_9e10;
/// Texture-code to DXGI conversion.
pub const FORMAT_TO_DXGI_RVA: usize = 0x0241_1320;
/// DDS DXT1 code retaining compressed blocks.
pub const FORMAT_DXT1: u32 = 1;
/// DDS DXT3 code retaining compressed blocks.
pub const FORMAT_DXT3: u32 = 3;
/// DDS DXT5 code retaining compressed blocks.
pub const FORMAT_DXT5: u32 = 5;
/// 24-bit BGR code whose upload expands each source triple.
pub const FORMAT_BGR24: u32 = 11;
/// 32-bit BGRA code preserving its fourth source byte.
pub const FORMAT_BGRA32: u32 = 13;
/// Fourth byte after BGR expansion; DXGI X-format sampling supplies opacity.
pub const EXPANDED_BGR_FOURTH_BYTE: u8 = 0;
/// Texture-code switch mapping to DXGI values.
pub const FORMAT_TO_DXGI_CASES: &[(u32, u32)] = &[
    (1, 0x47),
    (2, 0x48),
    (3, 0x4a),
    (4, 0x4b),
    (5, 0x4d),
    (6, 0x4e),
    (7, 0x55),
    (8, 0x56),
    (0x0c, 0x5d),
    (0x0d, 0x57),
    (0x0e, 0x5b),
    (0x10, 0x5d),
    (0x11, 0x41),
    (0x12, 0x3d),
    (0x13, 0x36),
    (0x14, 0x22),
    (0x15, 10),
    (0x16, 0x29),
    (0x17, 0x10),
    (0x18, 6),
    (0x19, 2),
];
/// Default DXGI result for every unlisted code, including code 11.
pub const FORMAT_TO_DXGI_DEFAULT: u32 = 0x58;
