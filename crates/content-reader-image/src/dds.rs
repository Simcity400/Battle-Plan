
use std::io;

use super::{Rgba8Image, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Bc1,
    Bc2,
    Bc3,
    Bc7 { srgb: bool },
    Rgba8,
    B5G6R5,
    B5G5R5A1,
}

impl TextureFormat {
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            Self::Bc1 => Some(8),
            Self::Bc2 | Self::Bc3 | Self::Bc7 { .. } => Some(16),
            Self::Rgba8 | Self::B5G6R5 | Self::B5G5R5A1 => None,
        }
    }

    pub fn unit(self) -> (u32, usize) {
        match self {
            Self::Bc1 => (4, 8),
            Self::Bc2 | Self::Bc3 | Self::Bc7 { .. } => (4, 16),
            Self::Rgba8 => (1, 4),
            Self::B5G6R5 | Self::B5G5R5A1 => (1, 2),
        }
    }

    pub fn level_bytes(self, width: u32, height: u32) -> io::Result<usize> {
        let (side, unit) = self.unit();
        let units = |texels: u32| (texels as usize).div_ceil(side as usize);
        let bytes = units(width) * units(height) * unit;
        if bytes == 0 {
            return Err(invalid("DDS mip level has no pixels"));
        }
        Ok(bytes)
    }

    pub fn sample_16(self, word: u16) -> Option<[f32; 4]> {
        let field = |shift: u16, max: u16| f32::from((word >> shift) & max) / f32::from(max);
        match self {
            Self::B5G6R5 => Some([field(11, 31), field(5, 63), field(0, 31), 1.0]),
            Self::B5G5R5A1 => Some([field(10, 31), field(5, 31), field(0, 31), field(15, 1)]),
            Self::Bc1 | Self::Bc2 | Self::Bc3 | Self::Bc7 { .. } | Self::Rgba8 => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub data: crate::Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
    pub faces: u32,
    pub mips: Vec<Vec<MipLevel>>,
}

impl Texture {
    pub fn from_rgba8<P: Into<crate::Bytes>>(image: Rgba8Image<P>) -> Self {
        Self {
            format: TextureFormat::Rgba8,
            width: image.width,
            height: image.height,
            faces: 1,
            mips: vec![vec![MipLevel {
                width: image.width,
                height: image.height,
                data: (image.pixels).into(),
            }]],
        }
    }

    pub fn base(&self) -> &MipLevel {
        &self.mips[0][0]
    }

    pub fn mip_count(&self) -> usize {
        self.mips[0].len()
    }
}
