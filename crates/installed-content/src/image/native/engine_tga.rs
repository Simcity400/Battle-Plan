//! The game's Targa decoding step, selected by a `.tga` texture name.
//!
//! It takes an 18-byte header, no colour map, image type 2 or 10 (true colour, raw or
//! run-length), and 16, 24 or 32 bits. 32-bit pixels are `B8G8R8A8` (format `0xD`), the stored
//! fourth byte the alpha. 24-bit pixels are `B8G8R8` (`0xB`, uploaded with an ignored fourth
//! byte), and so are 16-bit ones, each 5-bit channel shifted left by 3 (`value << 3`, not
//! rescaled) and the top bit dropped. Rows are flipped to top first unless the descriptor's
//! `0x20` bit says they are stored so, and mirrored for its `0x10` bit. The loader never
//! checks the file's length against the pixels it reads, and
//! a run-length packet may write past the image; both are refused here as reads and writes
//! outside the game's buffers.

use super::{Failure, Zero, oversized, rgba};
use crate::image::dds::Texture;

/// The loader's failure messages, each the reason the chain goes on to FreeImage.
const SHORT: &str = "Targa header is shorter than 18 bytes.";
const COLOUR_MAP: &str = "Targa colour maps are unsupported.";
const NOT_TRUE_COLOUR: &str = "Targa image type must be raw or run-length true colour.";
const DEPTH: &str = "Targa pixel depth must be 16, 24 or 32 bits.";

fn beyond(what: &str) -> Failure {
    Failure::Undefined(format!(
        "a Targa whose {what}: the Targa decoding step reads or writes past its buffers"
    ))
}

