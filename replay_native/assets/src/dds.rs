//! Minimal, self-contained DDS reader for the game textures.
//!
//! the game's `.dds` files (audited in M1 across ~1600 sampled interface/font/flag/goal
//! textures) are overwhelmingly **BGRA8 uncompressed** (32-bit, masks
//! R=0x00ff0000 G=0x0000ff00 B=0x000000ff A=0xff000000), with a minority of
//! **DXT1 (BC1) / DXT3 (BC2) / DXT5 (BC3)** and a handful of oddities (RGB24,
//! a DX10-header B8G8R8A8_UNORM_SRGB).
//!
//! Two output paths:
//!   * [`DecodedImage`] — straight RGBA8 in linear-ish bytes ready for an
//!     `Rgba8Unorm` texture. The CPU transcode FALLBACK (BGRA/RGB24/A8/luminance,
//!     and a software BCn decoder) always lands here, so anything wgpu cannot
//!     sample directly still renders.
//!   * [`GpuBlocks`] — the raw BCn block payload + a `wgpu`-compatible format tag,
//!     for the direct-upload fast path (requires the `TEXTURE_COMPRESSION_BC`
//!     feature on the device).
//!
//! Alpha is reported as straight vs premultiplied so the renderer can pick the
//! right blend (the game DXT2/DXT4 would be premultiplied; DXT3/DXT5 are straight).

use std::fmt;

const DDS_MAGIC: u32 = 0x2053_4444; // "DDS " little-endian

// ddspf flags
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;

/// How the source pixels were authored, w.r.t. alpha pre-multiplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    /// RGB is independent of A (the common case: BGRA8, DXT1/3/5).
    Straight,
    /// RGB already multiplied by A (DXT2/DXT4); renderer must NOT multiply again.
    Premultiplied,
    /// Fully opaque / no alpha channel.
    Opaque,
}

/// The DDS pixel format we classified the file as (for the audit + routing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DdsFormat {
    Bgra8,
    Bgrx8,
    Rgba8,
    Rgbx8,
    Rgb24,
    A8,
    Lum8,
    LumA8,
    Dxt1, // BC1
    Dxt2, // BC2 premultiplied
    Dxt3, // BC2 straight
    Dxt4, // BC3 premultiplied
    Dxt5, // BC3 straight
    Bc4,
    Bc5,
    Bc7,
    /// 16-bit packed RGB: X1R5G5B5 (the game generic portraits). No alpha.
    Rgb555,
    /// 16-bit packed ARGB1555.
    Argb1555,
    /// RXGB — a DXT5-compressed normal-map swizzle (R and A channels swapped).
    Rxgb,
    /// DX10 header: carries the raw DXGI format number.
    Dx10(u32),
    Unknown(u32),
}

impl fmt::Display for DdsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdsFormat::Dx10(n) => write!(f, "DX10/DXGI={}", n),
            DdsFormat::Unknown(flags) => write!(f, "Unknown(pfflags={:#x})", flags),
            other => write!(f, "{:?}", other),
        }
    }
}

/// A fully CPU-decoded RGBA8 image (straight alpha), top-down, tightly packed.
#[derive(Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, top-left origin, no padding. Length = w*h*4.
    pub rgba: Vec<u8>,
    pub source_format: DdsFormat,
    pub alpha: AlphaMode,
}

impl fmt::Debug for DecodedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .field("source_format", &self.source_format)
            .field("alpha", &self.alpha)
            .finish()
    }
}

/// Raw compressed block payload for the direct-upload path.
#[derive(Clone, Debug)]
pub struct GpuBlocks {
    pub width: u32,
    pub height: u32,
    /// The single (mip 0) compressed block payload.
    pub blocks: Vec<u8>,
    pub source_format: DdsFormat,
    pub alpha: AlphaMode,
}

#[derive(Debug)]
pub enum DdsError {
    TooShort,
    BadMagic,
    Unsupported(String),
}

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DdsError::TooShort => write!(f, "file too short for a DDS header"),
            DdsError::BadMagic => write!(f, "not a DDS file (bad magic)"),
            DdsError::Unsupported(s) => write!(f, "unsupported DDS: {}", s),
        }
    }
}
impl std::error::Error for DdsError {}

/// The parsed DDS header fields we care about.
#[derive(Clone, Debug)]
pub struct DdsHeader {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub format: DdsFormat,
    pub alpha: AlphaMode,
    /// Byte offset where mip-0 pixel/block data begins.
    pub data_offset: usize,
}

