//! DDS texture decoding for the GUI resource path (no mip skipped: sprite loaders pass a zero
//! skip).
//!
//! The engine reads its header fields without checking them: the four-character code alone
//! picks a block format (`DXT1`, `DXT3`, `DXT5`; `DXT2` and `DXT4` fail the load), with no look
//! at the `DDPF_FOURCC` flag; any other surface takes its bytes per pixel from the flags and the
//! bit count (4 when `flags & 0x20240` is clear or the count is 32, else 3 for 24, 2 for 16,
//! else 1) and never reads the channel masks except to tell the three 16-bit forms apart. A
//! 32-bit surface is `B8G8R8A8` (format `0xD`) whatever its masks and alpha flag say, a 24-bit
//! one `B8G8R8` (`0xB`, uploaded with an ignored fourth byte), a 16-bit one kept in its own
//! words (formats 7 and 8, uploaded as `B5G6R5_UNORM` and `B5G5R5A1_UNORM` unexpanded, so
//! a 5-bit channel samples as `c / 31`, never as an 8-bit value), so a DX10 file (its 20 extra
//! header bytes skipped only when the flags are exactly `DDPF_FOURCC`) is read as `B8G8R8A8`
//! whatever `dxgiFormat` it names. A block texture's extent is rounded up to whole blocks:
//! that rounded extent is the texture's size, which every sprite size and UV divides. The
//! chain of levels halves both sides from there, stops after a level with a side of 1, at the
//! header's mip count (at least one level), or at the first level the file does not hold,
//! keeping the levels before it.
//! A cube map (`DDSCAPS2_CUBEMAP`) takes the six-face branch: every face, in DDS order, holds
//! that chain in turn, and the loader reads them without checking the file's length.
//!
//! DX10 BC7 alone takes the supported exception below: its logical DDS extent and complete
//! authored mip chain, never the native block-rounded chain.

use super::{Failure, Zero, oversized};
use crate::image::dds::{MipLevel, Texture, TextureFormat};

const HEADER: usize = 128;
/// `DDSCAPS2_CUBEMAP`, which sends `load_dds` to its six-face branch.
const CUBE_MAP: u32 = 0x200;
/// `DDSCAPS2_VOLUME`, which the BC7 branch refuses.
const VOLUME: u32 = 0x20_0000;
/// The DX10 header's `DDS_RESOURCE_MISC_TEXTURECUBE`, which the BC7 branch refuses.
const DX10_CUBE: u32 = 4;

fn code(name: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*name)
}

/// The native pixel format a non-block surface takes.
#[derive(Clone, Copy)]
enum Pixels {
    /// 7: `B5G6R5`.
    B5G6R5,
    /// 8: `A1R5G5B5`.
    A1R5G5B5,
    /// 9: `X1R5G5B5`, promoted to format 8 with every top bit set.
    X1R5G5B5,
    /// 0xB: `B8G8R8`, uploaded as `B8G8R8X8`.
    B8G8R8,
    /// 0xD: `B8G8R8A8`.
    B8G8R8A8,
}

impl Pixels {
    /// What a zeroed texel of the format samples as once texture decoding has promoted
    /// `X1R5G5B5` to `A1R5G5B5`: what the padding fills with (for a 16-bit format, its zeroed
    /// word).
    fn zero(self) -> Zero {
        match self {
            Self::B5G6R5 | Self::B8G8R8 => Zero::Opaque,
            Self::A1R5G5B5 | Self::X1R5G5B5 | Self::B8G8R8A8 => Zero::Transparent,
        }
    }

    fn bytes(self) -> usize {
        match self {
            Self::B5G6R5 | Self::A1R5G5B5 | Self::X1R5G5B5 => 2,
            Self::B8G8R8 => 3,
            Self::B8G8R8A8 => 4,
        }
    }

    /// The texture format a level's texels are kept in.
    fn format(self) -> TextureFormat {
        match self {
            Self::B5G6R5 => TextureFormat::B5G6R5,
            Self::A1R5G5B5 | Self::X1R5G5B5 => TextureFormat::B5G5R5A1,
            Self::B8G8R8 | Self::B8G8R8A8 => TextureFormat::Rgba8,
        }
    }

    /// One level's texels in [`Self::format`]: straight RGBA8 for the 8-bit forms, the file's
    /// own words for the 16-bit ones, `X1R5G5B5`'s with the top bit set.
    fn convert(self, source: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(source.len() / self.bytes() * self.format().unit().1);
        for texel in source.chunks_exact(self.bytes()) {
            match self {
                Self::B8G8R8A8 => out.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]),
                Self::B8G8R8 => out.extend_from_slice(&[texel[2], texel[1], texel[0], 255]),
                Self::B5G6R5 | Self::A1R5G5B5 => out.extend_from_slice(texel),
                // Set the top bit to make the texel opaque, then keep format 8.
                Self::X1R5G5B5 => out.extend_from_slice(&[texel[0], texel[1] | 0x80]),
            }
        }
        out
    }
}

