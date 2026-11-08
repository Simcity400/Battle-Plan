//! Windows `.cur` files holding one 32-bit image; others have no RGBA form and are refused.

use super::{Rgba8Image, invalid};
use std::io;

/// A cursor image and the pixel the pointer position names in it, from the top-left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub image: Rgba8Image,
    pub hotspot: [u16; 2],
}

fn u16_at(input: &[u8], at: usize) -> io::Result<u16> {
    input
        .get(at..at + 2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .ok_or_else(|| invalid("cursor is truncated"))
}

fn u32_at(input: &[u8], at: usize) -> io::Result<u32> {
    input
        .get(at..at + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| invalid("cursor is truncated"))
}

/// Decodes a `.cur` holding one uncompressed 32-bit image.
pub fn decode(input: &[u8]) -> io::Result<Cursor> {
    if u16_at(input, 0)? != 0 || u16_at(input, 2)? != 2 {
        return Err(invalid("is not a cursor resource file"));
    }
    match u16_at(input, 4)? {
        1 => {}
        count => {
            return Err(invalid(format!(
                "holds {count} images, which Windows chooses among by the system cursor size"
            )));
        }
    }
    let hotspot = [u16_at(input, 10)?, u16_at(input, 12)?];
    let size = u32_at(input, 14)? as usize;
    let offset = u32_at(input, 18)? as usize;
    let image = input
        .get(offset..offset.checked_add(size).ok_or_else(|| invalid("cursor is truncated"))?)
        .ok_or_else(|| invalid("cursor is truncated"))?;
    if image.starts_with(b"\x89PNG") {
        return Err(invalid("holds a PNG image, which this reader does not decode"));
    }
    let header = u32_at(image, 0)? as usize;
    let width = u32_at(image, 4)? as i32;
    // The height covers the colour bitmap and the AND mask below it.
    let height = u32_at(image, 8)? as i32 / 2;
    let bits = u16_at(image, 14)?;
    let compression = u32_at(image, 16)?;
    if width <= 0 || height <= 0 {
        return Err(invalid("cursor image has no pixels or runs top-down"));
    }
    if bits != 32 || compression != 0 {
        return Err(invalid(format!(
            "cursor image has {bits} bits per pixel and compression {compression}; only \
             uncompressed 32-bit images are decoded"
        )));
    }
    let (width, height) = (width as usize, height as usize);
    let colour = header;
    let mask = colour + width * height * 4;
    let mask_stride = width.div_ceil(32) * 4;
    if image.len() < mask + mask_stride * height {
        return Err(invalid("cursor image is truncated"));
    }
    let alpha = (0..width * height).any(|pixel| image[colour + pixel * 4 + 3] != 0);
    let mut pixels = vec![0; width * height * 4];
    for row in 0..height {
        // Rows are stored bottom-up.
        let stored = height - 1 - row;
        for column in 0..width {
            let at = colour + (stored * width + column) * 4;
            let [b, g, r, a] = [image[at], image[at + 1], image[at + 2], image[at + 3]];
            let a = if alpha {
                a
            } else {
                let bit = image[mask + stored * mask_stride + column / 8] >> (7 - column % 8) & 1;
                if bit == 1 && (r | g | b) != 0 {
                    return Err(invalid(
                        "cursor image inverts the screen under its AND mask: no RGBA form",
                    ));
                }
                if bit == 1 { 0 } else { 255 }
            };
            let out = (row * width + column) * 4;
            pixels[out..out + 4].copy_from_slice(&[r, g, b, a]);
        }
    }
    Ok(Cursor {
        image: Rgba8Image {
            width: width as u32,
            height: height as u32,
            pixels,
        },
        hotspot,
    })
}
