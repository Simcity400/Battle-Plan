//! DirectDraw Surface decoding for the map, interface and font textures.
//!
//! Block-compressed surfaces keep their authored blocks; uncompressed surfaces
//! become RGBA8. The `pitch` / `linearSize` header field is ignored because the
//! shipped files disagree with it, so every offset is derived from the format.

use std::io;

use super::{Rgba8Image, invalid, reserved};

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_SIZE: u32 = 124;
const PIXEL_FORMAT_SIZE: u32 = 32;

const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;
const DDPF_ALPHAPIXELS: u32 = 0x1;

const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_CUBEMAP_ALL_FACES: u32 = 0xfc00;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;

const DDS_DIMENSION_TEXTURE2D: u32 = 3;
const DDS_RESOURCE_MISC_TEXTURECUBE: u32 = 0x4;

/// Cube faces in DDS storage order: +X, -X, +Y, -Y, +Z, -Z.
pub const CUBE_FACE_NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];

/// What the decoded bytes of every mip level hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    /// DXT1 blocks, 8 bytes per 4x4 block, kept as authored.
    Bc1,
    /// DXT3 blocks, 16 bytes per 4x4 block, kept as authored.
    Bc2,
    /// DXT5 blocks, 16 bytes per 4x4 block, kept as authored.
    Bc3,
    /// Straight 8-bit R, G, B, A, row-major from the top-left corner.
    Rgba8,
}

impl TextureFormat {
    /// `None` for the uncompressed format, which is not block based.
    pub fn block_bytes(self) -> Option<usize> {
        match self {
            Self::Bc1 => Some(8),
            Self::Bc2 | Self::Bc3 => Some(16),
            Self::Rgba8 => None,
        }
    }

    /// Bytes one mip level of this size occupies, blocks included.
    pub fn level_bytes(self, width: u32, height: u32) -> io::Result<usize> {
        let (width, height) = (width as usize, height as usize);
        let bytes = match self.block_bytes() {
            Some(block) => width.div_ceil(4) * height.div_ceil(4) * block,
            None => width * height * 4,
        };
        if bytes == 0 {
            return Err(invalid("DDS mip level has no pixels"));
        }
        Ok(bytes)
    }
}

/// One mip level of one face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    /// Block bytes for a compressed format, otherwise RGBA8 pixels.
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
    /// Indexed `[face][mip]`; faces follow [`CUBE_FACE_NAMES`].
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

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated DDS header"))?
            .try_into()
            .unwrap(),
    ))
}

/// One channel of an uncompressed surface, as a contiguous run of mask bits.
#[derive(Clone, Copy, Debug)]
struct Channel {
    shift: u32,
    max: u32,
}

impl Channel {
    fn new(mask: u32, bits: u32) -> io::Result<Option<Self>> {
        if mask == 0 {
            return Ok(None);
        }
        let shift = mask.trailing_zeros();
        let width = mask.count_ones();
        if mask >> shift != (1_u64 << width) as u32 - 1 {
            return Err(invalid(format!(
                "DDS channel mask {mask:#x} is not a contiguous run of bits"
            )));
        }
        if shift + width > bits {
            return Err(invalid(format!(
                "DDS channel mask {mask:#x} lies outside its {bits}-bit pixel"
            )));
        }
        Ok(Some(Self {
            shift,
            max: (1_u64 << width) as u32 - 1,
        }))
    }

    fn sample(self, pixel: u32) -> u8 {
        let value = (pixel >> self.shift) & self.max;
        // Exact rounding of value/max into 0..=255; identical to bit
        // replication for the 1, 4, 5, 6 and 8 bit widths the game ships.
        ((u64::from(value) * 255 + u64::from(self.max) / 2) / u64::from(self.max)) as u8
    }
}

/// How to read one uncompressed pixel, described by its masks.
#[derive(Clone, Copy, Debug)]
struct Layout {
    bytes: usize,
    red: Option<Channel>,
    green: Option<Channel>,
    blue: Option<Channel>,
    alpha: Option<Channel>,
}

impl Layout {
    fn convert(&self, source: &[u8], out: &mut Vec<u8>) {
        for pixel in source.chunks_exact(self.bytes) {
            let mut value = 0_u32;
            for (index, byte) in pixel.iter().enumerate() {
                value |= u32::from(*byte) << (8 * index);
            }
            out.push(self.red.map_or(0, |channel| channel.sample(value)));
            out.push(self.green.map_or(0, |channel| channel.sample(value)));
            out.push(self.blue.map_or(0, |channel| channel.sample(value)));
            out.push(self.alpha.map_or(255, |channel| channel.sample(value)));
        }
    }
}