enum Form {
    Blocks(TextureFormat, usize),
    Pixels(Pixels),
}

/// Load a file with DDS magic: `Ok(None)` for `DXT2`/`DXT4`, so the decoding chain goes on.
/// A zero side is kept: texture creation refuses it unless `pad_power_of_two` widens it first.
pub(super) fn load(bytes: &[u8]) -> Result<Option<(Texture, Zero)>, Failure> {
    if bytes.len() < HEADER {
        return Err(Failure::Undefined(format!(
            "a {}-byte DDS: DDS decoding reads its 128-byte header past the end of the file",
            bytes.len()
        )));
    }
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let (height, width, mip_count) = (word(12), word(16), word(28));
    let (flags, fourcc, bits) = (word(80), word(84), word(88));
    let masks = [word(92), word(96), word(100), word(104)];
    let start = if flags == 4 && fourcc == code(b"DX10") {
        HEADER + 20
    } else {
        HEADER
    };
    // The BC7-only exception decodes DDS logical mips and the header's colour space.
    // The ordinary DDS path ignores dxgiFormat, reads BC7 as BGRA and keeps no levels,
    // which texture creation refuses. Other DX10 files use that path; BC1-3 keeps its
    // block-rounded chain below.
    if start == HEADER + 20 && bytes.len() >= HEADER + 4 && matches!(word(128), 98 | 99) {
        return bc7(bytes, word, word(128) == 99);
    }
    let form = if fourcc == code(b"DXT1") {
        Form::Blocks(TextureFormat::Bc1, 8)
    } else if fourcc == code(b"DXT2") || fourcc == code(b"DXT4") {
        return Ok(None);
    } else if fourcc == code(b"DXT3") {
        Form::Blocks(TextureFormat::Bc2, 16)
    } else if fourcc == code(b"DXT5") {
        Form::Blocks(TextureFormat::Bc3, 16)
    } else if flags & 0x20240 == 0 || bits == 32 {
        Form::Pixels(Pixels::B8G8R8A8)
    } else if bits == 24 {
        Form::Pixels(Pixels::B8G8R8)
    } else if bits == 16 {
        let rgb = flags & 0x40 != 0;
        Form::Pixels(if rgb && masks[..3] == [0xF800, 0x7E0, 0x1F] {
            Pixels::B5G6R5
        } else if flags & 0x41 == 0x41 && masks == [0x7C00, 0x3E0, 0x1F, 0x8000] {
            Pixels::A1R5G5B5
        } else if rgb && masks[..3] == [0x7C00, 0x3E0, 0x1F] {
            Pixels::X1R5G5B5
        } else {
            return Err(Failure::Undefined(format!(
                "a 16-bit DDS with flags {flags:#x} and masks {masks:#x?}: DDS decoding \
                 loads it as format 0, uploaded as B8G8R8X8 with 0-bit rows: the texture does \
                 not hold the file's texels"
            )));
        })
    } else {
        return Err(Failure::Undefined(format!(
            "a {bits}-bit DDS with flags {flags:#x}: DDS decoding loads it as format 0x1A, \
             uploaded as B8G8R8X8 with 0-bit rows: the texture does not hold the file's texels"
        )));
    };
    let cube = word(112) & CUBE_MAP != 0;
    let (block, unit) = match &form {
        Form::Blocks(_, size) => (true, *size),
        Form::Pixels(pixels) => (false, pixels.bytes()),
    };
    let (width, height) = if block {
        (width.wrapping_add(3) & !3, height.wrapping_add(3) & !3)
    } else {
        (width, height)
    };
    oversized(width, height)?;
    let level_bytes = |w: u32, h: u32| {
        let (w, h) = (w as usize, h as usize);
        if block {
            w.div_ceil(4) * h.div_ceil(4) * unit
        } else {
            w * h * unit
        }
    };
    let convert = |source: &[u8]| match &form {
        Form::Blocks(..) => source.to_vec(),
        Form::Pixels(pixels) => pixels.convert(source),
    };
    let (format, zero) = match &form {
        Form::Blocks(format, _) => (*format, Zero::Transparent),
        Form::Pixels(pixels) => (pixels.format(), pixels.zero()),
    };
    if cube {
        // The six-face branch: each face in DDS order holds its whole chain,
        // halved from the (block-rounded) extent, ending after a level with a side of 1 or at the
        // header's count, one level at least. It records where each level lies in the file
        // without checking the file's length.
        let mut offset = start;
        let mut faces = Vec::with_capacity(6);
        for face in 0..6 {
            let (mut level_width, mut level_height) = (width, height);
            let mut levels = Vec::new();
            loop {
                let size = level_bytes(level_width, level_height);
                let source = bytes.get(offset..offset + size).ok_or_else(|| {
                    Failure::Undefined(format!(
                        "a cube-map DDS whose face {face} level {} runs past the file: \
                         DDS decoding reads the six faces without checking the file's \
                         length, so texture creation uploads memory past it",
                        levels.len()
                    ))
                })?;
                levels.push(MipLevel {
                    width: level_width,
                    height: level_height,
                    data: convert(source),
                });
                offset += size;
                if level_width == 1
                    || level_height == 1
                    || levels.len() as u64 >= u64::from(mip_count)
                {
                    break;
                }
                level_width >>= 1;
                level_height >>= 1;
            }
            faces.push(levels);
        }
        return Ok(Some((
            Texture {
                format,
                width,
                height,
                faces: 6,
                mips: faces,
            },
            zero,
        )));
    }
    let (mut level_width, mut level_height) = (width, height);
    let mut offset = start;
    let mut levels = Vec::new();
    loop {
        let size = level_bytes(level_width, level_height);
        let Some(source) = bytes.get(offset..offset + size) else {
            break;
        };
        levels.push(MipLevel {
            width: level_width,
            height: level_height,
            data: convert(source),
        });
        offset += size;
        if level_width == 1 || level_height == 1 || levels.len() as u64 >= u64::from(mip_count) {
            break;
        }
        // Two sides of 0 never reach 1: the game appends empty levels up to the header's count.
        // Past the second, none changes the outcome (the texture is refused unpadded, and its
        // second padded level is already empty), so decoding stops there.
        if level_width == 0 && level_height == 0 && levels.len() == 2 {
            break;
        }
        level_width >>= 1;
        level_height >>= 1;
    }
    if levels.is_empty() {
        return Err(Failure::Undefined(
            "a DDS whose first level runs past the file: DDS decoding keeps no level, and \
             texture creation then creates the texture without initial data"
                .to_owned(),
        ));
    }
    Ok(Some((
        Texture {
            format,
            width,
            height,
            faces: 1,
            mips: vec![levels],
        },
        zero,
    )))
}