fn rd_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// Parse just the header (and classify the format). Cheap; used by the audit.
pub fn parse_header(bytes: &[u8]) -> Result<DdsHeader, DdsError> {
    if bytes.len() < 128 {
        return Err(DdsError::TooShort);
    }
    if rd_u32(bytes, 0) != DDS_MAGIC {
        return Err(DdsError::BadMagic);
    }
    // DDS_HEADER begins at byte 4. Fields (offsets relative to file start):
    //   4  dwSize, 8 dwFlags, 12 dwHeight, 16 dwWidth, 20 dwPitchOrLinearSize,
    //   24 dwDepth, 28 dwMipMapCount, 32..76 reserved1[11],
    //   76 ddspf (32 bytes), 108 dwCaps...
    let height = rd_u32(bytes, 12);
    let width = rd_u32(bytes, 16);
    let mip_count = rd_u32(bytes, 28);

    // ddspf at offset 76.
    let pf = 76usize;
    let pf_flags = rd_u32(bytes, pf + 4);
    let fourcc = &bytes[pf + 8..pf + 12];
    let rgb_bits = rd_u32(bytes, pf + 12);
    let r_mask = rd_u32(bytes, pf + 16);
    let g_mask = rd_u32(bytes, pf + 20);
    let b_mask = rd_u32(bytes, pf + 24);
    let a_mask = rd_u32(bytes, pf + 28);

    let mut data_offset = 128usize;
    let format;
    let alpha;

    if pf_flags & DDPF_FOURCC != 0 {
        match fourcc {
            b"DXT1" => {
                format = DdsFormat::Dxt1;
                // DXT1 may carry 1-bit alpha; treat as straight (opaque-ish).
                alpha = AlphaMode::Straight;
            }
            b"DXT2" => {
                format = DdsFormat::Dxt2;
                alpha = AlphaMode::Premultiplied;
            }
            b"DXT3" => {
                format = DdsFormat::Dxt3;
                alpha = AlphaMode::Straight;
            }
            b"DXT4" => {
                format = DdsFormat::Dxt4;
                alpha = AlphaMode::Premultiplied;
            }
            b"DXT5" => {
                format = DdsFormat::Dxt5;
                alpha = AlphaMode::Straight;
            }
            b"BC4U" | b"ATI1" => {
                format = DdsFormat::Bc4;
                alpha = AlphaMode::Opaque;
            }
            b"BC5U" | b"ATI2" => {
                format = DdsFormat::Bc5;
                alpha = AlphaMode::Opaque;
            }
            b"RXGB" => {
                // DXT5 payload with R<->A swizzled (normal maps). Opaque colour.
                format = DdsFormat::Rxgb;
                alpha = AlphaMode::Opaque;
            }
            b"DX10" => {
                // DXGI format number lives in the 20-byte DX10 header at offset 128.
                if bytes.len() < 148 {
                    return Err(DdsError::TooShort);
                }
                let dxgi = rd_u32(bytes, 128);
                data_offset = 148;
                // Map a few that the game actually emits.
                (format, alpha) = match dxgi {
                    87 => (DdsFormat::Bgra8, AlphaMode::Straight), // B8G8R8A8_UNORM
                    91 => (DdsFormat::Bgra8, AlphaMode::Straight), // B8G8R8A8_UNORM_SRGB
                    28 | 29 => (DdsFormat::Rgba8, AlphaMode::Straight), // R8G8B8A8_(S)UNORM
                    98 => (DdsFormat::Bc7, AlphaMode::Straight),   // BC7_UNORM
                    99 => (DdsFormat::Bc7, AlphaMode::Straight),   // BC7_UNORM_SRGB
                    71 | 72 => (DdsFormat::Dxt1, AlphaMode::Straight), // BC1
                    74 | 75 => (DdsFormat::Dxt3, AlphaMode::Straight), // BC2
                    77 | 78 => (DdsFormat::Dxt5, AlphaMode::Straight), // BC3
                    80 | 81 => (DdsFormat::Bc4, AlphaMode::Opaque),
                    83 | 84 => (DdsFormat::Bc5, AlphaMode::Opaque),
                    other => (DdsFormat::Dx10(other), AlphaMode::Straight),
                };
            }
            other => {
                let n = u32::from_le_bytes([other[0], other[1], other[2], other[3]]);
                format = DdsFormat::Unknown(n);
                alpha = AlphaMode::Straight;
            }
        }
    } else if pf_flags & DDPF_RGB != 0 {
        let has_alpha = pf_flags & DDPF_ALPHAPIXELS != 0;
        format = match (rgb_bits, r_mask, g_mask, b_mask) {
            (32, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff) => {
                if has_alpha && a_mask == 0xff00_0000 {
                    DdsFormat::Bgra8
                } else {
                    DdsFormat::Bgrx8
                }
            }
            (32, 0x0000_00ff, 0x0000_ff00, 0x00ff_0000) => {
                if has_alpha {
                    DdsFormat::Rgba8
                } else {
                    DdsFormat::Rgbx8
                }
            }
            (24, _, _, _) => DdsFormat::Rgb24,
            // 16-bit packed: X1R5G5B5 (R=0x7c00 G=0x03e0 B=0x001f) used by the
            // generic portraits, and A1R5G5B5 when the alpha bit is present.
            (16, 0x7c00, 0x03e0, 0x001f) => {
                if has_alpha {
                    DdsFormat::Argb1555
                } else {
                    DdsFormat::Rgb555
                }
            }
            _ => DdsFormat::Unknown(pf_flags),
        };
        alpha = match format {
            DdsFormat::Rgb555 => AlphaMode::Opaque,
            _ if has_alpha => AlphaMode::Straight,
            _ => AlphaMode::Opaque,
        };
    } else if pf_flags & DDPF_LUMINANCE != 0 {
        let has_alpha = pf_flags & DDPF_ALPHAPIXELS != 0;
        format = if has_alpha {
            DdsFormat::LumA8
        } else {
            DdsFormat::Lum8
        };
        alpha = if has_alpha {
            AlphaMode::Straight
        } else {
            AlphaMode::Opaque
        };
    } else if pf_flags & DDPF_ALPHA != 0 {
        format = DdsFormat::A8;
        alpha = AlphaMode::Straight;
    } else {
        format = DdsFormat::Unknown(pf_flags);
        alpha = AlphaMode::Straight;
    }

    // Sanity: zero dims would break every downstream consumer.
    if width == 0 || height == 0 {
        return Err(DdsError::Unsupported(format!(
            "degenerate dimensions {}x{}",
            width, height
        )));
    }

    Ok(DdsHeader {
        width,
        height,
        mip_count,
        format,
        alpha,
        data_offset,
    })
}

