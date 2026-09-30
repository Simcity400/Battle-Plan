//! `native_texture_image_layout::load_dds@142410030` (`gfx_dds_loader.cpp`), as the GUI loads a
//! texture through it (no mip skipped: `CSpriteType::load_base_texture@14222B2A0` passes a zero
//! skip).
//!
//! The engine reads its header fields without checking them: the four-character code alone
//! picks a block format (`DXT1`, `DXT3`, `DXT5`; `DXT2` and `DXT4` fail the load), with no look
//! at the `DDPF_FOURCC` flag; any other surface takes its bytes per pixel from the flags and the
//! bit count (4 when `flags & 0x20240` is clear or the count is 32, else 3 for 24, 2 for 16,
//! else 1) and never reads the channel masks except to tell the three 16-bit forms apart. A
//! 32-bit surface is `B8G8R8A8` (format `0xD`) whatever its masks and alpha flag say, a 24-bit
//! one `B8G8R8` (`0xB`, uploaded with an ignored fourth byte), a 16-bit one kept in its own
//! words (formats 7 and 8, which `format_to_dxgi@142411320` creates as `B5G6R5_UNORM` and
//! `B5G5R5A1_UNORM` and `build_upload_rows@1423FC160` uploads unexpanded, so a 5-bit channel
//! samples as `c / 31`, never as an 8-bit value), so a DX10 file (its 20 extra
//! header bytes skipped only when the flags are exactly `DDPF_FOURCC`) is read as `B8G8R8A8`
//! whatever `dxgiFormat` it names. A block texture's extent is rounded up to whole blocks:
//! that rounded extent is the texture's size, which every sprite size and UV divides. The
//! chain of levels halves both sides from there, stops after a level with a side of 1, at the
//! header's mip count (at least one level), or at the first level the file does not hold ("Expected
//! more data in the texture mipmaps than was actually present"), keeping the levels before it.

use super::{Failure, Zero, oversized};
use crate::image::dds::{MipLevel, Texture, TextureFormat};

const HEADER: usize = 128;
/// `DDSCAPS2_CUBEMAP`, which sends `load_dds` to its six-face branch.
const CUBE_MAP: u32 = 0x200;

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
    /// What a zeroed texel of the format samples as once `DecodeTextureImage` has promoted
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
                // DecodeTextureImage@1423F9A50: `texel | 0x8000`, then format 8.
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