/// The BC7-only DX10 branch: DDS logical sizes and complete authored mip chains.
fn bc7(
    bytes: &[u8],
    word: impl Fn(usize) -> u32,
    srgb: bool,
) -> Result<Option<(Texture, Zero)>, Failure> {
    let refuse = |why: String| Failure::Undecodable(format!("a DX10 BC7 DDS: {why}"));
    if bytes.len() < HEADER + 20 {
        return Err(refuse("truncated DX10 header".to_owned()));
    }
    let (width, height) = (word(16), word(12));
    if width == 0 || height == 0 {
        return Err(refuse(format!("zero extent {width}x{height}")));
    }
    oversized(width, height)?;
    if word(132) != 3 || word(24) > 1 || word(112) & VOLUME != 0 {
        return Err(refuse(format!(
            "unsupported resource dimension {} or volume depth {}",
            word(132),
            word(24)
        )));
    }
    if word(112) & CUBE_MAP != 0 || word(136) & DX10_CUBE != 0 {
        return Err(refuse(
            "cube maps are not supported by the BC7 reader".to_owned(),
        ));
    }
    if word(140) != 1 {
        return Err(refuse(format!(
            "unsupported texture array size {}",
            word(140)
        )));
    }
    // DDSD_MIPMAPCOUNT: without it the mipMapCount word does not declare a chain.
    let count = if word(8) & 0x2_0000 != 0 {
        word(28).max(1)
    } else {
        1
    };
    let maximum = width.max(height).ilog2() + 1;
    if count > maximum {
        return Err(refuse(format!(
            "declares {count} mip levels but {width}x{height} permits only {maximum}"
        )));
    }
    let mut levels = Vec::new();
    let mut offset = HEADER + 20;
    for level in 0..count {
        let (width, height) = ((width >> level).max(1), (height >> level).max(1));
        let size = width.div_ceil(4) as usize * height.div_ceil(4) as usize * 16;
        let end = offset + size;
        let data = bytes.get(offset..end).ok_or_else(|| {
            refuse(format!(
                "truncated declared mip chain at level {level} ({width}x{height}): \
                 needs bytes {offset}..{end}, file holds {} bytes",
                bytes.len()
            ))
        })?;
        levels.push(MipLevel {
            width,
            height,
            data: data.to_vec(),
        });
        offset = end;
    }
    Ok(Some((
        Texture {
            format: TextureFormat::Bc7 { srgb },
            width,
            height,
            faces: 1,
            mips: vec![levels],
        },
        Zero::Transparent,
    )))
}

#[cfg(test)]
mod tests;
