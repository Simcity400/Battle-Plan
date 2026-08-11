use crate::{ErrorKind, GameUiError, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const DDS_HEADER_LEN: usize = 128;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_VOLUME: u32 = 0x20_0000;
/// Upper bound on header-claimed pixel count (8192x8192). Header dimensions
/// are attacker-controlled bytes from mod content that is hash-pinned but not
/// structurally validated; without this gate a crafted header can wrap the
/// size arithmetic or force a multi-gigabyte allocation. Fail-closed: reject
/// with a typed `InvalidTexture` error, never panic or abort.
const DDS_MAX_PIXELS: usize = 1 << 26;

/// Number of mip levels a `width`x`height` surface can actually carry: each
/// level halves both axes (floored at 1), so the chain ends when both axes
/// reach 1. Under `DDS_MAX_PIXELS` this is at most 27.
///
/// `dwMipMapCount` (header offset 28) is attacker-controlled and NOT covered
/// by the pixel cap, which bounds only width x height. Without this bound a
/// header claiming `0xFFFF_FFFF` levels asks `Vec::with_capacity` for ~160 GiB
/// of `RgbaImage` headers; a failed allocation goes to `handle_alloc_error`,
/// which ABORTS the process rather than returning a `GameUiError`. Bound the
/// header field where every other header field is bounded — at parse — so no
/// downstream size expression can ever see an impossible level count.
fn max_mip_levels(width: usize, height: usize) -> u32 {
    let mut levels = 1u32;
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        levels += 1;
    }
    levels
}

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

/// Reject non-2D surface topologies. Cubemap faces and volume slices follow
/// the mip-0 payload in the same file, so decoding one as a plain 2D surface
/// silently reinterprets the extra faces/slices as mip data. Every decode
/// entry point must apply this — see `decode_mip_chain_bytes`.
fn reject_non_2d(header: &Header) -> Result<()> {
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
    Ok(())
}

pub fn decode_bytes(data: &[u8]) -> Result<DdsImage> {
    let header = Header::parse(data)?;
    reject_non_2d(&header)?;

    let image = match header.format.as_str() {
        "DXT1" => decode_bc1(data, &header, DDS_HEADER_LEN, true)?,
        "DXT3" => decode_bc2(data, &header, DDS_HEADER_LEN)?,
        "DXT5" => decode_bc3(data, &header, DDS_HEADER_LEN)?,
        "A8R8G8B8" | "X8R8G8B8" | "R8G8B8" | "X1R5G5B5" => decode_uncompressed(data, &header)?,
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
        if width
            .checked_mul(height)
            .is_none_or(|pixels| pixels > DDS_MAX_PIXELS)
        {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("dimensions".to_string()),
                format!("DDS dimensions {width}x{height} exceed the {DDS_MAX_PIXELS}-pixel cap"),
            ));
        }
        let level_cap = max_mip_levels(width, height);
        if mip_count > level_cap {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("mip_count".to_string()),
                format!(
                    "DDS mip count {mip_count} exceeds the {level_cap} levels a \
                     {width}x{height} surface can carry"
                ),
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
        if rgb_bits == 16
            && rmask == 0x0000_7c00
            && gmask == 0x0000_03e0
            && bmask == 0x0000_001f
            && amask == 0
        {
            return "X1R5G5B5".to_string();
        }
        return format!("RGB{rgb_bits}_R{rmask:08x}_G{gmask:08x}_B{bmask:08x}_A{amask:08x}");
    }
    if rgb_bits > 0 {
        format!("RGB{rgb_bits}")
    } else {
        "UNKNOWN".to_string()
    }
}

/// Checked `factors product + offset` for header-derived sizes. Overflow maps
/// to a typed `InvalidTexture("dimensions")` refusal — the header math cannot
/// overflow under the `DDS_MAX_PIXELS` gate, but every size expression stays
/// checked so no future path can silently wrap.
fn checked_size(factors: [usize; 3], offset: usize, label: &str) -> Result<usize> {
    factors
        .iter()
        .try_fold(1usize, |acc, factor| acc.checked_mul(*factor))
        .and_then(|bytes| bytes.checked_add(offset))
        .ok_or_else(|| {
            GameUiError::new(
                ErrorKind::InvalidTexture("dimensions".to_string()),
                format!("{label} DDS size arithmetic overflows"),
            )
        })
}

