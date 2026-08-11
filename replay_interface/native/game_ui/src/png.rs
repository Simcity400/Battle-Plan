use crate::dds::RgbaImage;
use crate::{ErrorKind, GameUiError, Result};
use flate2::read::ZlibDecoder;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const PNG_MAX_PIXELS: usize = 1 << 26;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PngImage {
    pub image: RgbaImage,
    pub format: String,
}

pub fn decode_file(path: &Path) -> Result<PngImage> {
    let data = fs::read(path)?;
    decode_bytes(&data).map_err(|mut err| {
        err.message = format!("{}: {}", path.display(), err.message);
        err
    })
}

/// Decode the non-interlaced, 8-bit PNG forms used by the game interface assets.
/// Unsupported PNG features fail closed instead of being silently reinterpreted.
pub fn decode_bytes(data: &[u8]) -> Result<PngImage> {
    if data.get(..PNG_SIGNATURE.len()) != Some(PNG_SIGNATURE) {
        return Err(invalid_png("png_signature", "invalid PNG signature"));
    }

    let mut offset = PNG_SIGNATURE.len();
    let mut header = None;
    let mut idat = Vec::new();
    let mut saw_iend = false;
    while offset < data.len() {
        let chunk_header = data
            .get(offset..offset + 8)
            .ok_or_else(|| invalid_png("png_truncated", "PNG chunk header is truncated"))?;
        let length = u32::from_be_bytes(chunk_header[..4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = chunk_header[4..8].try_into().unwrap();
        offset += 8;
        let end = offset
            .checked_add(length)
            .and_then(|value| value.checked_add(4))
            .ok_or_else(|| invalid_png("png_chunk", "PNG chunk length overflows"))?;
        let chunk = data
            .get(offset..offset + length)
            .ok_or_else(|| invalid_png("png_truncated", "PNG chunk payload is truncated"))?;
        let stored_crc = data
            .get(offset + length..end)
            .ok_or_else(|| invalid_png("png_truncated", "PNG chunk CRC is truncated"))?;
        let mut crc_input = Vec::with_capacity(4 + length);
        crc_input.extend_from_slice(&kind);
        crc_input.extend_from_slice(chunk);
        if crc32(&crc_input).to_be_bytes() != stored_crc {
            return Err(invalid_png("png_crc", "PNG chunk CRC does not match"));
        }

        match &kind {
            b"IHDR" => {
                if header.is_some() || offset != PNG_SIGNATURE.len() + 8 || length != 13 {
                    return Err(invalid_png("png_header", "PNG IHDR is invalid or repeated"));
                }
                header = Some(PngHeader::parse(chunk)?);
            }
            b"IDAT" => {
                if header.is_none() || saw_iend {
                    return Err(invalid_png("png_chunk_order", "PNG IDAT is out of order"));
                }
                idat.extend_from_slice(chunk);
            }
            b"IEND" => {
                if length != 0 || saw_iend {
                    return Err(invalid_png("png_iend", "PNG IEND is invalid or repeated"));
                }
                saw_iend = true;
            }
            _ => {}
        }
        offset = end;
        if saw_iend {
            if offset != data.len() {
                return Err(invalid_png("png_trailing", "PNG has bytes after IEND"));
            }
            break;
        }
    }

    let header = header.ok_or_else(|| invalid_png("png_header", "PNG has no IHDR"))?;
    if !saw_iend || idat.is_empty() {
        return Err(invalid_png("png_truncated", "PNG is missing IDAT or IEND"));
    }
    let row_bytes = header
        .width
        .checked_mul(header.bytes_per_pixel)
        .ok_or_else(|| invalid_png("png_dimensions", "PNG row size overflows"))?;
    let filtered_len = row_bytes
        .checked_add(1)
        .and_then(|value| value.checked_mul(header.height))
        .ok_or_else(|| invalid_png("png_dimensions", "PNG decoded size overflows"))?;
    let mut filtered = Vec::with_capacity(filtered_len.min(1 << 20));
    ZlibDecoder::new(idat.as_slice())
        .take(filtered_len as u64 + 1)
        .read_to_end(&mut filtered)
        .map_err(|err| invalid_png("png_deflate", format!("PNG zlib stream is invalid: {err}")))?;
    if filtered.len() != filtered_len {
        return Err(invalid_png(
            "png_data_size",
            format!(
                "PNG expands to {} bytes, expected {filtered_len}",
                filtered.len()
            ),
        ));
    }

    let mut scanlines = vec![0u8; row_bytes * header.height];
    for y in 0..header.height {
        let filtered_row = &filtered[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = filtered_row[0];
        let source = &filtered_row[1..];
        let (before, rest) = scanlines.split_at_mut(y * row_bytes);
        let row = &mut rest[..row_bytes];
        let previous = if y == 0 {
            None
        } else {
            Some(&before[(y - 1) * row_bytes..y * row_bytes])
        };
        unfilter_row(filter, source, row, previous, header.bytes_per_pixel)?;
    }

    let pixels = expand_rgba(&scanlines, header.color_type);
    Ok(PngImage {
        image: RgbaImage {
            width: header.width,
            height: header.height,
            pixels,
        },
        format: format!("PNG8 color type {}", header.color_type),
    })
}

#[derive(Clone, Copy, Debug)]
struct PngHeader {
    width: usize,
    height: usize,
    color_type: u8,
    bytes_per_pixel: usize,
}

impl PngHeader {
    fn parse(data: &[u8]) -> Result<Self> {
        let width = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
        let height = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
        let bit_depth = data[8];
        let color_type = data[9];
        let compression = data[10];
        let filter = data[11];
        let interlace = data[12];
        if width == 0
            || height == 0
            || width
                .checked_mul(height)
                .is_none_or(|pixels| pixels > PNG_MAX_PIXELS)
        {
            return Err(invalid_png(
                "png_dimensions",
                format!("PNG dimensions {width}x{height} exceed the {PNG_MAX_PIXELS}-pixel cap"),
            ));
        }
        if bit_depth != 8 || compression != 0 || filter != 0 || interlace != 0 {
            return Err(invalid_png(
                "png_format",
                format!(
                    "unsupported PNG bit depth {bit_depth}, compression {compression}, filter {filter}, interlace {interlace}"
                ),
            ));
        }
        let bytes_per_pixel = match color_type {
            0 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => {
                return Err(invalid_png(
                    "png_color_type",
                    format!("unsupported PNG color type {color_type}"),
                ))
            }
        };
        Ok(Self {
            width,
            height,
            color_type,
            bytes_per_pixel,
        })
    }
}

fn unfilter_row(
    filter: u8,
    source: &[u8],
    row: &mut [u8],
    previous: Option<&[u8]>,
    bytes_per_pixel: usize,
) -> Result<()> {
    for index in 0..source.len() {
        let left = index
            .checked_sub(bytes_per_pixel)
            .map_or(0, |left| row[left]);
        let above = previous.map_or(0, |previous| previous[index]);
        let upper_left = previous.map_or(0, |previous| {
            index
                .checked_sub(bytes_per_pixel)
                .map_or(0, |left| previous[left])
        });
        let predictor = match filter {
            0 => 0,
            1 => left,
            2 => above,
            3 => ((u16::from(left) + u16::from(above)) / 2) as u8,
            4 => paeth(left, above, upper_left),
            _ => {
                return Err(invalid_png(
                    "png_filter",
                    format!("unsupported PNG scanline filter {filter}"),
                ))
            }
        };
        row[index] = source[index].wrapping_add(predictor);
    }
    Ok(())
}

fn paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let above = i32::from(above);
    let upper_left = i32::from(upper_left);
    let estimate = left + above - upper_left;
    let left_distance = (estimate - left).abs();
    let above_distance = (estimate - above).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= above_distance && left_distance <= upper_left_distance {
        left as u8
    } else if above_distance <= upper_left_distance {
        above as u8
    } else {
        upper_left as u8
    }
}

fn expand_rgba(scanlines: &[u8], color_type: u8) -> Vec<u8> {
    let channels = match color_type {
        0 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => unreachable!("color type checked by PngHeader::parse"),
    };
    let mut pixels = Vec::with_capacity(scanlines.len() / channels * 4);
    for pixel in scanlines.chunks_exact(channels) {
        match color_type {
            0 => pixels.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]),
            2 => pixels.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]),
            4 => pixels.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            6 => pixels.extend_from_slice(pixel),
            _ => unreachable!(),
        }
    }
    pixels
}

