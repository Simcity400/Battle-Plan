use crate::{ErrorKind, GameUiError, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const DDS_HEADER_LEN: usize = 128;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdsImage {
    pub image: RgbaImage,
    pub format: String,
    pub mip_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: usize,
    pub height: usize,
    pub format: String,
    pub mip_count: u32,
    pub is_cubemap: bool,
    pub is_volume: bool,
}

pub fn decode_file(path: &Path) -> Result<DdsImage> {
    let data = fs::read(path)?;
    decode_bytes(&data).map_err(|err| add_path(err, path))
}

pub fn info_file(path: &Path) -> Result<DdsInfo> {
    let data = fs::read(path)?;
    info_bytes(&data).map_err(|err| add_path(err, path))
}

pub fn decode_bytes(data: &[u8]) -> Result<DdsImage> {
    let header = Header::parse(data)?;
    if header.caps2 & DDSCAPS2_CUBEMAP != 0 {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("cubemap".to_string()),
            "DDS cubemaps are not UI textures and are rejected",
        ));
    }
    if header.caps2 & DDSCAPS2_VOLUME != 0 {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("volume".to_string()),
            "DDS volume textures are not UI textures and are rejected",
        ));
    }

    let image = match header.format.as_str() {
        "DXT1" => decode_bc1(data, &header, DDS_HEADER_LEN, true)?,
        "DXT3" => decode_bc2(data, &header, DDS_HEADER_LEN)?,
        "DXT5" => decode_bc3(data, &header, DDS_HEADER_LEN)?,
        "A8R8G8B8" | "X8R8G8B8" | "R8G8B8" => decode_uncompressed(data, &header)?,
        other => {
            return Err(GameUiError::unsupported_dds(
                other.to_string(),
                format!("unsupported DDS format {other}"),
            ));
        }
    };

    Ok(DdsImage {
        image,
        format: header.format,
        mip_count: header.mip_count,
    })
}

pub fn info_bytes(data: &[u8]) -> Result<DdsInfo> {
    let header = Header::parse(data)?;
    Ok(DdsInfo {
        width: header.width,
        height: header.height,
        format: header.format,
        mip_count: header.mip_count,
        is_cubemap: header.caps2 & DDSCAPS2_CUBEMAP != 0,
        is_volume: header.caps2 & DDSCAPS2_VOLUME != 0,
    })
}

pub fn format_histogram(paths: &[impl AsRef<Path>]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for path in paths {
        let format = info_file(path.as_ref())
            .map(|info| info.format)
            .unwrap_or_else(|err| match err.kind {
                ErrorKind::UnsupportedDdsFormat(format) => format,
                _ => "UNREADABLE".to_string(),
            });
        *out.entry(format).or_insert(0) += 1;
    }
    out
}

#[derive(Clone, Debug)]
struct Header {
    width: usize,
    height: usize,
    mip_count: u32,
    caps2: u32,
    pf_flags: u32,
    rgb_bits: usize,
    rmask: u32,
    gmask: u32,
    bmask: u32,
    amask: u32,
    format: String,
}

impl Header {
    fn parse(data: &[u8]) -> Result<Self> {
        if data.get(0..4) != Some(b"DDS ") {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("magic".to_string()),
                "not a DDS file",
            ));
        }
        if data.len() < DDS_HEADER_LEN {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("truncated".to_string()),
                "DDS header is truncated",
            ));
        }
        let header_size = le_u32(data, 4)?;
        let pf_size = le_u32(data, 76)?;
        if header_size != 124 || pf_size != 32 {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("header".to_string()),
                format!("invalid DDS header sizes: header={header_size} pixel_format={pf_size}"),
            ));
        }
        let height = le_u32(data, 12)? as usize;
        let width = le_u32(data, 16)? as usize;
        let mip_count = le_u32(data, 28)?.max(1);
        let caps2 = le_u32(data, 112)?;
        let pf_flags = le_u32(data, 80)?;
        let fourcc: [u8; 4] = data[84..88].try_into().expect("slice length checked");
        let rgb_bits = le_u32(data, 88)? as usize;
        let rmask = le_u32(data, 92)?;
        let gmask = le_u32(data, 96)?;
        let bmask = le_u32(data, 100)?;
        let amask = le_u32(data, 104)?;
        let format = dds_format(pf_flags, fourcc, rgb_bits, rmask, gmask, bmask, amask);
        if width == 0 || height == 0 {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("dimensions".to_string()),
                format!("invalid DDS dimensions {width}x{height}"),
            ));
        }
        Ok(Self {
            width,
            height,
            mip_count,
            caps2,
            pf_flags,
            rgb_bits,
            rmask,
            gmask,
            bmask,
            amask,
            format,
        })
    }
}