/// `Ok(Err(message))` when Targa decoding fails, so the chain goes on.
pub(super) fn load(bytes: &[u8]) -> Result<Result<(Texture, Zero), &'static str>, Failure> {
    if bytes.len() < 18 {
        return Ok(Err(SHORT));
    }
    if bytes[1] != 0 {
        return Ok(Err(COLOUR_MAP));
    }
    let image_type = bytes[2];
    if image_type.wrapping_sub(2) & 0xF7 != 0 {
        return Ok(Err(NOT_TRUE_COLOUR));
    }
    // (bytes read per stored pixel, bytes kept per pixel)
    let (stored, kept) = match bytes[16] {
        32 => (4, 4),
        24 => (3, 3),
        16 => (2, 3),
        _ => return Ok(Err(DEPTH)),
    };
    let width = u32::from(u16::from_le_bytes([bytes[12], bytes[13]]));
    let height = u32::from(u16::from_le_bytes([bytes[14], bytes[15]]));
    // A zero side loads (no pixel is read); texture creation refuses it unless
    // `pad_power_of_two` widens it first.
    oversized(width, height)?;
    let total = width as usize * height as usize * kept;
    let mut data = bytes.get(18 + usize::from(bytes[0])..).unwrap_or_default();
    let mut out = Vec::with_capacity(total);
    let take = |data: &mut &[u8]| -> Result<[u8; 4], Failure> {
        let Some(pixel) = data.get(..stored) else {
            return Err(beyond("pixels run past the file"));
        };
        *data = &data[stored..];
        Ok(match stored {
            2 => {
                let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                [
                    ((value & 0x1F) << 3) as u8,
                    (((value >> 5) & 0x1F) << 3) as u8,
                    (((value >> 10) & 0x1F) << 3) as u8,
                    0,
                ]
            }
            3 => [pixel[0], pixel[1], pixel[2], 0],
            _ => [pixel[0], pixel[1], pixel[2], pixel[3]],
        })
    };
    if image_type == 2 {
        while out.len() < total {
            out.extend_from_slice(&take(&mut data)?[..kept]);
        }
    } else {
        while out.len() < total {
            let Some((&packet, rest)) = data.split_first() else {
                return Err(beyond("run-length packets run past the file"));
            };
            data = rest;
            let count = usize::from(packet & 0x7F) + 1;
            if out.len() + count * kept > total {
                return Err(beyond("run-length packet runs past the image"));
            }
            if packet & 0x80 != 0 {
                let pixel = take(&mut data)?;
                for _ in 0..count {
                    out.extend_from_slice(&pixel[..kept]);
                }
            } else {
                for _ in 0..count {
                    out.extend_from_slice(&take(&mut data)?[..kept]);
                }
            }
        }
    }
    let (width_usize, height_usize) = (width as usize, height as usize);
    let descriptor = bytes[17] & 0x30;
    let mut pixels = Vec::with_capacity(width_usize * height_usize * 4);
    for row in 0..height_usize {
        let stored_row = if descriptor & 0x20 != 0 {
            row
        } else {
            height_usize - 1 - row
        };
        for column in 0..width_usize {
            let stored_column = if descriptor & 0x10 != 0 {
                width_usize - 1 - column
            } else {
                column
            };
            let at = (stored_row * width_usize + stored_column) * kept;
            let texel = &out[at..at + kept];
            pixels.extend_from_slice(&[
                texel[2],
                texel[1],
                texel[0],
                if kept == 4 { texel[3] } else { 255 },
            ]);
        }
    }
    // 32 bits are `0xD`; 24 and 16 bits `0xB`.
    let zero = if kept == 4 {
        Zero::Transparent
    } else {
        Zero::Opaque
    };
    Ok(Ok((rgba(width, height, pixels), zero)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targa(
        image_type: u8,
        width: u16,
        height: u16,
        depth: u8,
        descriptor: u8,
        body: &[u8],
    ) -> Vec<u8> {
        let mut bytes = vec![0_u8; 18];
        bytes[2] = image_type;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = depth;
        bytes[17] = descriptor;
        bytes.extend_from_slice(body);
        bytes
    }

    fn pixels(file: &[u8]) -> Vec<u8> {
        load(file).unwrap().unwrap().0.base().data.clone()
    }

    /// Bottom-up rows come out top first; 16-bit channels shift left by 3 and drop the top bit.
    #[test]
    fn orientation_and_sixteen_bit_channels() {
        let rows = [[1_u8, 2, 3, 4], [5, 6, 7, 8]].concat();
        assert_eq!(
            pixels(&targa(2, 1, 2, 32, 0, &rows)),
            [7, 6, 5, 8, 3, 2, 1, 4]
        );
        assert_eq!(
            pixels(&targa(2, 1, 2, 32, 0x20, &rows)),
            [3, 2, 1, 4, 7, 6, 5, 8]
        );
        let white = 0xFFFF_u16.to_le_bytes();
        assert_eq!(
            pixels(&targa(2, 1, 1, 16, 0x20, &white)),
            [248, 248, 248, 255]
        );
        // A run of two 24-bit pixels.
        assert_eq!(
            pixels(&targa(10, 2, 1, 24, 0x20, &[0x81, 1, 2, 3])),
            [3, 2, 1, 255, 3, 2, 1, 255]
        );
    }

    /// Greyscale and colour-mapped files fail here and go on to FreeImage; reads past the file
    /// are refused rather than invented.
    #[test]
    fn failures_and_refusals() {
        assert_eq!(
            load(&targa(3, 1, 1, 8, 0, &[9])).unwrap(),
            Err(NOT_TRUE_COLOUR)
        );
        let mut mapped = targa(2, 1, 1, 24, 0, &[0; 3]);
        mapped[1] = 1;
        assert_eq!(load(&mapped).unwrap(), Err(COLOUR_MAP));
        assert_eq!(load(&targa(2, 1, 1, 8, 0, &[0])).unwrap(), Err(DEPTH));
        assert_eq!(load(&[0; 17]).unwrap(), Err(SHORT));
        assert!(matches!(
            load(&targa(2, 2, 1, 24, 0, &[0; 3])),
            Err(Failure::Undefined(_))
        ));
        assert!(matches!(
            load(&targa(10, 1, 1, 24, 0, &[0x81, 0, 0, 0])),
            Err(Failure::Undefined(_))
        ));
        // A zero side loads; the device refuses it later (`super::finish`).
        let (texture, zero) = load(&targa(2, 0, 1, 24, 0, &[])).unwrap().unwrap();
        assert_eq!((texture.width, texture.height, zero), (0, 1, Zero::Opaque));
    }
}
