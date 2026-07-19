//! Shared 24-bit BMP writer for the `map_viewer` dump tools.
//!
//! Each `src/bin/dump_*.rs` includes this file via
//! `#[path = "dump_shared/bmp_out.rs"] mod bmp_out;` so the near-identical
//! per-tool writer copies collapse into one implementation without touching
//! the `map_viewer` library surface. Cargo does not treat this directory as a
//! binary target (no `main.rs` here).
//!
//! The output is byte-exact with the writers this module replaced:
//! BITMAPFILEHEADER + 40-byte BITMAPINFOHEADER (54 bytes total), 24 bpp
//! BI_RGB, 2835 px/m resolution, bottom-up BGR rows zero-padded to 4 bytes.

use map_viewer::Result;
use std::io::Write;
use std::path::Path;

/// Write a 24-bpp bottom-up BI_RGB BMP from top-down `[r, g, b]` pixels.
#[allow(dead_code)]
pub fn write_bmp24_rgb(path: &Path, width: usize, height: usize, rgb: &[[u8; 3]]) -> Result<()> {
    if rgb.len() != width * height {
        return Err(format!(
            "BMP payload has {} pixels but {}x{} needs {}",
            rgb.len(),
            width,
            height,
            width * height
        )
        .into());
    }
    let row_padded = (width * 3 + 3) & !3;
    let image_size = row_padded * height;
    let file_size = 54 + image_size;
    let mut buf = Vec::with_capacity(file_size);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&(file_size as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&54u32.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(width as i32).to_le_bytes());
    buf.extend_from_slice(&(height as i32).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(image_size as u32).to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&2835i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..height).rev() {
        for x in 0..width {
            let [r, g, b] = rgb[y * width + x];
            buf.push(b);
            buf.push(g);
            buf.push(r);
        }
        for _ in 0..(row_padded - width * 3) {
            buf.push(0);
        }
    }
    let mut file =
        std::fs::File::create(path).map_err(|err| format!("create {}: {err}", path.display()))?;
    file.write_all(&buf)
        .map_err(|err| format!("write {}: {err}", path.display()))?;
    Ok(())
}

/// Flat-slice convenience for callers holding packed `r,g,b` triples in a
/// `&[u8]`, preserving the byte-length refusal of the writers it replaced.
#[allow(dead_code)]
pub fn write_bmp24_rgb_flat(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    if rgb.len() != width * height * 3 {
        return Err(format!(
            "BMP payload has {} bytes but {}x{} RGB needs {}",
            rgb.len(),
            width,
            height,
            width * height * 3
        )
        .into());
    }
    let pixels: Vec<[u8; 3]> = rgb
        .chunks_exact(3)
        .map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    write_bmp24_rgb(path, width, height, &pixels)
}