/// `DXT2`/`DXT4` codes), so the chain goes on. A zero side is kept: `create_texture@1423FB4F0`
/// refuses it unless `pad_power_of_two` widens it first.
pub(super) fn load(bytes: &[u8]) -> Result<Option<(Texture, Zero)>, Failure> {
    if bytes.len() < HEADER {
        return Err(Failure::Undefined(format!(
            "a {}-byte DDS: load_dds@142410030 reads its 128-byte header past the end of the file",
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
                "a 16-bit DDS with flags {flags:#x} and masks {masks:#x?}: load_dds@142410030 \
                 loads it as format 0, which create_texture@1423FB4F0 uploads as B8G8R8X8 \
                 with rows of format_bits_per_pixel@1423F9E10's 0 bits: the texture does not \
                 hold the file's texels"
            )));
        })
    } else {
        return Err(Failure::Undefined(format!(
            "a {bits}-bit DDS with flags {flags:#x}: load_dds@142410030 loads it as format 0x1A, \
             which create_texture@1423FB4F0 uploads as B8G8R8X8 with rows of \
             format_bits_per_pixel@1423F9E10's 0 bits: the texture does not hold the file's texels"
        )));
    };
    if word(112) & CUBE_MAP != 0 {
        return Err(Failure::Undefined(
            "a cube-map DDS: load_dds@142410030 reads six faces without checking the file's \
             length, into a cube texture a GUI sprite samples through a two-dimensional binding"
                .to_owned(),
        ));
    }
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
    let (mut level_width, mut level_height) = (width, height);
    let mut offset = start;
    let mut levels = Vec::new();
    loop {
        let (w, h) = (level_width as usize, level_height as usize);
        let size = if block {
            w.div_ceil(4) * h.div_ceil(4) * unit
        } else {
            w * h * unit
        };
        let Some(source) = bytes.get(offset..offset + size) else {
            break;
        };
        let data = match &form {
            Form::Blocks(..) => source.to_vec(),
            Form::Pixels(pixels) => pixels.convert(source),
        };
        levels.push(MipLevel {
            width: level_width,
            height: level_height,
            data,
        });
        offset += size;
        if level_width == 1 || level_height == 1 || levels.len() as u64 >= u64::from(mip_count) {
            break;
        }
        // Two sides of 0 never reach 1: the game appends empty levels up to the header's count.
        // Past the second, none changes the outcome (the texture is refused unpadded, and its
        // second padded level is already empty), so the port stops there.
        if level_width == 0 && level_height == 0 && levels.len() == 2 {
            break;
        }
        level_width >>= 1;
        level_height >>= 1;
    }
    if levels.is_empty() {
        return Err(Failure::Undefined(
            "a DDS whose first level runs past the file: load_dds@142410030 keeps no level, and \
             create_texture@1423FB4F0 then creates the texture without initial data"
                .to_owned(),
        ));
    }
    let (format, zero) = match form {
        Form::Blocks(format, _) => (format, Zero::Transparent),
        Form::Pixels(pixels) => (pixels.format(), pixels.zero()),
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn header(
        width: u32,
        height: u32,
        mips: u32,
        flags: u32,
        fourcc: &[u8; 4],
        bits: u32,
        masks: [u32; 4],
    ) -> Vec<u8> {
        let mut bytes = vec![0_u8; HEADER];
        bytes[..4].copy_from_slice(b"DDS ");
        let mut put = |offset: usize, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes())
        };
        put(4, 124);
        put(12, height);
        put(16, width);
        put(28, mips);
        put(76, 32);
        put(80, flags);
        put(84, u32::from_le_bytes(*fourcc));
        put(88, bits);
        for (index, mask) in masks.into_iter().enumerate() {
            put(92 + index * 4, mask);
        }
        bytes
    }

    /// A 5x3 `DXT5` texture is a 8x4 texture: its extent rounds to whole blocks, and its chain
    /// halves the rounded sides.
    #[test]
    fn block_extents_round_up_to_whole_blocks() {
        let mut file = header(5, 3, 9, 4, b"DXT5", 0, [0; 4]);
        file.extend(std::iter::repeat_n(0xAB, 16 * 2 + 16 + 16));
        let texture = load(&file).unwrap().unwrap().0;
        assert_eq!((texture.width, texture.height), (8, 4));
        let sides: Vec<_> = texture.mips[0]
            .iter()
            .map(|mip| (mip.width, mip.height))
            .collect();
        assert_eq!(sides, [(8, 4), (4, 2), (2, 1)]);
        // `DXT2` fails the load; the chain goes on.
        assert_eq!(load(&header(4, 4, 1, 4, b"DXT2", 0, [0; 4])).unwrap(), None);
    }

    /// A 32-bit surface is `B8G8R8A8` whatever its masks and alpha flag say; a missing level
    /// ends the chain with the levels before it.
    #[test]
    fn a_32_bit_surface_is_bgra_whatever_its_masks() {
        let mut file = header(2, 2, 5, 0x40, b"\0\0\0\0", 32, [0xFF, 0xFF00, 0xFF_0000, 0]);
        file.extend([
            1, 2, 3, 0, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 9, 9, 9,
        ]);
        let texture = load(&file).unwrap().unwrap().0;
        assert_eq!(texture.mips[0].len(), 1);
        assert_eq!(
            texture.base().data,
            [3, 2, 1, 0, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15]
        );
    }

    /// A 16-bit surface keeps its words: `X1R5G5B5` is promoted to opaque `A1R5G5B5`,
    /// `A1R5G5B5` keeps its top bit as alpha, and a 5-bit channel of 1 samples as `1 / 31`, not
    /// as the 8-bit `8 / 255` (`format_to_dxgi@142411320`, `build_upload_rows@1423FC160`).
    #[test]
    fn sixteen_bit_forms() {
        let texels = [0x7FFF_u16, 0x001F].map(u16::to_le_bytes).concat();
        let mut x555 = header(2, 1, 1, 0x40, b"\0\0\0\0", 16, [0x7C00, 0x3E0, 0x1F, 0]);
        x555.extend(&texels);
        let x555 = load(&x555).unwrap().unwrap().0;
        assert_eq!(x555.format, TextureFormat::B5G5R5A1);
        assert_eq!(x555.base().data, [0xFF, 0xFF, 0x1F, 0x80]);
        let mut a1555 = header(
            2,
            1,
            1,
            0x41,
            b"\0\0\0\0",
            16,
            [0x7C00, 0x3E0, 0x1F, 0x8000],
        );
        a1555.extend([0x00_u8, 0x80, 0x10, 0x02]);
        let a1555 = load(&a1555).unwrap().unwrap().0;
        assert_eq!(a1555.format, TextureFormat::B5G5R5A1);
        assert_eq!(a1555.base().data, [0x00, 0x80, 0x10, 0x02]);
        let word = |texture: &Texture, index: usize| {
            let data = &texture.base().data;
            u16::from_le_bytes([data[2 * index], data[2 * index + 1]])
        };
        assert_eq!(
            a1555.format.sample_16(word(&a1555, 1)),
            Some([0.0, 16.0 / 31.0, 16.0 / 31.0, 0.0])
        );
        // Red, green and blue 1 of a 5-6-5 surface: `c / max`, alpha 1.
        let mut r565 = header(1, 1, 1, 0x40, b"\0\0\0\0", 16, [0xF800, 0x7E0, 0x1F, 0]);
        r565.extend(0x0821_u16.to_le_bytes());
        let r565 = load(&r565).unwrap().unwrap().0;
        assert_eq!(r565.format, TextureFormat::B5G6R5);
        let sampled = r565.format.sample_16(word(&r565, 0)).unwrap();
        assert_eq!(sampled, [1.0 / 31.0, 1.0 / 63.0, 1.0 / 31.0, 1.0]);
        assert_ne!(sampled[0], 8.0 / 255.0);
        let mut other = header(1, 1, 1, 0x41, b"\0\0\0\0", 16, [0xF00, 0xF0, 0xF, 0xF000]);
        other.extend([0, 0]);
        assert!(matches!(load(&other), Err(Failure::Undefined(_))));
    }
}
