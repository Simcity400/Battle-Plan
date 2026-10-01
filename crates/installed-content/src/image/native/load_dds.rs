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
//! A cube map (`DDSCAPS2_CUBEMAP`) takes the six-face branch: every face, in DDS order, holds
//! that chain in turn, and the loader reads them without checking the file's length.
//!
//! DX10 BC7 alone takes the owner's exception below: its logical DDS extent and complete
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
    // OWNER EXCEPTION (2026-10-01, BC7 only): load_dds@142410030 never reads dxgiFormat,
    // reads BC7 as BGRA and keeps no levels; create_texture@1423FB4F0 fails. Decode BC7
    // properly instead, with DDS logical mips and the header's colour space. Other DX10
    // files and the native BC1-3 chain below are unchanged.
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
        // The six-face branch (`param_1[4] = 1`): each face in DDS order holds its whole chain,
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
                         load_dds@142410030 reads the six faces without checking the file's \
                         length, so create_texture@1423FB4F0 uploads memory past it",
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

    /// A cube map takes the six-face branch: each face, in order, holds its whole chain, halved
    /// from the block-rounded extent; a file too short for the six is not what the game reads
    /// past its end.
    #[test]
    fn a_cube_map_holds_six_chains_in_face_order() {
        // 6x5 DXT1 rounds to 8x8; two levels of 8x8 (4 blocks) and 4x4 (1 block) per face.
        let mut file = header(6, 5, 2, 4, b"DXT1", 0, [0; 4]);
        file[112..116].copy_from_slice(&0xFE00_u32.to_le_bytes());
        for face in 0..6_u8 {
            file.extend(std::iter::repeat_n(face * 2, 4 * 8));
            file.extend(std::iter::repeat_n(face * 2 + 1, 8));
        }
        let texture = load(&file).unwrap().unwrap().0;
        assert_eq!((texture.faces, texture.width, texture.height), (6, 8, 8));
        for (face, levels) in texture.mips.iter().enumerate() {
            let sides: Vec<_> = levels.iter().map(|l| (l.width, l.height)).collect();
            assert_eq!(sides, [(8, 8), (4, 4)]);
            assert!(levels[0].data.iter().all(|&b| b == face as u8 * 2));
            assert!(levels[1].data.iter().all(|&b| b == face as u8 * 2 + 1));
        }
        file.pop();
        assert!(matches!(load(&file), Err(Failure::Undefined(why)) if why.contains("face 5")));
    }

    fn dx10(width: u32, height: u32, mips: u32, dxgi: u32) -> Vec<u8> {
        let mut file = header(width, height, mips, 4, b"DX10", 0, [0; 4]);
        file[8..12].copy_from_slice(&0x2_0000_u32.to_le_bytes());
        for word in [dxgi, 3, 0, 1, 0] {
            file.extend(word.to_le_bytes());
        }
        file
    }

    #[test]
    fn bc7_keeps_logical_mips_and_exact_payload_offsets() {
        for (dxgi, srgb) in [(98, false), (99, true)] {
            let mut file = dx10(10, 6, 4, dxgi);
            let expected = [
                (10, 6, 96, 148),
                (5, 3, 32, 244),
                (2, 1, 16, 276),
                (1, 1, 16, 292),
            ];
            for (level, &(_, _, size, offset)) in expected.iter().enumerate() {
                assert_eq!(file.len(), offset);
                file.extend(std::iter::repeat_n(level as u8 + 1, size));
            }
            let (texture, zero) = load(&file).unwrap().unwrap();
            assert_eq!((texture.width, texture.height, texture.faces), (10, 6, 1));
            assert_eq!(texture.format, TextureFormat::Bc7 { srgb });
            assert_eq!(zero, Zero::Transparent);
            assert_eq!(texture.mip_count(), 4);
            for (mip, &(width, height, size, offset)) in texture.mips[0].iter().zip(&expected) {
                assert_eq!(
                    (mip.width, mip.height, mip.data.len()),
                    (width, height, size)
                );
                assert_eq!(mip.data, file[offset..offset + size]);
            }
        }
    }

    /// The native padding step copies BC7's logical block rows, including sub-block mips,
    /// without reinterpreting them as the native BC1-3 chain.
    #[test]
    fn bc7_force_pow2_copies_logical_block_rows_at_the_padded_pitch() {
        use super::super::{Decode, decode_texture_image};

        for (dxgi, srgb) in [(98, false), (99, true)] {
            let mut file = dx10(10, 6, 4, dxgi);
            let data: Vec<Vec<u8>> = [96, 32, 16, 16]
                .into_iter()
                .enumerate()
                .map(|(level, size)| {
                    (0..size)
                        .map(|byte| (level * 32 + byte + 1) as u8)
                        .collect()
                })
                .collect();
            for bytes in &data {
                file.extend(bytes);
            }
            let padded = decode_texture_image(
                b"gfx/padded-bc7.dds",
                &file,
                Decode {
                    force_pow2_textures: true,
                },
            )
            .unwrap();
            assert_eq!((padded.width, padded.height, padded.faces), (16, 8, 1));
            assert_eq!(padded.format, TextureFormat::Bc7 { srgb });
            assert_eq!(padded.mip_count(), 4);
            // Logical and padded extents.
            let expected = [
                ((10, 6), (16, 8)),
                ((5, 3), (8, 4)),
                ((2, 1), (4, 2)),
                ((1, 1), (2, 1)),
            ];
            let unpadded =
                decode_texture_image(b"gfx/padded-bc7.dds", &file, Decode::default()).unwrap();
            for (index, &(logical_extent, padded_extent)) in expected.iter().enumerate() {
                let original = &unpadded.mips[0][index];
                assert_eq!((original.width, original.height), logical_extent);
                assert_eq!(original.data, data[index]);
                let mip = &padded.mips[0][index];
                assert_eq!((mip.width, mip.height), padded_extent);
                let source_pitch = original.width.div_ceil(4) as usize * 16;
                let padded_pitch = mip.width.div_ceil(4) as usize * 16;
                let rows = mip.height.div_ceil(4) as usize;
                assert_eq!(mip.data.len(), padded_pitch * rows);
                for (row, bytes) in mip.data.chunks_exact(padded_pitch).enumerate() {
                    assert_eq!(
                        bytes[..source_pitch],
                        data[index][row * source_pitch..(row + 1) * source_pitch],
                        "level {index}, row {row}"
                    );
                    assert!(bytes[source_pitch..].iter().all(|&byte| byte == 0));
                }
            }
        }
    }

    /// DDS clamps the logical short side to one; the native padding step does not clamp its
    /// shifted side, so it refuses before an empty padded buffer could reach the device.
    #[test]
    fn bc7_force_pow2_refuses_a_zero_padded_mip_side_by_file_name() {
        use super::super::{Decode, decode_texture_image};
        use crate::gui::native::resources::{NoTexture, texture};

        for dxgi in [98, 99] {
            let mut file = dx10(10, 2, 4, dxgi);
            for (level, size) in [48, 32, 16, 16].into_iter().enumerate() {
                file.extend(std::iter::repeat_n(level as u8 + 1, size));
            }
            let name = b"gfx/thin-bc7.dds";
            let unpadded = decode_texture_image(name, &file, Decode::default()).unwrap();
            let sides: Vec<_> = unpadded.mips[0]
                .iter()
                .map(|m| (m.width, m.height))
                .collect();
            assert_eq!(sides, [(10, 2), (5, 1), (2, 1), (1, 1)]);
            let padded = texture(
                name,
                &|path| {
                    assert_eq!(path, name);
                    Ok(file.clone())
                },
                Decode {
                    force_pow2_textures: true,
                },
            )
            .unwrap();
            assert!(matches!(padded, Err(NoTexture::Undefined { path, reason })
                if path == "gfx/thin-bc7.dds"
                    && reason.contains("pads a 10x2 image to 16x2")
                    && reason.contains("level 2 is 4x0")
                    && reason.contains("pad_power_of_two@1423F8FA0")
                    && reason.contains("create_texture@1423FB4F0")));
        }
    }

    #[test]
    fn bc7_uses_the_mip_count_only_when_the_header_declares_it() {
        for (flags, count, levels) in [(0, 4, 1), (0x2_0000, 0, 1), (0x2_0000, 2, 2)] {
            let mut file = dx10(10, 6, count, 98);
            file[8..12].copy_from_slice(&u32::to_le_bytes(flags));
            file.extend([1; 96]);
            file.extend([2; 32]);
            let texture = load(&file).unwrap().unwrap().0;
            assert_eq!(texture.mip_count(), levels);
        }
    }

    #[test]
    fn bc7_refuses_every_truncated_declared_level_by_name() {
        let mut file = dx10(10, 6, 4, 98);
        file.extend([0; 160]);
        for (length, level) in [(243, 0), (275, 1), (291, 2), (307, 3)] {
            let error = load(&file[..length]).unwrap_err();
            assert!(
                matches!(error, Failure::Undecodable(ref why)
                    if why.contains("DX10 BC7 DDS")
                        && why.contains("truncated declared mip chain")
                        && why.contains(&format!("level {level}"))),
                "{error:?}"
            );
        }
        assert!(matches!(load(&file[..132]), Err(Failure::Undecodable(why))
            if why.contains("BC7") && why.contains("truncated DX10 header")));
    }

    #[test]
    fn bc7_refuses_unreached_cube_array_and_volume_forms_by_name() {
        let file = dx10(10, 6, 4, 98);
        for (offset, value, reason) in [
            (112, CUBE_MAP, "cube maps"),
            (136, 4, "cube maps"),
            (140, 2, "array size 2"),
            (132, 4, "resource dimension 4"),
        ] {
            let mut invalid = file.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(matches!(load(&invalid), Err(Failure::Undecodable(why))
                if why.contains("BC7") && why.contains(reason)));
        }
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