fn invalid_png(code: &str, message: impl Into<String>) -> GameUiError {
    GameUiError::new(ErrorKind::InvalidTexture(code.to_string()), message)
}

pub fn write_png(path: &Path, image: &RgbaImage) -> Result<()> {
    if image.pixels.len() != image.width * image.height * 4 {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("rgba".to_string()),
            format!(
                "RGBA buffer has {} bytes, expected {} for {}x{}",
                image.pixels.len(),
                image.width * image.height * 4,
                image.width,
                image.height
            ),
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(PNG_SIGNATURE)?;
    write_chunk(
        &mut file,
        b"IHDR",
        &ihdr(image.width as u32, image.height as u32),
    )?;
    let raw = filtered_scanlines(image);
    let compressed = zlib_store(&raw);
    write_chunk(&mut file, b"IDAT", &compressed)?;
    write_chunk(&mut file, b"IEND", &[])?;
    Ok(())
}

fn ihdr(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(13);
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.push(8);
    out.push(6);
    out.push(0);
    out.push(0);
    out.push(0);
    out
}

fn filtered_scanlines(image: &RgbaImage) -> Vec<u8> {
    let stride = image.width * 4;
    let mut out = Vec::with_capacity((stride + 1) * image.height);
    for y in 0..image.height {
        out.push(0);
        let start = y * stride;
        out.extend_from_slice(&image.pixels[start..start + stride]);
    }
    out
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65_535 * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut offset = 0usize;
    while offset < data.len() {
        let remaining = data.len() - offset;
        let len = remaining.min(65_535);
        let final_block = offset + len == data.len();
        out.push(if final_block { 0x01 } else { 0x00 });
        let len16 = len as u16;
        out.extend_from_slice(&len16.to_le_bytes());
        out.extend_from_slice(&(!len16).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + len]);
        offset += len;
    }
    if data.is_empty() {
        out.push(0x01);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn write_chunk(mut writer: impl Write, kind: &[u8; 4], data: &[u8]) -> Result<()> {
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(kind)?;
    writer.write_all(data)?;
    let mut crc_input = Vec::with_capacity(kind.len() + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    writer.write_all(&crc32(&crc_input).to_be_bytes())?;
    Ok(())
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn writes_png_signature_and_chunks() {
        let image = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![255, 0, 0, 255],
        };
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("game_ui_png_{nonce}.png"));
        write_png(&path, &image).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[..8], PNG_SIGNATURE);
        assert!(bytes.windows(4).any(|window| window == b"IDAT"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn decodes_written_rgba_png_without_changing_pixels() {
        let image = RgbaImage {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
            ],
        };
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("game_ui_png_roundtrip_{nonce}.png"));
        write_png(&path, &image).unwrap();
        let decoded = decode_file(&path).unwrap();
        assert_eq!(decoded.image, image);
        assert_eq!(decoded.format, "PNG8 color type 6");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn refuses_corrupt_chunk_crc() {
        let image = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 4],
        };
        let mut path = std::env::temp_dir();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("game_ui_png_crc_{nonce}.png"));
        write_png(&path, &image).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[29] ^= 1; // IHDR CRC starts immediately after its 13-byte payload.
        let err = decode_bytes(&bytes).expect_err("corrupt CRC must fail closed");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("png_crc".to_string()));
        let _ = fs::remove_file(path);
    }
}