fn dds_format(
    pf_flags: u32,
    fourcc: [u8; 4],
    rgb_bits: usize,
    rmask: u32,
    gmask: u32,
    bmask: u32,
    amask: u32,
) -> String {
    if pf_flags & DDPF_FOURCC != 0 && fourcc.iter().any(|byte| *byte != 0) {
        return String::from_utf8_lossy(&fourcc)
            .trim_end_matches('\0')
            .to_string();
    }
    if pf_flags & DDPF_RGB != 0 {
        if rgb_bits == 32
            && rmask == 0x00ff_0000
            && gmask == 0x0000_ff00
            && bmask == 0x0000_00ff
            && amask == 0xff00_0000
        {
            return "A8R8G8B8".to_string();
        }
        if rgb_bits == 32
            && rmask == 0x00ff_0000
            && gmask == 0x0000_ff00
            && bmask == 0x0000_00ff
            && amask == 0
        {
            return "X8R8G8B8".to_string();
        }
        if rgb_bits == 24
            && rmask == 0x00ff_0000
            && gmask == 0x0000_ff00
            && bmask == 0x0000_00ff
            && amask == 0
        {
            return "R8G8B8".to_string();
        }
        return format!("RGB{rgb_bits}_R{rmask:08x}_G{gmask:08x}_B{bmask:08x}_A{amask:08x}");
    }
    if rgb_bits > 0 {
        format!("RGB{rgb_bits}")
    } else {
        "UNKNOWN".to_string()
    }
}

fn decode_bc1(data: &[u8], header: &Header, offset: usize, transparent: bool) -> Result<RgbaImage> {
    let block_w = header.width.div_ceil(4);
    let block_h = header.height.div_ceil(4);
    let needed = offset + block_w * block_h * 8;
    require_len(data, needed, "DXT1")?;
    let mut pixels = vec![0u8; header.width * header.height * 4];
    for by in 0..block_h {
        for bx in 0..block_w {
            let block = offset + (by * block_w + bx) * 8;
            let color0 = u16::from_le_bytes([data[block], data[block + 1]]);
            let color1 = u16::from_le_bytes([data[block + 2], data[block + 3]]);
            let codes = u32::from_le_bytes([
                data[block + 4],
                data[block + 5],
                data[block + 6],
                data[block + 7],
            ]);
            let palette = if transparent {
                bc1_color_palette(color0, color1)
            } else {
                bc_color_palette(color0, color1)
            };
            let alpha = if transparent {
                bc1_alpha_palette(color0, color1)
            } else {
                [255, 255, 255, 255]
            };
            write_color_block(
                &mut pixels,
                header.width,
                header.height,
                bx,
                by,
                codes,
                palette,
                alpha,
            );
        }
    }
    Ok(RgbaImage {
        width: header.width,
        height: header.height,
        pixels,
    })
}

fn decode_bc2(data: &[u8], header: &Header, offset: usize) -> Result<RgbaImage> {
    let block_w = header.width.div_ceil(4);
    let block_h = header.height.div_ceil(4);
    let needed = offset + block_w * block_h * 16;
    require_len(data, needed, "DXT3")?;
    let mut pixels = vec![0u8; header.width * header.height * 4];
    for by in 0..block_h {
        for bx in 0..block_w {
            let block = offset + (by * block_w + bx) * 16;
            let color0 = u16::from_le_bytes([data[block + 8], data[block + 9]]);
            let color1 = u16::from_le_bytes([data[block + 10], data[block + 11]]);
            let codes = u32::from_le_bytes([
                data[block + 12],
                data[block + 13],
                data[block + 14],
                data[block + 15],
            ]);
            let palette = bc_color_palette(color0, color1);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= header.width || y >= header.height {
                        continue;
                    }
                    let pos = py * 4 + px;
                    let color_code = ((codes >> (2 * pos)) & 0x3) as usize;
                    let alpha_byte = data[block + pos / 2];
                    let alpha4 = if pos % 2 == 0 {
                        alpha_byte & 0x0f
                    } else {
                        alpha_byte >> 4
                    };
                    let dst = (y * header.width + x) * 4;
                    pixels[dst] = palette[color_code][0];
                    pixels[dst + 1] = palette[color_code][1];
                    pixels[dst + 2] = palette[color_code][2];
                    pixels[dst + 3] = alpha4 * 17;
                }
            }
        }
    }
    Ok(RgbaImage {
        width: header.width,
        height: header.height,
        pixels,
    })
}