fn masked_layout(bits: u32, masks: [u32; 4]) -> io::Result<Layout> {
    if !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(invalid(format!(
            "unsupported uncompressed DDS pixel size of {bits} bits"
        )));
    }
    Ok(Layout {
        bytes: bits as usize / 8,
        red: Channel::new(masks[0], bits)?,
        green: Channel::new(masks[1], bits)?,
        blue: Channel::new(masks[2], bits)?,
        alpha: Channel::new(masks[3], bits)?,
    })
}

enum Source {
    Blocks(TextureFormat),
    Pixels(Layout),
}

fn fourcc_name(fourcc: u32) -> String {
    let bytes = fourcc
        .to_le_bytes()
        .map(|byte| if byte.is_ascii_graphic() { byte } else { b'?' });
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Maps the legacy pixel-format block onto a decodable source.
fn legacy_source(flags: u32, fourcc: u32, bits: u32, masks: [u32; 4]) -> io::Result<Source> {
    if flags & DDPF_FOURCC != 0 {
        return match &fourcc.to_le_bytes() {
            b"DXT1" => Ok(Source::Blocks(TextureFormat::Bc1)),
            b"DXT3" => Ok(Source::Blocks(TextureFormat::Bc2)),
            b"DXT5" => Ok(Source::Blocks(TextureFormat::Bc3)),
            // DXT2 and DXT4 premultiply alpha, which would change the
            // composite; the game ships neither, so they are refused.
            _ => Err(invalid(format!(
                "unsupported DDS four-character code {:?}",
                fourcc_name(fourcc)
            ))),
        };
    }
    let alpha = if flags & DDPF_ALPHAPIXELS != 0 {
        masks[3]
    } else {
        0
    };
    if flags & DDPF_RGB != 0 {
        return Ok(Source::Pixels(masked_layout(
            bits,
            [masks[0], masks[1], masks[2], alpha],
        )?));
    }
    if flags & DDPF_LUMINANCE != 0 {
        let layout = masked_layout(bits, [masks[0], masks[0], masks[0], alpha])?;
        return Ok(Source::Pixels(layout));
    }
    if flags & DDPF_ALPHA != 0 {
        // Alpha-only surfaces sample as (0, 0, 0, a), as Direct3D defines A8.
        return Ok(Source::Pixels(masked_layout(bits, [0, 0, 0, masks[3]])?));
    }
    Err(invalid(format!(
        "unsupported DDS pixel-format flags {flags:#x}"
    )))
}

/// Maps a DX10 `dxgiFormat` onto a decodable source.
///
/// Each `_SRGB` twin decodes to the same bytes as its `_UNORM` source, and the
/// map renderer types every one of them linear (`viewer`'s `map::textures`).
/// That is the engine's own behaviour rather than a simplification:
/// four-character code with `DX10` only to step the payload pointer from
/// `header + 128` to `header + 148`, reads `dxgiFormat` nowhere, and falls
/// through to its four-bytes-per-pixel branch — `pfFlags & 0x20240` is zero for
/// a `DDPF_FOURCC` header — where it stores the enum `0xd`. So a DX10 file
/// reaches the GPU as `B8G8R8A8_UNORM` whatever it names. Nine of the 31,791
/// `.dds` the game ships carry a DX10 header and all nine name an sRGB
/// format — six
/// `B8G8R8A8_UNORM_SRGB` and three `R8G8B8A8_UNORM_SRGB`, every one under
/// `gfx/interface` or `gfx/event_pictures` and none under `map/`.
///
/// The three `R8G8B8A8_UNORM_SRGB` files are the one place this decoder and the
/// engine differ: the masks below read them in the channel order their DXGI
/// name gives, where the engine's fall-through reads every DX10 file as BGRA and
/// so draws those three with red and blue swapped. They are
/// `gfx/interface/main_lobby_button_transparent.dds`,
/// `main_lobby_button_white.dds` and `small_flag_frame_thin3_gold.dds`; no pass
/// in this port loads any of them, so the divergence is recorded rather than
/// reproduced.
fn dx10_source(format: u32) -> io::Result<Source> {
    match format {
        71 | 72 => Ok(Source::Blocks(TextureFormat::Bc1)),
        74 | 75 => Ok(Source::Blocks(TextureFormat::Bc2)),
        77 | 78 => Ok(Source::Blocks(TextureFormat::Bc3)),
        28 | 29 => Ok(Source::Pixels(masked_layout(
            32,
            [0xff, 0xff00, 0xff_0000, 0xff00_0000],
        )?)),
        87 | 91 => Ok(Source::Pixels(masked_layout(
            32,
            [0xff_0000, 0xff00, 0xff, 0xff00_0000],
        )?)),
        _ => Err(invalid(format!("unsupported DX10 DDS dxgiFormat {format}"))),
    }
}

/// Decodes a DDS file into its mip chain, one entry per face.
///
/// Refuses volume textures, partial cube maps and every format outside
/// BC1/BC2/BC3 and the mask-described 8, 16, 24 and 32-bit layouts.
pub fn decode(input: &[u8]) -> io::Result<Texture> {
    if input.get(..4) != Some(MAGIC) {
        return Err(invalid("file does not start with the DDS magic"));
    }
    if u32_at(input, 4)? != HEADER_SIZE {
        return Err(invalid("DDS header size is not 124"));
    }
    let height = u32_at(input, 12)?;
    let width = u32_at(input, 16)?;
    let depth = u32_at(input, 24)?;
    let declared_mips = u32_at(input, 28)?;
    if u32_at(input, 76)? != PIXEL_FORMAT_SIZE {
        return Err(invalid("DDS pixel-format size is not 32"));
    }
    let pixel_flags = u32_at(input, 80)?;
    let fourcc = u32_at(input, 84)?;
    let bits = u32_at(input, 88)?;
    let masks = [
        u32_at(input, 92)?,
        u32_at(input, 96)?,
        u32_at(input, 100)?,
        u32_at(input, 104)?,
    ];
    let caps2 = u32_at(input, 112)?;
    if width == 0 || height == 0 {
        return Err(invalid("DDS dimensions must be nonzero"));
    }
    if caps2 & DDSCAPS2_VOLUME != 0 || depth > 1 {
        return Err(invalid("volume DDS textures are not supported"));
    }

    let dx10 = pixel_flags & DDPF_FOURCC != 0 && &fourcc.to_le_bytes() == b"DX10";
    let mut faces = if caps2 & DDSCAPS2_CUBEMAP != 0 {
        if caps2 & DDSCAPS2_CUBEMAP_ALL_FACES != DDSCAPS2_CUBEMAP_ALL_FACES {
            return Err(invalid("partial cube-map DDS textures are not supported"));
        }
        6
    } else {
        1
    };
    let (source, data_start) = if dx10 {
        let dxgi = u32_at(input, 128)?;
        let dimension = u32_at(input, 132)?;
        let misc = u32_at(input, 136)?;
        let array = u32_at(input, 140)?;
        if dimension != DDS_DIMENSION_TEXTURE2D {
            return Err(invalid(format!(
                "DX10 DDS resource dimension {dimension} is not a 2D texture"
            )));
        }
        if misc & DDS_RESOURCE_MISC_TEXTURECUBE != 0 {
            faces = 6;
        }
        if array > 1 {
            return Err(invalid("DDS texture arrays are not supported"));
        }
        (dx10_source(dxgi)?, 148)
    } else {
        (legacy_source(pixel_flags, fourcc, bits, masks)?, 128_usize)
    };

    let mips = declared_mips.max(1);
    let format = match source {
        Source::Blocks(format) => format,
        Source::Pixels(_) => TextureFormat::Rgba8,
    };
    let mut sizes = Vec::new();
    for level in 0..mips {
        let level_width = (width >> level).max(1);
        let level_height = (height >> level).max(1);
        sizes.push((level_width, level_height));
        if level_width == 1 && level_height == 1 && level + 1 < mips {
            return Err(invalid(format!(
                "DDS declares {mips} mip levels but {width}x{height} has only {}",
                level + 1
            )));
        }
    }

    let mut offset = data_start;
    let mut all_faces = Vec::new();
    for _ in 0..faces {
        let mut levels = Vec::new();
        for &(level_width, level_height) in &sizes {
            let (stored, decoded) = match &source {
                Source::Blocks(format) => {
                    let bytes = format.level_bytes(level_width, level_height)?;
                    (bytes, bytes)
                }
                Source::Pixels(layout) => (
                    (level_width as usize)
                        .checked_mul(level_height as usize)
                        .and_then(|pixels| pixels.checked_mul(layout.bytes))
                        .ok_or_else(|| invalid("DDS mip level size overflows"))?,
                    TextureFormat::Rgba8.level_bytes(level_width, level_height)?,
                ),
            };
            let end = offset
                .checked_add(stored)
                .ok_or_else(|| invalid("DDS mip level offset overflows"))?;
            let chunk = input
                .get(offset..end)
                .ok_or_else(|| invalid("truncated DDS pixel data"))?;
            let mut data = reserved(decoded, "DDS mip level")?;
            match &source {
                Source::Blocks(_) => data.extend_from_slice(chunk),
                Source::Pixels(layout) => layout.convert(chunk, &mut data),
            }
            levels.push(MipLevel {
                width: level_width,
                height: level_height,
                data,
            });
            offset = end;
        }
        all_faces.push(levels);
    }
    Ok(Texture {
        format,
        width,
        height,
        faces,
        mips: all_faces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Header {
        width: u32,
        height: u32,
        mips: u32,
        pixel_flags: u32,
        fourcc: &'static [u8; 4],
        bits: u32,
        masks: [u32; 4],
        caps2: u32,
        dx10: Option<[u32; 5]>,
    }

    impl Default for Header {
        fn default() -> Self {
            Self {
                width: 4,
                height: 4,
                mips: 1,
                pixel_flags: DDPF_FOURCC,
                fourcc: b"DXT1",
                bits: 0,
                masks: [0; 4],
                caps2: 0,
                dx10: None,
            }
        }
    }

    fn file(header: &Header, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 128];
        bytes[..4].copy_from_slice(MAGIC);
        let put = |bytes: &mut Vec<u8>, offset: usize, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut bytes, 4, HEADER_SIZE);
        put(&mut bytes, 12, header.height);
        put(&mut bytes, 16, header.width);
        put(&mut bytes, 28, header.mips);
        put(&mut bytes, 76, PIXEL_FORMAT_SIZE);
        put(&mut bytes, 80, header.pixel_flags);
        put(&mut bytes, 84, u32::from_le_bytes(*header.fourcc));
        put(&mut bytes, 88, header.bits);
        for (index, mask) in header.masks.iter().enumerate() {
            put(&mut bytes, 92 + index * 4, *mask);
        }
        put(&mut bytes, 112, header.caps2);
        if let Some(extension) = header.dx10 {
            for value in extension {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend_from_slice(payload);
        bytes
    }

    fn bgra_header(width: u32, height: u32, mips: u32) -> Header {
        Header {
            width,
            height,
            mips,
            pixel_flags: DDPF_RGB | DDPF_ALPHAPIXELS,
            fourcc: b"\0\0\0\0",
            bits: 32,
            masks: [0xff_0000, 0xff00, 0xff, 0xff00_0000],
            ..Header::default()
        }
    }

    #[test]
    fn bgra32_becomes_rgba8_in_storage_order() {
        let payload = [1_u8, 2, 3, 4, 5, 6, 7, 8];
        let texture = decode(&file(&bgra_header(2, 1, 1), &payload)).unwrap();
        assert_eq!(texture.format, TextureFormat::Rgba8);
        assert_eq!((texture.width, texture.height, texture.faces), (2, 1, 1));
        assert_eq!(texture.base().data, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn block_mip_chains_use_one_block_below_four_pixels() {
        // 4x4 then 2x2 then 1x1: every level is exactly one DXT5 block.
        let payload: Vec<u8> = (0..48).collect();
        let header = Header {
            width: 4,
            height: 4,
            mips: 3,
            fourcc: b"DXT5",
            ..Header::default()
        };
        let texture = decode(&file(&header, &payload)).unwrap();
        assert_eq!(texture.format, TextureFormat::Bc3);
        assert_eq!(texture.mip_count(), 3);
        let levels = &texture.mips[0];
        assert_eq!(
            levels
                .iter()
                .map(|level| (level.width, level.height, level.data.len()))
                .collect::<Vec<_>>(),
            vec![(4, 4, 16), (2, 2, 16), (1, 1, 16)]
        );
        assert_eq!(levels[2].data, (32..48).collect::<Vec<u8>>());
        // The same chain in DXT1 halves every level.
        let header = Header {
            fourcc: b"DXT1",
            ..header
        };
        let texture = decode(&file(&header, &payload[..24])).unwrap();
        assert_eq!(texture.format, TextureFormat::Bc1);
        assert!(texture.mips[0].iter().all(|level| level.data.len() == 8));
        // DXT3 shares BC2's 16-byte blocks.
        let header = Header {
            fourcc: b"DXT3",
            ..header
        };
        assert_eq!(
            decode(&file(&header, &payload)).unwrap().format,
            TextureFormat::Bc2
        );
    }

    #[test]
    fn non_square_blocks_round_each_axis_up_separately() {
        // 6x2 rounds to 2x1 blocks; the 3x1 mip rounds to 1x1.
        let payload: Vec<u8> = (0..24).collect();
        let header = Header {
            width: 6,
            height: 2,
            mips: 2,
            fourcc: b"DXT1",
            ..Header::default()
        };
        let texture = decode(&file(&header, &payload)).unwrap();
        assert_eq!(
            texture.mips[0]
                .iter()
                .map(|level| (level.width, level.height, level.data.len()))
                .collect::<Vec<_>>(),
            vec![(6, 2, 16), (3, 1, 8)]
        );
        assert!(decode(&file(&header, &payload[..23])).is_err());
    }

    #[test]
    fn cube_faces_keep_dds_order_and_partial_cubes_are_refused() {
        let mut payload = Vec::new();
        for face in 0..6_u8 {
            payload.extend_from_slice(&[face, face, face, 255]);
        }
        let header = Header {
            caps2: DDSCAPS2_CUBEMAP | DDSCAPS2_CUBEMAP_ALL_FACES,
            ..bgra_header(1, 1, 1)
        };
        let texture = decode(&file(&header, &payload)).unwrap();
        assert_eq!(texture.faces, 6);
        assert_eq!(CUBE_FACE_NAMES[0], "+X");
        assert_eq!(CUBE_FACE_NAMES[5], "-Z");
        for (index, face) in texture.mips.iter().enumerate() {
            assert_eq!(
                face[0].data,
                vec![index as u8; 3]
                    .into_iter()
                    .chain([255])
                    .collect::<Vec<_>>()
            );
        }
        let partial = Header {
            caps2: DDSCAPS2_CUBEMAP | 0x400,
            ..bgra_header(1, 1, 1)
        };
        assert!(
            decode(&file(&partial, &payload))
                .unwrap_err()
                .to_string()
                .contains("partial cube-map")
        );
    }

    #[test]
    fn sixteen_and_eight_bit_layouts_expand_to_full_range() {
        // 5-6-5 with no alpha.
        let header = Header {
            width: 2,
            height: 1,
            pixel_flags: DDPF_RGB,
            fourcc: b"\0\0\0\0",
            bits: 16,
            masks: [0xf800, 0x7e0, 0x1f, 0],
            ..Header::default()
        };
        let payload = 0xffff_u16
            .to_le_bytes()
            .into_iter()
            .chain(0_u16.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            decode(&file(&header, &payload)).unwrap().base().data,
            vec![255, 255, 255, 255, 0, 0, 0, 255]
        );
        // 1-5-5-5 keeps the single alpha bit.
        let header = Header {
            pixel_flags: DDPF_RGB | DDPF_ALPHAPIXELS,
            masks: [0x7c00, 0x3e0, 0x1f, 0x8000],
            ..header
        };
        assert_eq!(
            decode(&file(&header, &payload)).unwrap().base().data,
            vec![255, 255, 255, 255, 0, 0, 0, 0]
        );
        // 4-4-4-4 halves to exactly 0x88 at the midpoint.
        let header = Header {
            masks: [0xf00, 0xf0, 0xf, 0xf000],
            ..header
        };
        let payload = 0x8888_u16
            .to_le_bytes()
            .into_iter()
            .chain(0_u16.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(
            decode(&file(&header, &payload)).unwrap().base().data[..4],
            [0x88, 0x88, 0x88, 0x88]
        );
        // 8-bit luminance replicates into RGB with opaque alpha.
        let header = Header {
            pixel_flags: DDPF_LUMINANCE,
            bits: 8,
            masks: [0xff, 0, 0, 0],
            ..header
        };
        assert_eq!(
            decode(&file(&header, &[7, 9])).unwrap().base().data,
            vec![7, 7, 7, 255, 9, 9, 9, 255]
        );
        // 8-bit alpha samples as (0, 0, 0, a).
        let header = Header {
            pixel_flags: DDPF_ALPHA,
            masks: [0, 0, 0, 0xff],
            ..header
        };
        assert_eq!(
            decode(&file(&header, &[7, 9])).unwrap().base().data,
            vec![0, 0, 0, 7, 0, 0, 0, 9]
        );
        // 24-bit BGR is opaque.
        let header = Header {
            width: 1,
            pixel_flags: DDPF_RGB,
            bits: 24,
            masks: [0xff_0000, 0xff00, 0xff, 0],
            ..header
        };
        assert_eq!(
            decode(&file(&header, &[1, 2, 3])).unwrap().base().data,
            vec![3, 2, 1, 255]
        );
    }

    #[test]
    fn dx10_covers_block_and_rgba8_formats_and_names_the_rest() {
        let dx10 = |dxgi: u32, misc: u32| Header {
            width: 1,
            height: 1,
            fourcc: b"DX10",
            dx10: Some([dxgi, DDS_DIMENSION_TEXTURE2D, misc, 1, 0]),
            ..Header::default()
        };
        assert_eq!(
            decode(&file(&dx10(77, 0), &[0; 16])).unwrap().format,
            TextureFormat::Bc3
        );
        assert_eq!(
            decode(&file(&dx10(28, 0), &[1, 2, 3, 4]))
                .unwrap()
                .base()
                .data,
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            decode(&file(&dx10(91, 0), &[1, 2, 3, 4]))
                .unwrap()
                .base()
                .data,
            vec![3, 2, 1, 4]
        );
        assert_eq!(
            decode(&file(&dx10(87, DDS_RESOURCE_MISC_TEXTURECUBE), &[0; 24]))
                .unwrap()
                .faces,
            6
        );
        assert!(
            decode(&file(&dx10(95, 0), &[0; 16]))
                .unwrap_err()
                .to_string()
                .contains("dxgiFormat 95")
        );
        let volume = Header {
            dx10: Some([28, 4, 0, 1, 0]),
            ..dx10(28, 0)
        };
        assert!(
            decode(&file(&volume, &[0; 4]))
                .unwrap_err()
                .to_string()
                .contains("resource dimension 4")
        );
    }

    #[test]
    fn unsupported_and_malformed_headers_are_named() {
        let bad_fourcc = Header {
            fourcc: b"RXGB",
            ..Header::default()
        };
        assert!(
            decode(&file(&bad_fourcc, &[0; 8]))
                .unwrap_err()
                .to_string()
                .contains("\"RXGB\"")
        );
        let odd_bits = Header {
            pixel_flags: DDPF_RGB,
            fourcc: b"\0\0\0\0",
            bits: 12,
            masks: [0xf00, 0xf0, 0xf, 0],
            ..Header::default()
        };
        assert!(
            decode(&file(&odd_bits, &[0; 64]))
                .unwrap_err()
                .to_string()
                .contains("12 bits")
        );
        let split_mask = Header {
            masks: [0xff_00ff, 0xff00, 0xff, 0],
            bits: 32,
            ..odd_bits
        };
        assert!(
            decode(&file(&split_mask, &[0; 64]))
                .unwrap_err()
                .to_string()
                .contains("contiguous")
        );
        let wide_mask = Header {
            masks: [0xff_0000, 0xff00, 0xff, 0],
            bits: 16,
            ..odd_bits
        };
        assert!(
            decode(&file(&wide_mask, &[0; 64]))
                .unwrap_err()
                .to_string()
                .contains("outside its 16-bit")
        );
        let too_many_mips = Header {
            width: 2,
            height: 2,
            mips: 5,
            ..bgra_header(2, 2, 5)
        };
        assert!(
            decode(&file(&too_many_mips, &[0; 256]))
                .unwrap_err()
                .to_string()
                .contains("only 2")
        );
        let zero = Header {
            width: 0,
            ..bgra_header(0, 4, 1)
        };
        assert!(decode(&file(&zero, &[0; 16])).is_err());
        let good = file(&bgra_header(2, 2, 1), &[0; 16]);
        for length in 0..good.len() {
            assert!(decode(&good[..length]).is_err());
        }
        assert!(decode(b"NOPE").is_err());
        let mut wrong_size = good.clone();
        wrong_size[4..8].copy_from_slice(&100_u32.to_le_bytes());
        assert!(decode(&wrong_size).is_err());
        let mut wrong_pf = good;
        wrong_pf[76..80].copy_from_slice(&24_u32.to_le_bytes());
        assert!(decode(&wrong_pf).is_err());
    }

    #[test]
    fn trailing_bytes_after_the_declared_surfaces_are_ignored() {
        let mut bytes = file(&bgra_header(1, 1, 1), &[1, 2, 3, 4]);
        bytes.extend_from_slice(b"tool footer");
        assert_eq!(decode(&bytes).unwrap().base().data, vec![3, 2, 1, 4]);
    }
}