/// Returns true if the format can be uploaded straight to a wgpu BCn texture
/// (given the `TEXTURE_COMPRESSION_BC` feature). Everything else must go through
/// the CPU [`decode`] fallback.
pub fn is_gpu_native_bcn(f: DdsFormat) -> bool {
    matches!(
        f,
        DdsFormat::Dxt1
            | DdsFormat::Dxt3
            | DdsFormat::Dxt5
            | DdsFormat::Bc4
            | DdsFormat::Bc5
            | DdsFormat::Bc7
    )
}

/// Pull the mip-0 BCn block payload for direct GPU upload. Caller must have
/// already checked [`is_gpu_native_bcn`]; premultiplied BC formats (DXT2/4) are
/// rejected here on purpose so they take the CPU path that un-multiplies.
pub fn read_gpu_blocks(bytes: &[u8]) -> Result<GpuBlocks, DdsError> {
    let h = parse_header(bytes)?;
    if !is_gpu_native_bcn(h.format) {
        return Err(DdsError::Unsupported(format!(
            "{} is not a directly-uploadable BCn format",
            h.format
        )));
    }
    let bpb = block_bytes(h.format);
    // Checked block-payload length (BUG B): header-controlled dims can overflow `bw*bh*bpb`.
    let len = checked_block_len(h.width as usize, h.height as usize, bpb)?;
    let end = h
        .data_offset
        .checked_add(len)
        .ok_or_else(|| DdsError::Unsupported("data offset + block length overflow".to_string()))?;
    if bytes.len() < end {
        return Err(DdsError::TooShort);
    }
    Ok(GpuBlocks {
        width: h.width,
        height: h.height,
        blocks: bytes[h.data_offset..end].to_vec(),
        source_format: h.format,
        alpha: h.alpha,
    })
}

fn block_bytes(f: DdsFormat) -> usize {
    match f {
        DdsFormat::Dxt1 | DdsFormat::Bc4 => 8,
        DdsFormat::Dxt2
        | DdsFormat::Dxt3
        | DdsFormat::Dxt4
        | DdsFormat::Dxt5
        | DdsFormat::Bc5
        | DdsFormat::Bc7 => 16,
        _ => 0,
    }
}

/// `width*height` as a `usize`, or `Err(Unsupported)` if the product overflows. The DDS
/// `width`/`height` come straight from the (attacker-controllable) file header, so a malformed
/// file can declare e.g. 0xFFFFFFFF×0xFFFFFFFF; on 64-bit, `w*h*bytes_per_pixel` overflows
/// `usize` and (with overflow-checks off in release) WRAPS to a small/garbage value, after which
/// `vec![0u8; that]` + the decode loops index out of bounds. Validating with checked arithmetic
/// BEFORE any allocation turns that into a clean Err instead of a panic / OOB. (Codex-confirmed.)
fn checked_pixels(w: usize, h: usize) -> Result<usize, DdsError> {
    w.checked_mul(h)
        .ok_or_else(|| DdsError::Unsupported(format!("dimensions {w}x{h} overflow usize")))
}

