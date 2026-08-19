use crate::*;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::Path;

pub(crate) const MAX_REPORTED_DEFINITION_GAPS: usize = 20;

pub(crate) fn load_definition_csv(
    path: &Path,
) -> Result<(BTreeMap<u32, ProvinceDefinition>, HashMap<u32, u32>)> {
    let text = read_latin1(path)?;
    decode_definition_csv(&text, path)
}

pub(crate) fn decode_definition_csv(
    text: &str,
    path: &Path,
) -> Result<(BTreeMap<u32, ProvinceDefinition>, HashMap<u32, u32>)> {
    let mut definitions: BTreeMap<u32, ProvinceDefinition> = BTreeMap::new();
    let mut color_to_id: HashMap<u32, u32> = HashMap::new();
    let mut id_lines: HashMap<u32, usize> = HashMap::new();
    let mut color_lines: HashMap<u32, usize> = HashMap::new();
    let mut gaps: Vec<String> = Vec::new();
    let mut gap_count = 0usize;
    let note = |gaps: &mut Vec<String>, gap_count: &mut usize, message: String| {
        *gap_count += 1;
        if gaps.len() < MAX_REPORTED_DEFINITION_GAPS {
            gaps.push(message);
        }
    };
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        let raw = if index == 0 {
            raw.trim_start_matches('\u{feff}')
                .trim_start_matches("\u{ef}\u{bb}\u{bf}")
        } else {
            raw
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts = trimmed.split(';').collect::<Vec<_>>();
        if parts.len() < 8 {
            note(
                &mut gaps,
                &mut gap_count,
                format!(
                    "line {line_no}: expected at least 8 semicolon-separated fields, found {}",
                    parts.len()
                ),
            );
            continue;
        }
        let Ok(id) = parts[0].parse::<u32>() else {
            note(
                &mut gaps,
                &mut gap_count,
                format!("line {line_no}: province id {:?} is not a u32", parts[0]),
            );
            continue;
        };
        let mut channel = |field: usize, name: &str| match parts[field].parse::<u8>() {
            Ok(value) => Some(value),
            Err(_) => {
                note(
                    &mut gaps,
                    &mut gap_count,
                    format!(
                        "line {line_no}: province {id} {name} channel {:?} is not a u8",
                        parts[field]
                    ),
                );
                None
            }
        };
        let r = channel(1, "red");
        let g = channel(2, "green");
        let b = channel(3, "blue");
        let (Some(r), Some(g), Some(b)) = (r, g, b) else {
            continue;
        };
        let Ok(continent) = parts[7].parse::<i32>() else {
            note(
                &mut gaps,
                &mut gap_count,
                format!(
                    "line {line_no}: province {id} continent {:?} is not an i32",
                    parts[7]
                ),
            );
            continue;
        };
        let key = color_key(r, g, b);
        if let Some(previous) = id_lines.get(&id) {
            note(
                &mut gaps,
                &mut gap_count,
                format!("line {line_no}: province id {id} already defined on line {previous}"),
            );
            continue;
        }
        if let Some(previous) = color_lines.get(&key) {
            note(
                &mut gaps,
                &mut gap_count,
                format!(
                    "line {line_no}: province {id} RGB ({r},{g},{b}) already claimed on line {previous}"
                ),
            );
            continue;
        }
        id_lines.insert(id, line_no);
        color_lines.insert(key, line_no);
        color_to_id.insert(key, id);
        definitions.insert(
            id,
            ProvinceDefinition {
                rgb: [r, g, b],
                province_type: parts[4].to_string(),
                coastal: parts[5].eq_ignore_ascii_case("true"),
                terrain: parts[6].to_string(),
                continent,
            },
        );
    }
    if gap_count != 0 {
        let elided = gap_count.saturating_sub(gaps.len());
        let suffix = if elided == 0 {
            String::new()
        } else {
            format!("; and {elided} more")
        };
        return Err(format!(
            "{} is malformed: {gap_count} unusable row(s) — {}{suffix}",
            path.display(),
            gaps.join("; ")
        )
        .into());
    }
    if definitions.len() != color_to_id.len() {
        return Err(format!(
            "{} produced {} province definitions but {} color keys",
            path.display(),
            definitions.len(),
            color_to_id.len()
        )
        .into());
    }
    Ok((definitions, color_to_id))
}

pub(crate) fn color_key(r: u8, g: u8, b: u8) -> u32 {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}

pub(crate) const MAX_IMAGE_DIMENSION: usize = 32_768;

