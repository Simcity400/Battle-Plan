//! Texture types shared by the native map, interface and font readers.
//!
//! The native loader ([`super::native`]) supplies block-compressed bytes,
//! RGBA8 texels or the game's retained 16-bit formats.

use std::io;

use super::{Rgba8Image, invalid};

/// What the decoded bytes of every mip level hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    /// DXT1 blocks, 8 bytes per 4x4 block, kept as authored.
    Bc1,
    /// DXT3 blocks, 16 bytes per 4x4 block, kept as authored.
    Bc2,
    /// DXT5 blocks, 16 bytes per 4x4 block, kept as authored.
    Bc3,
    /// DX10 BC7 blocks, 16 bytes per 4x4 block, with the header's colour space.
    Bc7 { srgb: bool },
    /// Straight 8-bit R, G, B, A, row-major from the top-left corner.
    Rgba8,
    /// `DXGI_FORMAT_B5G6R5_UNORM`, as the game's DDS loader keeps a 5-6-5 surface (its format 7,
    /// `format_to_dxgi@142411320`, uploaded unexpanded by `build_upload_rows@1423FC160`): one
    /// little-endian word per texel, blue in bits 0-4, green in 5-10, red in 11-15, row-major
    /// from the top-left corner. It samples as [`Self::sample_16`] says.
    B5G6R5,
    /// `DXGI_FORMAT_B5G5R5A1_UNORM` (the game's format 8, likewise kept): one little-endian word
    /// per texel, blue in bits 0-4, green in 5-9, red in 10-14, alpha in 15.
    B5G5R5A1,
}

impl TextureFormat {
    /// `None` for an uncompressed format, which is not block based.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            Self::Bc1 => Some(8),
            Self::Bc2 | Self::Bc3 | Self::Bc7 { .. } => Some(16),
            Self::Rgba8 | Self::B5G6R5 | Self::B5G5R5A1 => None,
        }
    }

    /// The side in texels of the unit the format stores (a 4x4 block or one texel) and its
    /// bytes.
    pub fn unit(self) -> (u32, usize) {
        match self {
            Self::Bc1 => (4, 8),
            Self::Bc2 | Self::Bc3 | Self::Bc7 { .. } => (4, 16),
            Self::Rgba8 => (1, 4),
            Self::B5G6R5 | Self::B5G5R5A1 => (1, 2),
        }
    }

    /// Bytes one mip level of this size occupies, blocks included.
    pub fn level_bytes(self, width: u32, height: u32) -> io::Result<usize> {
        let (side, unit) = self.unit();
        let units = |texels: u32| (texels as usize).div_ceil(side as usize);
        let bytes = units(width) * units(height) * unit;
        if bytes == 0 {
            return Err(invalid("DDS mip level has no pixels"));
        }
        Ok(bytes)
    }

    /// What a texel word of a 16-bit format samples as, straight R, G, B, A: each field over its
    /// largest value (`c / 31`, `c / 63`), as the device converts a `_UNORM` texel, and alpha 1
    /// where the format has none. `None` for any other format.
    pub fn sample_16(self, word: u16) -> Option<[f32; 4]> {
        let field = |shift: u16, max: u16| f32::from((word >> shift) & max) / f32::from(max);
        match self {
            Self::B5G6R5 => Some([field(11, 31), field(5, 63), field(0, 31), 1.0]),
            Self::B5G5R5A1 => Some([field(10, 31), field(5, 31), field(0, 31), field(15, 1)]),
            Self::Bc1 | Self::Bc2 | Self::Bc3 | Self::Bc7 { .. } | Self::Rgba8 => None,
        }
    }
}

/// One mip level of one face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    /// Block bytes for a compressed format, otherwise its texels ([`TextureFormat`]).
    pub data: Vec<u8>,
}

/// A decoded DDS surface: one face for a plain texture, six for a cube map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    /// 1 for a plain texture, 6 for a cube map.
    pub faces: u32,
    /// Indexed `[face][mip]`; cube faces follow +X, -X, +Y, -Y, +Z, -Z.
    pub mips: Vec<Vec<MipLevel>>,
}

impl Texture {
    /// Wraps an already decoded image as a single-face, single-mip texture, so
    /// a `.tga` page and a `.dds` page reach the renderer in one shape.
    pub fn from_rgba8(image: Rgba8Image) -> Self {
        Self {
            format: TextureFormat::Rgba8,
            width: image.width,
            height: image.height,
            faces: 1,
            mips: vec![vec![MipLevel {
                width: image.width,
                height: image.height,
                data: image.pixels,
            }]],
        }
    }

    /// The largest mip of the first face, the one the game samples at level 0.
    pub fn base(&self) -> &MipLevel {
        &self.mips[0][0]
    }

    /// Number of mip levels every face carries.
    pub fn mip_count(&self) -> usize {
        self.mips[0].len()
    }
}
