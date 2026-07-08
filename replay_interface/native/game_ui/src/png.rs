use crate::dds::RgbaImage;
use crate::{ErrorKind, GameUiError, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

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
}