/// `pixels * bytes_per_pixel` with overflow → Err. Caps at `isize::MAX` (the max a `Vec`
/// allocation can address) so a merely-huge-but-non-overflowing size is also rejected up front.
fn checked_byte_len(pixels: usize, bpp: usize) -> Result<usize, DdsError> {
    let n = pixels.checked_mul(bpp).ok_or_else(|| {
        DdsError::Unsupported(format!("{pixels} px * {bpp} bytes overflow usize"))
    })?;
    if n > isize::MAX as usize {
        return Err(DdsError::Unsupported(format!(
            "decoded size {n} exceeds isize::MAX"
        )));
    }
    Ok(n)
}

/// Required BCn block-payload byte length for `w*h` at `bytes_per_block`, with overflow → Err.
fn checked_block_len(w: usize, h: usize, bpb: usize) -> Result<usize, DdsError> {
    // block grid is ceil(w/4) x ceil(h/4); +3 cannot overflow for any real usize dimension
    // but guard it anyway to stay total.
    let bw = w.checked_add(3).map(|v| v / 4).unwrap_or(0).max(1);
    let bh = h.checked_add(3).map(|v| v / 4).unwrap_or(0).max(1);
    let blocks = bw
        .checked_mul(bh)
        .ok_or_else(|| DdsError::Unsupported(format!("block grid {bw}x{bh} overflows usize")))?;
    checked_byte_len(blocks, bpb)
}

/// CPU-decode ANY supported DDS to straight-alpha RGBA8 (the transcode fallback).
///
/// Covers: BGRA8/BGRX8, RGBA8/RGBX8, RGB24, A8, luminance/luminance-alpha, and a
/// software BCn (DXT1/2/3/4/5) decoder. Premultiplied inputs (DXT2/DXT4) are
/// un-premultiplied so the output is always straight alpha.
pub fn decode(bytes: &[u8]) -> Result<DecodedImage, DdsError> {
    let h = parse_header(bytes)?;
    let (w, hgt) = (h.width as usize, h.height as usize);
    let src = &bytes[h.data_offset..];
    // BUG B (Codex-confirmed): validate the output-buffer size with CHECKED arithmetic before
    // allocating. `w`/`hgt` are header-controlled u32s; `w * hgt * 4` can overflow usize on a
    // malformed file and (overflow-checks off in release) wrap to a tiny value → the decode
    // loops would then index out of bounds. checked_* turns that into a clean Err.
    let pixels = checked_pixels(w, hgt)?;
    let out_len = checked_byte_len(pixels, 4)?;
    let mut rgba = vec![0u8; out_len];

    match h.format {
        DdsFormat::Bgra8 | DdsFormat::Bgrx8 => {
            need(src, w * hgt * 4)?;
            for i in 0..w * hgt {
                let s = i * 4;
                rgba[s] = src[s + 2]; // R <- B-position
                rgba[s + 1] = src[s + 1]; // G
                rgba[s + 2] = src[s]; // B <- R-position
                rgba[s + 3] = if h.format == DdsFormat::Bgra8 {
                    src[s + 3]
                } else {
                    255
                };
            }
        }
        DdsFormat::Rgba8 | DdsFormat::Rgbx8 => {
            need(src, w * hgt * 4)?;
            rgba[..w * hgt * 4].copy_from_slice(&src[..w * hgt * 4]);
            if h.format == DdsFormat::Rgbx8 {
                for i in 0..w * hgt {
                    rgba[i * 4 + 3] = 255;
                }
            }
        }
        DdsFormat::Rgb24 => {
            need(src, w * hgt * 3)?;
            for i in 0..w * hgt {
                rgba[i * 4] = src[i * 3];
                rgba[i * 4 + 1] = src[i * 3 + 1];
                rgba[i * 4 + 2] = src[i * 3 + 2];
                rgba[i * 4 + 3] = 255;
            }
        }
        DdsFormat::A8 => {
            need(src, w * hgt)?;
            for i in 0..w * hgt {
                rgba[i * 4] = 255;
                rgba[i * 4 + 1] = 255;
                rgba[i * 4 + 2] = 255;
                rgba[i * 4 + 3] = src[i];
            }
        }
        DdsFormat::Lum8 => {
            need(src, w * hgt)?;
            for i in 0..w * hgt {
                let l = src[i];
                rgba[i * 4] = l;
                rgba[i * 4 + 1] = l;
                rgba[i * 4 + 2] = l;
                rgba[i * 4 + 3] = 255;
            }
        }
        DdsFormat::LumA8 => {
            need(src, w * hgt * 2)?;
            for i in 0..w * hgt {
                let l = src[i * 2];
                let a = src[i * 2 + 1];
                rgba[i * 4] = l;
                rgba[i * 4 + 1] = l;
                rgba[i * 4 + 2] = l;
                rgba[i * 4 + 3] = a;
            }
        }
        DdsFormat::Rgb555 | DdsFormat::Argb1555 => {
            need(src, w * hgt * 2)?;
            let with_alpha = h.format == DdsFormat::Argb1555;
            for i in 0..w * hgt {
                let v = u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
                let r = ((v >> 10) & 0x1f) as u32;
                let g = ((v >> 5) & 0x1f) as u32;
                let b = (v & 0x1f) as u32;
                rgba[i * 4] = ((r * 255 + 15) / 31) as u8;
                rgba[i * 4 + 1] = ((g * 255 + 15) / 31) as u8;
                rgba[i * 4 + 2] = ((b * 255 + 15) / 31) as u8;
                rgba[i * 4 + 3] = if !with_alpha || (v & 0x8000) != 0 {
                    255
                } else {
                    0
                };
            }
        }
        DdsFormat::Dxt1 => decode_bc1(src, w, hgt, &mut rgba)?,
        DdsFormat::Dxt2 | DdsFormat::Dxt3 => decode_bc2(src, w, hgt, &mut rgba)?,
        DdsFormat::Dxt4 | DdsFormat::Dxt5 => decode_bc3(src, w, hgt, &mut rgba)?,
        DdsFormat::Rxgb => {
            // RXGB = DXT5 block layout, but the colour holds (A?,G,B) and the
            // separate alpha block holds R. Decode as BC3, then un-swizzle:
            // real R = decoded alpha, real A = opaque.
            decode_bc3(src, w, hgt, &mut rgba)?;
            for i in 0..w * hgt {
                let stored_alpha = rgba[i * 4 + 3];
                rgba[i * 4] = stored_alpha; // R <- alpha channel
                rgba[i * 4 + 3] = 255; // opaque
            }
        }
        other => {
            return Err(DdsError::Unsupported(format!(
                "{} has no CPU decode path",
                other
            )))
        }
    }

    // Normalize to straight alpha if the source was premultiplied.
    if h.alpha == AlphaMode::Premultiplied {
        for i in 0..w * hgt {
            let a = rgba[i * 4 + 3];
            if a != 0 && a != 255 {
                for c in 0..3 {
                    let v = rgba[i * 4 + c] as u32 * 255 / a as u32;
                    rgba[i * 4 + c] = v.min(255) as u8;
                }
            }
        }
    }

    let out_alpha = if matches!(h.alpha, AlphaMode::Opaque) {
        AlphaMode::Opaque
    } else {
        AlphaMode::Straight
    };

    Ok(DecodedImage {
        width: h.width,
        height: h.height,
        rgba,
        source_format: h.format,
        alpha: out_alpha,
    })
}