fn decode_bc1(data: &[u8], header: &Header, offset: usize, transparent: bool) -> Result<RgbaImage> {
    let block_w = header.width.div_ceil(4);
    let block_h = header.height.div_ceil(4);
    let needed = checked_size([block_w, block_h, 8], offset, "DXT1")?;
    require_len(data, needed, "DXT1")?;
    let mut pixels = vec![0u8; checked_size([header.width, header.height, 4], 0, "DXT1")?];
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
    let needed = checked_size([block_w, block_h, 16], offset, "DXT3")?;
    require_len(data, needed, "DXT3")?;
    let mut pixels = vec![0u8; checked_size([header.width, header.height, 4], 0, "DXT3")?];
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
    let needed = checked_size([block_w, block_h, 16], offset, "DXT5")?;
    require_len(data, needed, "DXT5")?;
    let mut pixels = vec![0u8; checked_size([header.width, header.height, 4], 0, "DXT5")?];
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
    if !matches!(bytes_per_pixel, 2 | 3 | 4) {
        return Err(GameUiError::unsupported_dds(
            header.format.clone(),
            format!("unsupported DDS RGB bit depth {}", header.rgb_bits),
        ));
    }
    let needed = checked_size([width, height, bytes_per_pixel], offset, &header.format)?;
    require_len(data, needed, &header.format)?;
    let mut pixels = vec![0u8; checked_size([width, height, 4], 0, &header.format)?];
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
    reject_non_2d(&header)?;
    if !matches!(
        header.format.as_str(),
        "A8R8G8B8" | "X8R8G8B8" | "R8G8B8" | "X1R5G5B5"
    ) {
        return Err(GameUiError::unsupported_dds(
            header.format.clone(),
            format!(
                "mip-chain decode is implemented for uncompressed DDS only, got {}",
                header.format
            ),
        ));
    }
    let bytes_per_pixel = header.rgb_bits / 8;
    // `Header::parse` bounds `mip_count` by `max_mip_levels(width, height)`
    // (<= 27 under the pixel cap), so this reservation is header-derived but
    // bounded BEFORE the allocation, like every other size in this file.
    let level_count = header.mip_count.max(1);
    debug_assert!(level_count <= max_mip_levels(header.width, header.height));
    let mut levels = Vec::with_capacity(level_count as usize);
    let mut offset = DDS_HEADER_LEN;
    let mut width = header.width;
    let mut height = header.height;
    for _ in 0..level_count {
        levels.push(decode_uncompressed_level(
            data, &header, offset, width, height,
        )?);
        offset = checked_size([width, height, bytes_per_pixel], offset, &header.format)?;
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

/// Scale one masked channel to 8 bits. Every step is `u64` so no mask shape
/// can panic: `bits == 32` would overflow `1u32 << 32`, and `value * 255`
/// would overflow `u32` for a wide mask — both are release panics under this
/// workspace's `overflow-checks = true`. A zero mask has no channel at all and
/// yields 0; it must never reach the division.
fn mask_channel(raw: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = u64::from(mask.count_ones());
    let value = u64::from((raw & mask) >> shift);
    let max = (1u64 << bits) - 1;
    debug_assert!(max > 0, "a non-zero mask always has at least one bit");
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

    #[test]
    fn uncompressed_x1r5g5b5_portrait_decodes_exact_masks() {
        let mut data = minimal_header(
            3,
            1,
            b"\0\0\0\0",
            DDPF_RGB,
            16,
            0x0000_7c00,
            0x0000_03e0,
            0x0000_001f,
        );
        data.extend_from_slice(&0x7c00u16.to_le_bytes());
        data.extend_from_slice(&0x03e0u16.to_le_bytes());
        data.extend_from_slice(&0x001fu16.to_le_bytes());
        let decoded = decode_bytes(&data).expect("decode X1R5G5B5 portrait");
        assert_eq!(decoded.format, "X1R5G5B5");
        assert_eq!(
            decoded.image.pixels,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]
        );
    }

    #[test]
    fn dxt3_known_block_decodes_color_and_nibble_alpha() {
        let mut data = minimal_header(4, 4, b"DXT3", 0, 0, 0, 0, 0);
        // 8 bytes of 4-bit alpha (all 0xF -> 255), then color0/color1/codes.
        data.extend_from_slice(&[0xff; 8]);
        data.extend_from_slice(&0xf800u16.to_le_bytes()); // color0 = pure red
        data.extend_from_slice(&0x001fu16.to_le_bytes()); // color1 = pure blue
        data.extend_from_slice(&0u32.to_le_bytes()); // all codes select color0
        let decoded = decode_bytes(&data).expect("decode dxt3");
        assert_eq!(decoded.format, "DXT3");
        assert_eq!(decoded.image.pixels.len(), 4 * 4 * 4);
        for pixel in decoded.image.pixels.chunks_exact(4) {
            assert_eq!(pixel, [255, 0, 0, 255]);
        }
    }

    #[test]
    fn dxt5_known_block_decodes_color_and_interpolated_alpha() {
        let mut data = minimal_header(4, 4, b"DXT5", 0, 0, 0, 0, 0);
        data.push(200); // alpha0
        data.push(0); // alpha1
        data.extend_from_slice(&[0u8; 6]); // all alpha codes select alpha0
        data.extend_from_slice(&0xf800u16.to_le_bytes()); // color0 = pure red
        data.extend_from_slice(&0u16.to_le_bytes()); // color1 = black
        data.extend_from_slice(&0u32.to_le_bytes()); // all codes select color0
        let decoded = decode_bytes(&data).expect("decode dxt5");
        assert_eq!(decoded.format, "DXT5");
        for pixel in decoded.image.pixels.chunks_exact(4) {
            assert_eq!(pixel, [255, 0, 0, 200]);
        }
    }

    #[test]
    fn cubemap_is_rejected_with_typed_error() {
        let mut data = minimal_header(4, 4, b"DXT1", 0, 0, 0, 0, 0);
        data[112..116].copy_from_slice(&DDSCAPS2_CUBEMAP.to_le_bytes());
        let err = decode_bytes(&data).expect_err("cubemap must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("cubemap".to_string()));
    }

    #[test]
    fn volume_is_rejected_with_typed_error() {
        let mut data = minimal_header(4, 4, b"DXT1", 0, 0, 0, 0, 0);
        data[112..116].copy_from_slice(&DDSCAPS2_VOLUME.to_le_bytes());
        let err = decode_bytes(&data).expect_err("volume must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("volume".to_string()));
    }

    #[test]
    fn truncated_header_is_rejected() {
        let data = b"DDS \x7c\x00\x00\x00".to_vec();
        let err = decode_bytes(&data).expect_err("short header must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("truncated".to_string()));
    }

    #[test]
    fn truncated_dxt1_payload_is_rejected() {
        let mut data = minimal_header(4, 4, b"DXT1", 0, 0, 0, 0, 0);
        data.extend_from_slice(&[0u8; 7]); // one block needs 8 bytes
        let err = decode_bytes(&data).expect_err("short payload must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("truncated".to_string()));
    }

    #[test]
    fn huge_uncompressed_dimensions_are_a_typed_refusal_not_a_panic() {
        // width = height = 2^31: unchecked release-mode math would wrap
        // width*height*4 to 0 and index out of bounds. Must refuse instead.
        let mut data = minimal_header(
            0x8000_0000,
            0x8000_0000,
            b"\0\0\0\0",
            DDPF_RGB,
            32,
            0x00ff_0000,
            0x0000_ff00,
            0x0000_00ff,
        );
        data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        let err = decode_bytes(&data).expect_err("huge dimensions must be rejected");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("dimensions".to_string())
        );
    }

    #[test]
    fn huge_dxt1_dimensions_are_a_typed_refusal_not_a_panic() {
        let data = minimal_header(u32::MAX, u32::MAX, b"DXT1", 0, 0, 0, 0, 0);
        let err = decode_bytes(&data).expect_err("huge dimensions must be rejected");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("dimensions".to_string())
        );
    }

    #[test]
    fn mip_chain_decodes_every_shipped_level() {
        let mut data = minimal_header(
            2,
            2,
            b"\0\0\0\0",
            DDPF_RGB,
            32,
            0x00ff_0000,
            0x0000_ff00,
            0x0000_00ff,
        );
        data[28..32].copy_from_slice(&2u32.to_le_bytes()); // mip_count = 2
        data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        for _ in 0..4 {
            data.extend_from_slice(&[0x33, 0x22, 0x11, 0x44]); // level 0 (BGRA)
        }
        data.extend_from_slice(&[0x66, 0x55, 0x44, 0x77]); // level 1 (BGRA)
        let levels = decode_mip_chain_bytes(&data).expect("decode mip chain");
        assert_eq!(levels.len(), 2);
        assert_eq!((levels[0].width, levels[0].height), (2, 2));
        assert_eq!((levels[1].width, levels[1].height), (1, 1));
        assert_eq!(&levels[0].pixels[0..4], [0x11, 0x22, 0x33, 0x44]);
        assert_eq!(levels[1].pixels, vec![0x44, 0x55, 0x66, 0x77]);
    }

    #[test]
    fn truncated_mip_chain_is_rejected() {
        let mut data = minimal_header(
            2,
            2,
            b"\0\0\0\0",
            DDPF_RGB,
            32,
            0x00ff_0000,
            0x0000_ff00,
            0x0000_00ff,
        );
        data[28..32].copy_from_slice(&2u32.to_le_bytes());
        data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        for _ in 0..4 {
            data.extend_from_slice(&[0x33, 0x22, 0x11, 0x44]); // level 0 only
        }
        let err = decode_mip_chain_bytes(&data).expect_err("missing level 1 must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("truncated".to_string()));
    }

    #[test]
    fn unbounded_mip_count_is_a_typed_refusal_not_an_allocation_abort() {
        // R11 C1: 0xFFFF_FFFF levels x 40 bytes per RgbaImage header is a
        // ~160 GiB `Vec::with_capacity`, which aborts the process instead of
        // returning an error. The header field must be bounded at parse.
        let mut data = minimal_header(
            4,
            4,
            b"\0\0\0\0",
            DDPF_RGB,
            32,
            0x00ff_0000,
            0x0000_ff00,
            0x0000_00ff,
        );
        data[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 4 * 4 * 4]);
        let err = decode_mip_chain_bytes(&data).expect_err("absurd mip count must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("mip_count".to_string()));
        // The same bound applies to every other entry point on this header.
        let err = decode_bytes(&data).expect_err("absurd mip count must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("mip_count".to_string()));
        let err = info_bytes(&data).expect_err("absurd mip count must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("mip_count".to_string()));
    }

    #[test]
    fn shipped_level_counts_are_still_admitted() {
        // A full 4x4 chain is 3 levels (4x4, 2x2, 1x1); the bound must admit
        // exactly that and refuse only one level more.
        assert_eq!(max_mip_levels(4, 4), 3);
        assert_eq!(max_mip_levels(1, 1), 1);
        assert_eq!(max_mip_levels(2048, 1024), 12);
        assert!(max_mip_levels(1 << 26, 1) <= 27);
    }

    #[test]
    fn mip_chain_rejects_cubemaps_and_volumes_like_the_2d_decoder() {
        for (caps2, label) in [(DDSCAPS2_CUBEMAP, "cubemap"), (DDSCAPS2_VOLUME, "volume")] {
            let mut data = minimal_header(
                2,
                2,
                b"\0\0\0\0",
                DDPF_RGB,
                32,
                0x00ff_0000,
                0x0000_ff00,
                0x0000_00ff,
            );
            data[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
            data[112..116].copy_from_slice(&caps2.to_le_bytes());
            data.extend_from_slice(&[0u8; 2 * 2 * 4]);
            // Without the shared gate this decodes the extra faces/slices as
            // ordinary deeper mip levels.
            let err = decode_mip_chain_bytes(&data)
                .expect_err("mip-chain decode must reject non-2D topologies");
            assert_eq!(err.kind, ErrorKind::InvalidTexture(label.to_string()));
        }
    }

    #[test]
    fn mask_channel_is_total_for_every_mask_shape() {
        // R11 C10(a) / R18-3: a 32-bit mask overflows `1u32 << 32` and
        // `value * 255`; both are release panics under overflow-checks.
        assert_eq!(mask_channel(u32::MAX, u32::MAX), 255);
        assert_eq!(mask_channel(0, u32::MAX), 0);
        assert_eq!(mask_channel(0x8000_0000, 0xffff_ffff), 128);
        assert_eq!(mask_channel(0xdead_beef, 0), 0);
        assert_eq!(mask_channel(0x00ff_0000, 0x00ff_0000), 255);
        assert_eq!(mask_channel(0x0000_0000, 0x00ff_0000), 0);
    }

    #[test]
    fn mip_chain_rejects_compressed_formats() {
        let mut data = minimal_header(4, 4, b"DXT1", 0, 0, 0, 0, 0);
        data.extend_from_slice(&[0u8; 8]);
        let err = decode_mip_chain_bytes(&data).expect_err("compressed chain must fail closed");
        assert_eq!(
            err.kind,
            ErrorKind::UnsupportedDdsFormat("DXT1".to_string())
        );
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