fn decode_bc3(data: &[u8], header: &Header, offset: usize) -> Result<RgbaImage> {
    let block_w = header.width.div_ceil(4);
    let block_h = header.height.div_ceil(4);
    let needed = offset + block_w * block_h * 16;
    require_len(data, needed, "DXT5")?;
    let mut pixels = vec![0u8; header.width * header.height * 4];
    for by in 0..block_h {
        for bx in 0..block_w {
            let block = offset + (by * block_w + bx) * 16;
            let alpha_palette = bc3_alpha_palette(data[block], data[block + 1]);
            let mut alpha_codes = 0u64;
            for i in 0..6 {
                alpha_codes |= u64::from(data[block + 2 + i]) << (8 * i);
            }
            let color0 = u16::from_le_bytes([data[block + 8], data[block + 9]]);
            let color1 = u16::from_le_bytes([data[block + 10], data[block + 11]]);
            let codes = u32::from_le_bytes([
                data[block + 12],
                data[block + 13],
                data[block + 14],
                data[block + 15],
            ]);
            let palette = bc_color_palette(color0, color1);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= header.width || y >= header.height {
                        continue;
                    }
                    let pos = py * 4 + px;
                    let color_code = ((codes >> (2 * pos)) & 0x3) as usize;
                    let alpha_code = ((alpha_codes >> (3 * pos)) & 0x7) as usize;
                    let dst = (y * header.width + x) * 4;
                    pixels[dst] = palette[color_code][0];
                    pixels[dst + 1] = palette[color_code][1];
                    pixels[dst + 2] = palette[color_code][2];
                    pixels[dst + 3] = alpha_palette[alpha_code];
                }
            }
        }
    }
    Ok(RgbaImage {
        width: header.width,
        height: header.height,
        pixels,
    })
}

fn decode_uncompressed(data: &[u8], header: &Header) -> Result<RgbaImage> {
    decode_uncompressed_level(data, header, DDS_HEADER_LEN, header.width, header.height)
}