fn need(src: &[u8], n: usize) -> Result<(), DdsError> {
    if src.len() < n {
        Err(DdsError::TooShort)
    } else {
        Ok(())
    }
}

// ---- Software BCn decoders (mip 0 only). -------------------------------------

#[inline]
fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 0x1f) as u32;
    let g = ((c >> 5) & 0x3f) as u32;
    let b = (c & 0x1f) as u32;
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}

/// Decode the 4-colour palette + 2-bit indices of a BC1 colour block into the
/// output. `with_1bit_alpha` enables the punch-through transparency case (BC1
/// only); BC2/BC3 always carry alpha separately so they use the opaque path.
fn decode_color_block(blk: &[u8], out: &mut [[u8; 4]; 16], with_1bit_alpha: bool) {
    let c0 = u16::from_le_bytes([blk[0], blk[1]]);
    let c1 = u16::from_le_bytes([blk[2], blk[3]]);
    let p0 = rgb565(c0);
    let p1 = rgb565(c1);
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || !with_1bit_alpha {
        for i in 0..3 {
            pal[2][i] = ((2 * p0[i] as u32 + p1[i] as u32) / 3) as u8;
            pal[3][i] = ((p0[i] as u32 + 2 * p1[i] as u32) / 3) as u8;
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = ((p0[i] as u32 + p1[i] as u32) / 2) as u8;
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0]; // transparent
    }
    let idx = u32::from_le_bytes([blk[4], blk[5], blk[6], blk[7]]);
    for px in 0..16 {
        out[px] = pal[((idx >> (px * 2)) & 0x3) as usize];
    }
}

fn blit_block(dst: &mut [u8], w: usize, h: usize, bx: usize, by: usize, texels: &[[u8; 4]; 16]) {
    for ty in 0..4 {
        for tx in 0..4 {
            let x = bx * 4 + tx;
            let y = by * 4 + ty;
            if x >= w || y >= h {
                continue;
            }
            let t = texels[ty * 4 + tx];
            let o = (y * w + x) * 4;
            dst[o..o + 4].copy_from_slice(&t);
        }
    }
}

fn decode_bc1(src: &[u8], w: usize, h: usize, out: &mut [u8]) -> Result<(), DdsError> {
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    need(src, checked_block_len(w, h, 8)?)?;
    let mut texels = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 8;
            decode_color_block(&src[off..off + 8], &mut texels, true);
            blit_block(out, w, h, bx, by, &texels);
        }
    }
    Ok(())
}