pub(crate) fn check_image_dimensions(path: &Path, width: usize, height: usize) -> Result<()> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(format!(
            "{} has unsupported image dimensions {width}x{height} (limit {MAX_IMAGE_DIMENSION})",
            path.display()
        )
        .into());
    }
    Ok(())
}

pub(crate) struct BmpIds {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) ids: Vec<u32>,
    pub(crate) unknown_pixels: usize,
}

pub(crate) const MAX_REPORTED_UNKNOWN_COLORS: usize = 8;

pub(crate) fn load_provinces_bmp(path: &Path, color_to_id: &HashMap<u32, u32>) -> Result<BmpIds> {
    let data = fs::read(path)?;
    decode_provinces_bmp(&data, path, color_to_id)
}

pub(crate) fn decode_provinces_bmp(
    data: &[u8],
    path: &Path,
    color_to_id: &HashMap<u32, u32>,
) -> Result<BmpIds> {
    if data.get(0..2) != Some(b"BM") {
        return Err("not a BMP file".into());
    }
    let pixel_offset = le_u32(&data, 10)? as usize;
    let width_raw = le_i32(&data, 18)?;
    let height_raw = le_i32(&data, 22)?;
    let bpp = le_u16(&data, 28)?;
    let compression = le_u32(&data, 30)?;
    if width_raw <= 0 || height_raw == 0 {
        return Err("unsupported BMP dimensions".into());
    }
    if bpp != 24 || compression != 0 {
        return Err(format!(
            "expected 24bpp uncompressed BMP, got bpp={bpp} compression={compression}"
        )
        .into());
    }
    let width = width_raw as usize;
    let height = height_raw.unsigned_abs() as usize;
    check_image_dimensions(path, width, height)?;
    let bottom_up = height_raw > 0;
    let stride = (width * 3 + 3) & !3;
    let needed = pixel_offset + stride * height;
    if data.len() < needed {
        return Err("BMP is truncated".into());
    }

    let mut ids = vec![0u32; width * height];
    let mut unknown_pixels = 0usize;
    let mut unknown_colors: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for y in 0..height {
        let source_y = if bottom_up { height - 1 - y } else { y };
        let row = pixel_offset + source_y * stride;
        for x in 0..width {
            let i = row + x * 3;
            let b = data[i];
            let g = data[i + 1];
            let r = data[i + 2];
            let key = color_key(r, g, b);
            match color_to_id.get(&key) {
                Some(id) => ids[y * width + x] = *id,
                None => {
                    unknown_pixels += 1;
                    if unknown_colors.len() < MAX_REPORTED_UNKNOWN_COLORS
                        || unknown_colors.contains_key(&key)
                    {
                        unknown_colors.entry(key).or_insert((x, y));
                    }
                }
            }
        }
    }
    if unknown_pixels != 0 {
        let samples = unknown_colors
            .iter()
            .map(|(key, (x, y))| {
                format!(
                    "RGB({},{},{}) first at ({x},{y})",
                    key & 0xff,
                    (key >> 8) & 0xff,
                    (key >> 16) & 0xff
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "{} has {unknown_pixels} unknown pixel(s) whose color has no definition.csv province; \
             province id 0 is the \"no province\" sentinel, so these cannot be admitted: {samples}",
            path.display()
        )
        .into());
    }

    Ok(BmpIds {
        width,
        height,
        ids,
        unknown_pixels,
    })
}

pub(crate) fn load_bmp_indexed(path: &Path) -> Result<IndexImage> {
    let data = fs::read(path)?;
    decode_bmp_indexed(&data, path)
}

pub(crate) fn decode_bmp_indexed(data: &[u8], path: &Path) -> Result<IndexImage> {
    if data.get(0..2) != Some(b"BM") {
        return Err(format!("{} is not a BMP file", path.display()).into());
    }
    let pixel_offset = le_u32(&data, 10)? as usize;
    let width_raw = le_i32(&data, 18)?;
    let height_raw = le_i32(&data, 22)?;
    let bpp = le_u16(&data, 28)?;
    let compression = le_u32(&data, 30)?;
    if width_raw <= 0 || height_raw == 0 {
        return Err("unsupported indexed BMP dimensions".into());
    }
    if bpp != 8 || compression != 0 {
        return Err(format!(
            "expected 8bpp uncompressed BMP for {}, got bpp={bpp} compression={compression}",
            path.display()
        )
        .into());
    }
    let width = width_raw as usize;
    let height = height_raw.unsigned_abs() as usize;
    check_image_dimensions(path, width, height)?;
    let bottom_up = height_raw > 0;
    let stride = (width + 3) & !3;
    let needed = pixel_offset + stride * height;
    if data.len() < needed {
        return Err(format!("{} is truncated", path.display()).into());
    }
    let mut pixels = vec![0u8; width * height];
    for y in 0..height {
        let source_y = if bottom_up { height - 1 - y } else { y };
        let row = pixel_offset + source_y * stride;
        pixels[y * width..(y + 1) * width].copy_from_slice(&data[row..row + width]);
    }
    Ok(IndexImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn load_bmp_rgbx(path: &Path) -> Result<RgbxImage> {
    let data = fs::read(path)?;
    decode_bmp_rgbx(&data, path)
}

pub(crate) fn decode_bmp_rgbx(data: &[u8], path: &Path) -> Result<RgbxImage> {
    if data.get(0..2) != Some(b"BM") {
        return Err(format!("{} is not a BMP file", path.display()).into());
    }
    let pixel_offset = le_u32(&data, 10)? as usize;
    let width_raw = le_i32(&data, 18)?;
    let height_raw = le_i32(&data, 22)?;
    let bpp = le_u16(&data, 28)? as usize;
    let compression = le_u32(&data, 30)?;
    if width_raw <= 0 || height_raw == 0 {
        return Err("unsupported RGB BMP dimensions".into());
    }
    if (bpp != 24 && bpp != 32) || compression != 0 {
        return Err(format!(
            "expected 24/32bpp uncompressed BMP for {}, got bpp={bpp} compression={compression}",
            path.display()
        )
        .into());
    }
    let width = width_raw as usize;
    let height = height_raw.unsigned_abs() as usize;
    check_image_dimensions(path, width, height)?;
    let bottom_up = height_raw > 0;
    let bytes_per_pixel = bpp / 8;
    let stride = (width * bytes_per_pixel + 3) & !3;
    let needed = pixel_offset + stride * height;
    if data.len() < needed {
        return Err(format!("{} is truncated", path.display()).into());
    }
    let mut pixels = vec![0u32; width * height];
    for y in 0..height {
        let source_y = if bottom_up { height - 1 - y } else { y };
        let row = pixel_offset + source_y * stride;
        for x in 0..width {
            let offset = row + x * bytes_per_pixel;
            let b = data[offset];
            let g = data[offset + 1];
            let r = data[offset + 2];
            pixels[y * width + x] = rgb_u32([r, g, b]);
        }
    }
    Ok(RgbxImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn load_dds_rgbx(path: &Path) -> Result<RgbxImage> {
    Ok(rgba_to_rgbx(load_dds_rgba(path)?))
}

pub(crate) fn load_dds_rgba(path: &Path) -> Result<RgbaImage> {
    let data = fs::read(path)?;
    if data.get(0..4) != Some(b"DDS ") {
        return Err(format!("{} is not a DDS file", path.display()).into());
    }
    let height = le_u32(&data, 12)? as usize;
    let width = le_u32(&data, 16)? as usize;
    check_image_dimensions(path, width, height)?;
    let pf_flags = le_u32(&data, 80)?;
    let fourcc = data.get(84..88).ok_or("truncated DDS pixel format")?;
    let rgb_bits = le_u32(&data, 88)? as usize;

    decode_dds_rgba_mip(path, &data, width, height, 128, pf_flags, fourcc, rgb_bits)
}

pub(crate) fn load_dds_rgba_mips(path: &Path) -> Result<Vec<RgbaImage>> {
    let data = fs::read(path)?;
    if data.get(0..4) != Some(b"DDS ") {
        return Err(format!("{} is not a DDS file", path.display()).into());
    }
    let height = le_u32(&data, 12)? as usize;
    let width = le_u32(&data, 16)? as usize;
    check_image_dimensions(path, width, height)?;
    let mip_count = le_u32(&data, 28)?.max(1) as usize;
    let pf_flags = le_u32(&data, 80)?;
    let fourcc = data.get(84..88).ok_or("truncated DDS pixel format")?;
    let rgb_bits = le_u32(&data, 88)? as usize;
    let mut offset = 128usize;
    let mut levels = Vec::with_capacity(mip_count);
    for level in 0..mip_count {
        let mip_width = (width >> level).max(1);
        let mip_height = (height >> level).max(1);
        levels.push(decode_dds_rgba_mip(
            path, &data, mip_width, mip_height, offset, pf_flags, fourcc, rgb_bits,
        )?);
        offset = offset
            .checked_add(dds_mip_byte_len(mip_width, mip_height, fourcc, rgb_bits)?)
            .ok_or_else(|| format!("{} DDS mip offset overflowed", path.display()))?;
    }
    if levels.is_empty() {
        return Err(format!("{} contains no DDS mip levels", path.display()).into());
    }
    Ok(levels)
}

pub(crate) fn decode_dds_rgba_mip(
    path: &Path,
    data: &[u8],
    width: usize,
    height: usize,
    offset: usize,
    pf_flags: u32,
    fourcc: &[u8],
    rgb_bits: usize,
) -> Result<RgbaImage> {
    if fourcc == b"DXT1" {
        return decode_dds_dxt1_rgba(path, data, width, height, offset);
    }

    if fourcc == b"DXT5" {
        return decode_dds_dxt5_rgba(path, data, width, height, offset);
    }

    if pf_flags & 0x40 == 0 || rgb_bits == 0 {
        return Err(format!("unsupported DDS pixel format in {}", path.display()).into());
    }
    let rmask = le_u32(data, 92)?;
    let gmask = le_u32(data, 96)?;
    let bmask = le_u32(data, 100)?;
    let amask = le_u32(data, 104)?;
    let bytes_per_pixel = rgb_bits / 8;
    if !(1..=4).contains(&bytes_per_pixel) {
        return Err(format!("unsupported DDS rgb bit depth {rgb_bits}").into());
    }
    let needed = width
        .checked_mul(height)
        .and_then(|pixel_count| pixel_count.checked_mul(bytes_per_pixel))
        .and_then(|byte_len| byte_len.checked_add(offset))
        .ok_or_else(|| format!("{} DDS dimensions overflow byte length", path.display()))?;
    if data.len() < needed {
        return Err(format!("{} is truncated", path.display()).into());
    }
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .ok_or_else(|| format!("{} DDS dimensions overflow pixel buffer", path.display()))?;
    let mut pixels = vec![0u8; pixel_bytes];
    for i in 0..width * height {
        let start = offset + i * bytes_per_pixel;
        let mut raw = 0u32;
        for b in 0..bytes_per_pixel {
            raw |= u32::from(data[start + b]) << (8 * b);
        }
        let out = i * 4;
        pixels[out] = mask_channel(raw, rmask);
        pixels[out + 1] = mask_channel(raw, gmask);
        pixels[out + 2] = mask_channel(raw, bmask);
        pixels[out + 3] = if amask == 0 {
            255
        } else {
            mask_channel(raw, amask)
        };
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn dds_mip_byte_len(
    width: usize,
    height: usize,
    fourcc: &[u8],
    rgb_bits: usize,
) -> Result<usize> {
    if fourcc == b"DXT1" {
        return width
            .div_ceil(4)
            .checked_mul(height.div_ceil(4))
            .and_then(|blocks| blocks.checked_mul(8))
            .ok_or_else(|| {
                format!("DDS mip dimensions overflow byte length: {width}x{height}").into()
            });
    }
    if fourcc == b"DXT5" {
        return width
            .div_ceil(4)
            .checked_mul(height.div_ceil(4))
            .and_then(|blocks| blocks.checked_mul(16))
            .ok_or_else(|| {
                format!("DDS mip dimensions overflow byte length: {width}x{height}").into()
            });
    }
    let bytes_per_pixel = rgb_bits / 8;
    if !(1..=4).contains(&bytes_per_pixel) {
        return Err(format!("unsupported DDS rgb bit depth {rgb_bits}").into());
    }
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| format!("DDS mip dimensions overflow byte length: {width}x{height}").into())
}

pub(crate) fn rgba_to_rgbx(image: RgbaImage) -> RgbxImage {
    let mut pixels = Vec::with_capacity(image.width * image.height);
    for px in image.pixels.chunks_exact(4) {
        pixels.push(rgb_u32([px[0], px[1], px[2]]));
    }
    RgbxImage {
        width: image.width,
        height: image.height,
        pixels,
    }
}

pub(crate) fn dds_dimensions(path: &Path) -> Result<Dimensions> {
    Ok(dds_info(path)?.dimensions)
}

pub(crate) fn dds_info(path: &Path) -> Result<DdsInfo> {
    let mut header = [0u8; 20];
    let mut full_header = [0u8; 128];
    fs::File::open(path)?.read_exact(&mut full_header)?;
    header.copy_from_slice(&full_header[..20]);
    if header.get(0..4) != Some(b"DDS ") {
        return Err(format!("{} is not a DDS file", path.display()).into());
    }
    let fourcc = &full_header[84..88];
    let rgb_bits = u32::from_le_bytes(full_header[88..92].try_into()?);
    let format = if fourcc.iter().any(|byte| *byte != 0) {
        String::from_utf8_lossy(fourcc).to_string()
    } else if rgb_bits > 0 {
        format!("RGB{rgb_bits}")
    } else {
        "UNKNOWN".to_string()
    };
    let mip_count = u32::from_le_bytes(full_header[28..32].try_into()?).max(1);
    Ok(DdsInfo {
        dimensions: Dimensions {
            height: u32::from_le_bytes(header[12..16].try_into()?) as usize,
            width: u32::from_le_bytes(header[16..20].try_into()?) as usize,
        },
        format,
        mip_count,
    })
}

/// `value * 255` (wide masks), which is a release panic under this crate's
pub(crate) fn mask_channel(raw: u32, mask: u32) -> u8 {
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

pub(crate) fn decode_dds_dxt5_rgba(
    path: &Path,
    data: &[u8],
    width: usize,
    height: usize,
    offset: usize,
) -> Result<RgbaImage> {
    let block_w = width.div_ceil(4);
    let block_h = height.div_ceil(4);
    let needed = block_w
        .checked_mul(block_h)
        .and_then(|blocks| blocks.checked_mul(16))
        .and_then(|byte_len| byte_len.checked_add(offset))
        .ok_or_else(|| format!("{} DXT5 dimensions overflow byte length", path.display()))?;
    if data.len() < needed {
        return Err(format!("{} is truncated", path.display()).into());
    }
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .ok_or_else(|| format!("{} DXT5 dimensions overflow pixel buffer", path.display()))?;
    let mut pixels = vec![0u8; pixel_bytes];
    for by in 0..block_h {
        for bx in 0..block_w {
            let block = offset + (by * block_w + bx) * 16;
            let alpha_palette = dxt5_alpha_palette(data[block], data[block + 1]);
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
            let palette = dxt_color_palette(color0, color1);
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= width || y >= height {
                        continue;
                    }
                    let pos = py * 4 + px;
                    let code = ((codes >> (2 * pos)) & 0x3) as usize;
                    let alpha_code = ((alpha_codes >> (3 * pos)) & 0x7) as usize;
                    let dst = (y * width + x) * 4;
                    pixels[dst] = palette[code][0];
                    pixels[dst + 1] = palette[code][1];
                    pixels[dst + 2] = palette[code][2];
                    pixels[dst + 3] = alpha_palette[alpha_code];
                }
            }
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn dxt5_alpha_palette(alpha0: u8, alpha1: u8) -> [u8; 8] {
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

pub(crate) fn decode_dds_dxt1_rgba(
    path: &Path,
    data: &[u8],
    width: usize,
    height: usize,
    offset: usize,
) -> Result<RgbaImage> {
    let block_w = width.div_ceil(4);
    let block_h = height.div_ceil(4);
    let needed = block_w
        .checked_mul(block_h)
        .and_then(|blocks| blocks.checked_mul(8))
        .and_then(|byte_len| byte_len.checked_add(offset))
        .ok_or_else(|| format!("{} DXT1 dimensions overflow byte length", path.display()))?;
    if data.len() < needed {
        return Err(format!("{} is truncated", path.display()).into());
    }
    let pixel_bytes = width
        .checked_mul(height)
        .and_then(|pixel_count| pixel_count.checked_mul(4))
        .ok_or_else(|| format!("{} DXT1 dimensions overflow pixel buffer", path.display()))?;
    let mut pixels = vec![0u8; pixel_bytes];
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
            let palette = dxt1_color_palette(color0, color1);
            let alpha = dxt1_alpha_palette(color0, color1);
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
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

pub(crate) fn dxt1_alpha_palette(color0: u16, color1: u16) -> [u8; 4] {
    if color0 > color1 {
        [255, 255, 255, 255]
    } else {
        [255, 255, 255, 0]
    }
}

pub(crate) fn dxt1_color_palette(color0: u16, color1: u16) -> [[u8; 3]; 4] {
    let c0 = rgb565(color0);
    let c1 = rgb565(color1);
    if color0 > color1 {
        dxt_color_palette(color0, color1)
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

pub(crate) fn dxt_color_palette(color0: u16, color1: u16) -> [[u8; 3]; 4] {
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

pub(crate) fn rgb565(value: u16) -> [u8; 3] {
    let r = u32::from((value >> 11) & 0x1F);
    let g = u32::from((value >> 5) & 0x3F);
    let b = u32::from(value & 0x1F);
    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
    ]
}

pub(crate) fn le_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes: [u8; 2] = data
        .get(offset..offset + 2)
        .ok_or("truncated little-endian u16")?
        .try_into()?;
    Ok(u16::from_le_bytes(bytes))
}

pub(crate) fn le_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or("truncated little-endian u32")?
        .try_into()?;
    Ok(u32::from_le_bytes(bytes))
}

pub(crate) fn le_i32(data: &[u8], offset: usize) -> Result<i32> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or("truncated little-endian i32")?
        .try_into()?;
    Ok(i32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fake_path() -> PathBuf {
        PathBuf::from("corrupt_asset.dds")
    }

    #[test]
    fn dimension_cap_rejects_zero_and_oversized() {
        let path = fake_path();
        assert!(check_image_dimensions(&path, 0, 64).is_err());
        assert!(check_image_dimensions(&path, 64, 0).is_err());
        assert!(check_image_dimensions(&path, MAX_IMAGE_DIMENSION + 1, 64).is_err());
        assert!(check_image_dimensions(&path, 64, MAX_IMAGE_DIMENSION + 1).is_err());
        assert!(check_image_dimensions(&path, MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION).is_ok());
        assert!(check_image_dimensions(&path, 1, 1).is_ok());
    }

    #[test]
    fn mask_channel_is_total_for_every_mask_shape() {
        // release panics under overflow-checks instead of a decode.
        assert_eq!(mask_channel(u32::MAX, u32::MAX), 255);
        assert_eq!(mask_channel(0, u32::MAX), 0);
        assert_eq!(mask_channel(0x8000_0000, 0xffff_ffff), 128);
        assert_eq!(mask_channel(0xdead_beef, 0), 0);
        assert_eq!(mask_channel(0x00ff_0000, 0x00ff_0000), 255);
        assert_eq!(mask_channel(0x0000_0000, 0x00ff_0000), 0);
    }

    #[test]
    fn full_channel_mask_dds_decodes_instead_of_panicking() {
        let path = fake_path();
        let mut data = vec![0u8; 132];
        data[92..96].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        data[96..100].copy_from_slice(&0u32.to_le_bytes());
        data[100..104].copy_from_slice(&0u32.to_le_bytes());
        data[104..108].copy_from_slice(&0u32.to_le_bytes());
        data[128..132].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        let image = decode_dds_rgba_mip(&path, &data, 1, 1, 128, 0x40, b"NONE", 32)
            .expect("a full 32-bit channel mask is decodable, not a panic");
        assert_eq!(image.pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn uncompressed_mip_dimension_overflow_is_typed_error() {
        let path = fake_path();
        let data = vec![0u8; 128];
        let result = decode_dds_rgba_mip(&path, &data, usize::MAX / 2, 3, 128, 0x40, b"NONE", 32);
        let err = result.err().expect("overflowing dimensions must refuse");
        assert!(err.to_string().contains("overflow"), "{err}");
    }

    #[test]
    fn dxt_decoder_dimension_overflow_is_typed_error() {
        let path = fake_path();
        let data = vec![0u8; 16];
        let dxt1 = decode_dds_dxt1_rgba(&path, &data, usize::MAX - 3, usize::MAX - 3, 0);
        assert!(dxt1
            .err()
            .expect("DXT1 must refuse")
            .to_string()
            .contains("overflow"));
        let dxt5 = decode_dds_dxt5_rgba(&path, &data, usize::MAX - 3, usize::MAX - 3, 0);
        assert!(dxt5
            .err()
            .expect("DXT5 must refuse")
            .to_string()
            .contains("overflow"));
    }

    #[test]
    fn dds_mip_byte_len_dxt_overflow_is_typed_error() {
        assert!(dds_mip_byte_len(usize::MAX / 2, usize::MAX / 2, b"DXT1", 0).is_err());
        assert!(dds_mip_byte_len(usize::MAX / 2, usize::MAX / 2, b"DXT5", 0).is_err());
        assert_eq!(dds_mip_byte_len(8, 8, b"DXT1", 0).unwrap(), 32);
        assert_eq!(dds_mip_byte_len(8, 8, b"DXT5", 0).unwrap(), 64);
        assert_eq!(dds_mip_byte_len(1, 1, b"DXT1", 0).unwrap(), 8);
    }


    fn definition_csv(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create temp dir");
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write definition.csv fixture");
        path
    }

    #[test]
    fn definition_csv_accepts_well_formed_rows_blank_lines_and_a_trailing_field() {
        let dir = crate::test_support::temp_test_dir("definition_csv_ok");
        let path = definition_csv(
            &dir,
            "definition.csv",
            "0;0;0;0;land;false;unknown;0\n\
             1;10;20;30;land;true;plains;1\n\
             \n\
             2;40;50;60;sea;false;ocean;-1;trailing-extra-field\n\
             \n",
        );
        let (definitions, color_to_id) = load_definition_csv(&path).expect("strict parse");
        assert_eq!(definitions.len(), 3);
        assert_eq!(color_to_id.len(), 3);
        assert_eq!(color_to_id.get(&color_key(10, 20, 30)), Some(&1));
        let one = definitions.get(&1).expect("province 1");
        assert_eq!(one.rgb, [10, 20, 30]);
        assert!(one.coastal);
        assert_eq!(one.terrain, "plains");
        assert_eq!(one.continent, 1);
        assert_eq!(definitions.get(&2).expect("province 2").continent, -1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn definition_csv_tolerates_a_utf8_bom_on_the_first_row() {
        let dir = crate::test_support::temp_test_dir("definition_csv_bom");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("definition.csv");
        let mut bytes = vec![0xefu8, 0xbb, 0xbf];
        bytes.extend_from_slice(b"0;0;0;0;land;false;unknown;0\n1;1;2;3;land;false;plains;0\n");
        std::fs::write(&path, bytes).expect("write BOM fixture");
        let (definitions, color_to_id) = load_definition_csv(&path).expect("BOM tolerated");
        assert_eq!(definitions.len(), 2);
        assert!(definitions.contains_key(&0));
        assert_eq!(color_to_id.get(&color_key(0, 0, 0)), Some(&0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn definition_csv_refuses_short_row() {
        let dir = crate::test_support::temp_test_dir("definition_csv_short");
        let path = definition_csv(&dir, "definition.csv", "1;10;20;30;land;true\n");
        let err = load_definition_csv(&path).expect_err("short row must refuse");
        let message = err.to_string();
        assert!(message.contains("line 1"), "{message}");
        assert!(message.contains("at least 8"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn definition_csv_refuses_unparseable_fields_with_line_context() {
        let dir = crate::test_support::temp_test_dir("definition_csv_fields");
        let bad_id = definition_csv(&dir, "id.csv", "x;1;2;3;land;false;plains;0\n");
        let message = load_definition_csv(&bad_id).unwrap_err().to_string();
        assert!(message.contains("line 1"), "{message}");
        assert!(message.contains("not a u32"), "{message}");

        let bad_channel = definition_csv(&dir, "rgb.csv", "1;1;999;3;land;false;plains;0\n");
        let message = load_definition_csv(&bad_channel).unwrap_err().to_string();
        assert!(message.contains("green channel"), "{message}");
        assert!(message.contains("not a u8"), "{message}");

        let bad_continent = definition_csv(&dir, "cont.csv", "1;1;2;3;land;false;plains;x\n");
        let message = load_definition_csv(&bad_continent).unwrap_err().to_string();
        assert!(message.contains("continent"), "{message}");
        assert!(message.contains("not an i32"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn definition_csv_refuses_duplicate_ids_and_duplicate_colors() {
        let dir = crate::test_support::temp_test_dir("definition_csv_dupes");
        let dup_id = definition_csv(
            &dir,
            "id.csv",
            "1;1;2;3;land;false;plains;0\n1;4;5;6;land;false;plains;0\n",
        );
        let message = load_definition_csv(&dup_id).unwrap_err().to_string();
        assert!(
            message.contains("province id 1 already defined on line 1"),
            "{message}"
        );

        let dup_color = definition_csv(
            &dir,
            "color.csv",
            "1;1;2;3;land;false;plains;0\n2;1;2;3;land;false;plains;0\n",
        );
        let message = load_definition_csv(&dup_color).unwrap_err().to_string();
        assert!(
            message.contains("RGB (1,2,3) already claimed on line 1"),
            "{message}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn definition_csv_refusal_bounds_the_enumeration_but_not_the_count() {
        let dir = crate::test_support::temp_test_dir("definition_csv_many");
        let mut body = String::new();
        for index in 0..MAX_REPORTED_DEFINITION_GAPS + 3 {
            body.push_str(&format!("bad-{index};1;2;3;land;false;plains;0\n"));
        }
        let path = definition_csv(&dir, "definition.csv", &body);
        let message = load_definition_csv(&path).unwrap_err().to_string();
        assert!(
            message.contains(&format!(
                "{} unusable row(s)",
                MAX_REPORTED_DEFINITION_GAPS + 3
            )),
            "{message}"
        );
        assert!(message.contains("and 3 more"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }


    fn write_provinces_bmp(path: &std::path::Path, width: usize, pixels: &[[u8; 3]]) {
        let height = pixels.len() / width;
        let stride = (width * 3 + 3) & !3;
        let pixel_offset = 54usize;
        let mut data = vec![0u8; pixel_offset + stride * height];
        data[0..2].copy_from_slice(b"BM");
        let total = u32::try_from(data.len()).expect("bmp size");
        data[2..6].copy_from_slice(&total.to_le_bytes());
        data[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes());
        data[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        data[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        data[26..28].copy_from_slice(&1u16.to_le_bytes());
        data[28..30].copy_from_slice(&24u16.to_le_bytes());
        for y in 0..height {
            let source_y = height - 1 - y;
            let row = pixel_offset + source_y * stride;
            for x in 0..width {
                let [r, g, b] = pixels[y * width + x];
                data[row + x * 3] = b;
                data[row + x * 3 + 1] = g;
                data[row + x * 3 + 2] = r;
            }
        }
        std::fs::write(path, data).expect("write provinces.bmp fixture");
    }

    #[test]
    fn provinces_bmp_loads_when_every_color_is_defined() {
        let dir = crate::test_support::temp_test_dir("provinces_bmp_ok");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("provinces.bmp");
        write_provinces_bmp(&path, 2, &[[1, 0, 0], [2, 0, 0], [1, 0, 0], [2, 0, 0]]);
        let mut color_to_id = HashMap::new();
        color_to_id.insert(color_key(1, 0, 0), 11u32);
        color_to_id.insert(color_key(2, 0, 0), 22u32);
        let bmp = load_provinces_bmp(&path, &color_to_id).expect("all colors defined");
        assert_eq!((bmp.width, bmp.height), (2, 2));
        assert_eq!(bmp.ids, vec![11, 22, 11, 22]);
        assert_eq!(bmp.unknown_pixels, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provinces_bmp_refuses_unknown_color_instead_of_substituting_zero() {
        let dir = crate::test_support::temp_test_dir("provinces_bmp_unknown");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("provinces.bmp");
        write_provinces_bmp(&path, 2, &[[1, 0, 0], [9, 9, 9], [1, 0, 0], [1, 0, 0]]);
        let mut color_to_id = HashMap::new();
        color_to_id.insert(color_key(1, 0, 0), 11u32);
        let message = match load_provinces_bmp(&path, &color_to_id) {
            Ok(_) => panic!("unknown color must refuse"),
            Err(error) => error.to_string(),
        };
        assert!(message.contains("1 unknown pixel(s)"), "{message}");
        assert!(message.contains("RGB(9,9,9) first at (1,0)"), "{message}");
        assert!(message.contains("no province"), "{message}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provinces_bmp_unknown_color_sample_list_is_bounded() {
        let dir = crate::test_support::temp_test_dir("provinces_bmp_many_unknown");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("provinces.bmp");
        let width = MAX_REPORTED_UNKNOWN_COLORS + 4;
        let pixels = (0..width)
            .map(|index| [u8::try_from(index + 1).expect("channel"), 0, 0])
            .collect::<Vec<_>>();
        write_provinces_bmp(&path, width, &pixels);
        let message = match load_provinces_bmp(&path, &HashMap::new()) {
            Ok(_) => panic!("unknown colors must refuse"),
            Err(error) => error.to_string(),
        };
        assert!(
            message.contains(&format!("{width} unknown pixel(s)")),
            "{message}"
        );
        assert_eq!(
            message.matches("first at").count(),
            MAX_REPORTED_UNKNOWN_COLORS,
            "{message}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dds_header_with_oversized_dimensions_is_refused() {
        let dir = std::env::temp_dir().join(format!("image_load_cap_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("oversized.dds");
        let mut header = vec![0u8; 128];
        header[0..4].copy_from_slice(b"DDS ");
        header[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        header[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &header).unwrap();
        let single = load_dds_rgba(&path);
        assert!(single
            .err()
            .expect("oversized DDS must refuse")
            .to_string()
            .contains("unsupported image dimensions"));
        let mips = load_dds_rgba_mips(&path);
        assert!(mips
            .err()
            .expect("oversized DDS mips must refuse")
            .to_string()
            .contains("unsupported image dimensions"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