/// Decode one uncompressed mip level at `offset` with explicit dimensions
/// (the header carries mip 0's; deeper levels halve, floored at 1).
fn decode_uncompressed_level(
    data: &[u8],
    header: &Header,
    offset: usize,
    width: usize,
    height: usize,
) -> Result<RgbaImage> {
    if header.pf_flags & DDPF_RGB == 0 {
        return Err(GameUiError::unsupported_dds(
            header.format.clone(),
            format!("unsupported DDS format {}", header.format),
        ));
    }
    let bytes_per_pixel = header.rgb_bits / 8;
    if !matches!(bytes_per_pixel, 3 | 4) {
        return Err(GameUiError::unsupported_dds(
            header.format.clone(),
            format!("unsupported DDS RGB bit depth {}", header.rgb_bits),
        ));
    }
    let needed = offset + width * height * bytes_per_pixel;
    require_len(data, needed, &header.format)?;
    let mut pixels = vec![0u8; width * height * 4];
    for i in 0..width * height {
        let start = offset + i * bytes_per_pixel;
        let mut raw = 0u32;
        for b in 0..bytes_per_pixel {
            raw |= u32::from(data[start + b]) << (8 * b);
        }
        let dst = i * 4;
        pixels[dst] = mask_channel(raw, header.rmask);
        pixels[dst + 1] = mask_channel(raw, header.gmask);
        pixels[dst + 2] = mask_channel(raw, header.bmask);
        pixels[dst + 3] = if header.amask == 0 {
            255
        } else {
            mask_channel(raw, header.amask)
        };
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

/// Decode EVERY mip level of an uncompressed (`A8R8G8B8`-family) DDS, in
/// file order (level 0 first, each level half the previous, floored at 1).
///
/// The map-label fonts ship real mip chains the engine samples with
/// trilinear filtering (`gfx/FX/mapname.shader` Samplers: MipFilter =
/// "Linear", MipMapLodBias = -0.4; the frame4700 capture binds the
/// `hoi_mapfont4` atlas with mips=4), so the replay must upload the SHIPPED
/// mip bytes rather than mip 0 alone or regenerated approximations.
/// Compressed formats fail closed — no current caller needs them.
pub fn decode_mip_chain_bytes(data: &[u8]) -> Result<Vec<RgbaImage>> {
    let header = Header::parse(data)?;
    if !matches!(header.format.as_str(), "A8R8G8B8" | "X8R8G8B8" | "R8G8B8") {
        return Err(GameUiError::unsupported_dds(
            header.format.clone(),
            format!(
                "mip-chain decode is implemented for uncompressed DDS only, got {}",
                header.format
            ),
        ));
    }
    let bytes_per_pixel = header.rgb_bits / 8;
    let mut levels = Vec::with_capacity(header.mip_count.max(1) as usize);
    let mut offset = DDS_HEADER_LEN;
    let mut width = header.width;
    let mut height = header.height;
    for _ in 0..header.mip_count.max(1) {
        levels.push(decode_uncompressed_level(
            data, &header, offset, width, height,
        )?);
        offset += width * height * bytes_per_pixel;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    Ok(levels)
}

fn write_color_block(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    bx: usize,
    by: usize,
    codes: u32,
    palette: [[u8; 3]; 4],
    alpha: [u8; 4],
) {
    for py in 0..4 {
        for px in 0..4 {
            let x = bx * 4 + px;
            let y = by * 4 + py;
            if x >= width || y >= height {
                continue;
            }
            let code = ((codes >> (2 * (py * 4 + px))) & 0x3) as usize;
            let dst = (y * width + x) * 4;
            pixels[dst] = palette[code][0];
            pixels[dst + 1] = palette[code][1];
            pixels[dst + 2] = palette[code][2];
            pixels[dst + 3] = alpha[code];
        }
    }
}

fn bc1_color_palette(color0: u16, color1: u16) -> [[u8; 3]; 4] {
    let c0 = rgb565(color0);
    let c1 = rgb565(color1);
    if color0 > color1 {
        bc_color_palette(color0, color1)
    } else {
        [
            c0,
            c1,
            [
                ((u16::from(c0[0]) + u16::from(c1[0])) / 2) as u8,
                ((u16::from(c0[1]) + u16::from(c1[1])) / 2) as u8,
                ((u16::from(c0[2]) + u16::from(c1[2])) / 2) as u8,
            ],
            [0, 0, 0],
        ]
    }
}

fn bc1_alpha_palette(color0: u16, color1: u16) -> [u8; 4] {
    if color0 > color1 {
        [255, 255, 255, 255]
    } else {
        [255, 255, 255, 0]
    }
}

fn bc_color_palette(color0: u16, color1: u16) -> [[u8; 3]; 4] {
    let c0 = rgb565(color0);
    let c1 = rgb565(color1);
    [
        c0,
        c1,
        [
            ((2 * u16::from(c0[0]) + u16::from(c1[0])) / 3) as u8,
            ((2 * u16::from(c0[1]) + u16::from(c1[1])) / 3) as u8,
            ((2 * u16::from(c0[2]) + u16::from(c1[2])) / 3) as u8,
        ],
        [
            ((u16::from(c0[0]) + 2 * u16::from(c1[0])) / 3) as u8,
            ((u16::from(c0[1]) + 2 * u16::from(c1[1])) / 3) as u8,
            ((u16::from(c0[2]) + 2 * u16::from(c1[2])) / 3) as u8,
        ],
    ]
}

fn bc3_alpha_palette(alpha0: u8, alpha1: u8) -> [u8; 8] {
    if alpha0 > alpha1 {
        [
            alpha0,
            alpha1,
            ((6 * u16::from(alpha0) + u16::from(alpha1)) / 7) as u8,
            ((5 * u16::from(alpha0) + 2 * u16::from(alpha1)) / 7) as u8,
            ((4 * u16::from(alpha0) + 3 * u16::from(alpha1)) / 7) as u8,
            ((3 * u16::from(alpha0) + 4 * u16::from(alpha1)) / 7) as u8,
            ((2 * u16::from(alpha0) + 5 * u16::from(alpha1)) / 7) as u8,
            ((u16::from(alpha0) + 6 * u16::from(alpha1)) / 7) as u8,
        ]
    } else {
        [
            alpha0,
            alpha1,
            ((4 * u16::from(alpha0) + u16::from(alpha1)) / 5) as u8,
            ((3 * u16::from(alpha0) + 2 * u16::from(alpha1)) / 5) as u8,
            ((2 * u16::from(alpha0) + 3 * u16::from(alpha1)) / 5) as u8,
            ((u16::from(alpha0) + 4 * u16::from(alpha1)) / 5) as u8,
            0,
            255,
        ]
    }
}

fn rgb565(value: u16) -> [u8; 3] {
    let r = u32::from((value >> 11) & 0x1f);
    let g = u32::from((value >> 5) & 0x3f);
    let b = u32::from(value & 0x1f);
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}

fn mask_channel(raw: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let value = (raw & mask) >> shift;
    let max = (1u32 << bits) - 1;
    ((value * 255 + max / 2) / max) as u8
}

fn require_len(data: &[u8], needed: usize, label: &str) -> Result<()> {
    if data.len() < needed {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("truncated".to_string()),
            format!(
                "{label} DDS data is truncated: {} bytes, need {needed}",
                data.len()
            ),
        ));
    }
    Ok(())
}

fn le_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data.get(offset..offset + 4).ok_or_else(|| {
        GameUiError::new(
            ErrorKind::InvalidTexture("truncated".to_string()),
            "DDS header is truncated",
        )
    })?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("slice length checked"),
    ))
}