fn decode_bc2(src: &[u8], w: usize, h: usize, out: &mut [u8]) -> Result<(), DdsError> {
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    need(src, checked_block_len(w, h, 16)?)?;
    let mut texels = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            // 8 bytes explicit 4-bit alpha, then an 8-byte BC1 colour block.
            let alpha = &src[off..off + 8];
            decode_color_block(&src[off + 8..off + 16], &mut texels, false);
            for i in 0..16 {
                let nib = (alpha[i / 2] >> ((i % 2) * 4)) & 0x0f;
                texels[i][3] = (nib as u32 * 255 / 15) as u8;
            }
            blit_block(out, w, h, bx, by, &texels);
        }
    }
    Ok(())
}

fn decode_bc3(src: &[u8], w: usize, h: usize, out: &mut [u8]) -> Result<(), DdsError> {
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    need(src, checked_block_len(w, h, 16)?)?;
    let mut texels = [[0u8; 4]; 16];
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let a0 = src[off] as u32;
            let a1 = src[off + 1] as u32;
            let mut atab = [0u32; 8];
            atab[0] = a0;
            atab[1] = a1;
            if a0 > a1 {
                for i in 1..7 {
                    atab[i + 1] = ((7 - i as u32) * a0 + i as u32 * a1) / 7;
                }
            } else {
                for i in 1..5 {
                    atab[i + 1] = ((5 - i as u32) * a0 + i as u32 * a1) / 5;
                }
                atab[6] = 0;
                atab[7] = 255;
            }
            // 48-bit alpha index field (3 bits per texel) after the two endpoints.
            let mut abits: u64 = 0;
            for k in 0..6 {
                abits |= (src[off + 2 + k] as u64) << (8 * k);
            }
            decode_color_block(&src[off + 8..off + 16], &mut texels, false);
            for i in 0..16 {
                let ai = ((abits >> (3 * i)) & 0x7) as usize;
                texels[i][3] = atab[ai] as u8;
            }
            blit_block(out, w, h, bx, by, &texels);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal uncompressed BGRA8 DDS in memory.
    fn synth_bgra8(w: u32, h: u32, px: &[[u8; 4]]) -> Vec<u8> {
        let mut v = vec![0u8; 128];
        v[0..4].copy_from_slice(&DDS_MAGIC.to_le_bytes());
        v[4..8].copy_from_slice(&124u32.to_le_bytes()); // dwSize
        v[8..12].copy_from_slice(&0x0000_1007u32.to_le_bytes()); // flags
        v[12..16].copy_from_slice(&h.to_le_bytes());
        v[16..20].copy_from_slice(&w.to_le_bytes());
        // ddspf @76
        v[76..80].copy_from_slice(&32u32.to_le_bytes()); // pfSize
        v[80..84].copy_from_slice(&(DDPF_RGB | DDPF_ALPHAPIXELS).to_le_bytes());
        // fourcc 0
        v[88..92].copy_from_slice(&32u32.to_le_bytes()); // bits
        v[92..96].copy_from_slice(&0x00ff_0000u32.to_le_bytes()); // R
        v[96..100].copy_from_slice(&0x0000_ff00u32.to_le_bytes()); // G
        v[100..104].copy_from_slice(&0x0000_00ffu32.to_le_bytes()); // B
        v[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes()); // A
        for p in px {
            // stored as B,G,R,A in file order
            v.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        v
    }

    #[test]
    fn bgra8_roundtrip_swizzle() {
        // 2x1: red opaque, green half-alpha
        let dds = synth_bgra8(2, 1, &[[255, 0, 0, 255], [0, 255, 0, 128]]);
        let img = decode(&dds).expect("decode");
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.source_format, DdsFormat::Bgra8);
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.rgba[4..8], &[0, 255, 0, 128]);
    }

    /// Build a FourCC DDS header (DXT1/3/5) wrapping a single 4x4 block payload.
    fn synth_fourcc(fourcc: &[u8; 4], w: u32, h: u32, block: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 128];
        v[0..4].copy_from_slice(&DDS_MAGIC.to_le_bytes());
        v[4..8].copy_from_slice(&124u32.to_le_bytes());
        v[8..12].copy_from_slice(&0x0008_1007u32.to_le_bytes()); // flags w/ LINEARSIZE
        v[12..16].copy_from_slice(&h.to_le_bytes());
        v[16..20].copy_from_slice(&w.to_le_bytes());
        v[76..80].copy_from_slice(&32u32.to_le_bytes()); // pfSize
        v[80..84].copy_from_slice(&DDPF_FOURCC.to_le_bytes());
        v[84..88].copy_from_slice(fourcc);
        v.extend_from_slice(block);
        v
    }

    /// Encode a solid-colour 4x4 BC1 block (both endpoints = the colour, all
    /// indices 0 => every texel is endpoint 0). Proves the CPU BCn fallback.
    #[test]
    fn bc1_cpu_fallback_decodes_solid_color() {
        // RGB565 for pure red (R=31,G=0,B=0) = 0xF800.
        let c: u16 = 0xF800;
        let mut block = [0u8; 8];
        block[0..2].copy_from_slice(&c.to_le_bytes()); // c0
        block[2..4].copy_from_slice(&c.to_le_bytes()); // c1 (== c0 => opaque path)
                                                       // indices all 0 => endpoint 0 everywhere.
        let dds = synth_fourcc(b"DXT1", 4, 4, &block);
        let hdr = parse_header(&dds).unwrap();
        assert_eq!(hdr.format, DdsFormat::Dxt1);
        let img = decode(&dds).expect("bc1 decode");
        assert_eq!((img.width, img.height), (4, 4));
        // every texel should be ~pure red, opaque.
        for px in img.rgba.chunks_exact(4) {
            assert!(px[0] > 250, "R={}", px[0]);
            assert_eq!(px[1], 0);
            assert_eq!(px[2], 0);
            assert_eq!(px[3], 255);
        }
    }

    /// DXT5: solid white colour + graded alpha endpoints. Proves the BC3 alpha
    /// interpolation branch of the CPU fallback (the format wgpu CAN sample, but
    /// the fallback must still exist + be proven, per the M1 spec).
    #[test]
    fn bc3_cpu_fallback_decodes_alpha() {
        let white: u16 = 0xFFFF;
        let mut block = [0u8; 16];
        // alpha endpoints: a0=255, a1=0, a0>a1 => 8-step interpolation.
        block[0] = 255;
        block[1] = 0;
        // alpha indices: all 0 => alpha = a0 = 255 for every texel.
        // (bytes 2..8 already zero)
        // colour block (bytes 8..16): white endpoints, indices 0.
        block[8..10].copy_from_slice(&white.to_le_bytes());
        block[10..12].copy_from_slice(&white.to_le_bytes());
        let dds = synth_fourcc(b"DXT5", 4, 4, &block);
        assert_eq!(parse_header(&dds).unwrap().format, DdsFormat::Dxt5);
        let img = decode(&dds).expect("bc3 decode");
        for px in img.rgba.chunks_exact(4) {
            assert!(px[0] > 250 && px[1] > 250 && px[2] > 250);
            assert_eq!(px[3], 255, "alpha index 0 -> a0=255");
        }
    }

    /// Premultiplied DXT2 must be un-multiplied to straight alpha by the
    /// fallback (no halo): a texel premultiplied at 50% alpha recovers full
    /// colour.
    #[test]
    fn dxt2_premultiplied_is_unmultiplied() {
        // half-bright red premultiplied (0x7800-ish) at alpha 0x8 (~136/255).
        // Use endpoints both = a mid red; alpha nibble = 8 everywhere.
        let red_half: u16 = rgb565_pack(128, 0, 0);
        let mut block = [0u8; 16];
        for i in 0..8 {
            block[i] = 0x88; // every 4-bit alpha nibble = 8 -> a=136
        }
        block[8..10].copy_from_slice(&red_half.to_le_bytes());
        block[10..12].copy_from_slice(&red_half.to_le_bytes());
        let dds = synth_fourcc(b"DXT2", 4, 4, &block);
        let hdr = parse_header(&dds).unwrap();
        assert_eq!(hdr.format, DdsFormat::Dxt2);
        assert_eq!(hdr.alpha, AlphaMode::Premultiplied);
        let img = decode(&dds).expect("dxt2 decode");
        assert_eq!(img.alpha, AlphaMode::Straight);
        for px in img.rgba.chunks_exact(4) {
            // R was ~128 premultiplied at a=136; straightened R ~= 128*255/136 ~= 240.
            assert!(px[0] > px[3], "un-multiply should brighten R above alpha");
            assert_eq!(px[3], 136);
        }
    }

    fn rgb565_pack(r: u8, g: u8, b: u8) -> u16 {
        let r = (r as u16 >> 3) & 0x1f;
        let g = (g as u16 >> 2) & 0x3f;
        let b = (b as u16 >> 3) & 0x1f;
        (r << 11) | (g << 5) | b
    }

    /// BUG B (Codex-confirmed): a malformed DDS declaring enormous dimensions must return an
    /// Err from the size-validation, NEVER overflow `w*h*4` (which wraps in release, off the
    /// overflow-check) into a tiny/garbage allocation followed by an OOB decode loop.
    #[test]
    fn huge_dimensions_error_not_overflow() {
        // Uncompressed BGRA8 header (so decode() takes the uncompressed path) with
        // width=height=0xFFFFFFFF → w*h*4 overflows usize on 64-bit.
        let mut v = synth_bgra8(1, 1, &[[0, 0, 0, 0]]);
        v.truncate(128); // header only; the pixel bytes are irrelevant (we Err before reading)
        v[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // height
        v[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // width
        let r = decode(&v);
        assert!(r.is_err(), "huge dimensions must Err, not overflow/panic");

        // Same for a FourCC (BCn) header through read_gpu_blocks + decode.
        let mut d = synth_fourcc(b"DXT5", 4, 4, &[0u8; 16]);
        d[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        d[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(
            read_gpu_blocks(&d).is_err(),
            "read_gpu_blocks must Err on huge dims"
        );
        assert!(decode(&d).is_err(), "decode must Err on huge BCn dims");
    }

    /// A header declaring valid-but-large dimensions whose pixel data is TRUNCATED returns an
    /// Err (TooShort) rather than reading past the buffer.
    #[test]
    fn truncated_pixel_data_errs() {
        // 4x4 BGRA8 (needs 64 bytes of pixels) but supply none.
        let mut v = vec![0u8; 128];
        v[0..4].copy_from_slice(&DDS_MAGIC.to_le_bytes());
        v[4..8].copy_from_slice(&124u32.to_le_bytes());
        v[8..12].copy_from_slice(&0x0000_1007u32.to_le_bytes());
        v[12..16].copy_from_slice(&4u32.to_le_bytes()); // height 4
        v[16..20].copy_from_slice(&4u32.to_le_bytes()); // width 4
        v[76..80].copy_from_slice(&32u32.to_le_bytes());
        v[80..84].copy_from_slice(&(DDPF_RGB | DDPF_ALPHAPIXELS).to_le_bytes());
        v[88..92].copy_from_slice(&32u32.to_le_bytes());
        v[92..96].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
        v[96..100].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
        v[100..104].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
        v[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
        // no pixel bytes appended → src is empty.
        assert!(matches!(decode(&v), Err(DdsError::TooShort)));
    }

    /// A non-4-aligned BCn texture (e.g. 5x3) decodes without OOB: the block grid is
    /// ceil(5/4)=2 by ceil(3/4)=1 blocks and blit clips to the real 5x3 bounds.
    #[test]
    fn non_aligned_bcn_dimensions_decode() {
        // 2x1 = 2 blocks of BC1 (8 bytes each) for a 5x3 image → 16 bytes.
        let c: u16 = 0xF800; // red
        let mut blocks = vec![0u8; 16];
        for blk in 0..2 {
            blocks[blk * 8..blk * 8 + 2].copy_from_slice(&c.to_le_bytes());
            blocks[blk * 8 + 2..blk * 8 + 4].copy_from_slice(&c.to_le_bytes());
        }
        let dds = synth_fourcc(b"DXT1", 5, 3, &blocks);
        let img = decode(&dds).expect("non-aligned BC1 decode");
        assert_eq!((img.width, img.height), (5, 3));
        assert_eq!(img.rgba.len(), 5 * 3 * 4);
        // top-left texel is red.
        assert!(img.rgba[0] > 250 && img.rgba[1] == 0 && img.rgba[2] == 0);
    }

    /// 16-bit X1R5G5B5 (the game generic-portrait format). Classify + decode.
    #[test]
    fn rgb555_16bit_decodes() {
        let mut v = vec![0u8; 128];
        v[0..4].copy_from_slice(&DDS_MAGIC.to_le_bytes());
        v[4..8].copy_from_slice(&124u32.to_le_bytes());
        v[8..12].copy_from_slice(&0x0000_1007u32.to_le_bytes());
        v[12..16].copy_from_slice(&1u32.to_le_bytes());
        v[16..20].copy_from_slice(&2u32.to_le_bytes());
        v[76..80].copy_from_slice(&32u32.to_le_bytes());
        v[80..84].copy_from_slice(&DDPF_RGB.to_le_bytes()); // RGB, no alpha bit
        v[88..92].copy_from_slice(&16u32.to_le_bytes()); // bits=16
        v[92..96].copy_from_slice(&0x7c00u32.to_le_bytes()); // R
        v[96..100].copy_from_slice(&0x03e0u32.to_le_bytes()); // G
        v[100..104].copy_from_slice(&0x001fu32.to_le_bytes()); // B
                                                               // 2 texels: pure red (0x7c00), pure blue (0x001f)
        v.extend_from_slice(&0x7c00u16.to_le_bytes());
        v.extend_from_slice(&0x001fu16.to_le_bytes());
        let hdr = parse_header(&v).unwrap();
        assert_eq!(hdr.format, DdsFormat::Rgb555);
        let img = decode(&v).unwrap();
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]); // red, opaque
        assert_eq!(&img.rgba[4..8], &[0, 0, 255, 255]); // blue, opaque
    }
}