fn add_path(mut err: GameUiError, path: &Path) -> GameUiError {
    err.message = format!("{}: {}", path.display(), err.message);
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxt1_non_multiple_of_four_uses_rounded_block_count() {
        let mut data = minimal_header(3, 2, b"DXT1", 0, 0, 0, 0, 0);
        data.extend_from_slice(&[0x00, 0xf8, 0xe0, 0x07, 0x00, 0x00, 0x00, 0x00]);
        let decoded = decode_bytes(&data).expect("decode dxt1");
        assert_eq!((decoded.image.width, decoded.image.height), (3, 2));
        assert_eq!(decoded.image.pixels.len(), 3 * 2 * 4);
    }

    #[test]
    fn uncompressed_argb_decodes_masks() {
        let mut data = minimal_header(
            1,
            1,
            b"\0\0\0\0",
            DDPF_RGB,
            32,
            0x00ff_0000,
            0x0000_ff00,
            0x0000_00ff,
        );
        data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        data.extend_from_slice(&[0x33, 0x22, 0x11, 0x44]);
        let decoded = decode_bytes(&data).expect("decode argb");
        assert_eq!(decoded.format, "A8R8G8B8");
        assert_eq!(decoded.image.pixels, vec![0x11, 0x22, 0x33, 0x44]);
    }

    fn minimal_header(
        width: u32,
        height: u32,
        fourcc: &[u8; 4],
        pf_flags: u32,
        rgb_bits: u32,
        rmask: u32,
        gmask: u32,
        bmask: u32,
    ) -> Vec<u8> {
        let mut data = vec![0u8; DDS_HEADER_LEN];
        data[0..4].copy_from_slice(b"DDS ");
        data[4..8].copy_from_slice(&124u32.to_le_bytes());
        data[12..16].copy_from_slice(&height.to_le_bytes());
        data[16..20].copy_from_slice(&width.to_le_bytes());
        data[28..32].copy_from_slice(&1u32.to_le_bytes());
        data[76..80].copy_from_slice(&32u32.to_le_bytes());
        let flags = if fourcc.iter().any(|b| *b != 0) {
            DDPF_FOURCC
        } else {
            pf_flags
        };
        data[80..84].copy_from_slice(&flags.to_le_bytes());
        data[84..88].copy_from_slice(fourcc);
        data[88..92].copy_from_slice(&rgb_bits.to_le_bytes());
        data[92..96].copy_from_slice(&rmask.to_le_bytes());
        data[96..100].copy_from_slice(&gmask.to_le_bytes());
        data[100..104].copy_from_slice(&bmask.to_le_bytes());
        data
    }
}
