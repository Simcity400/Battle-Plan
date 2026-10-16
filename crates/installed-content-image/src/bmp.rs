//! Windows BMP decoding for the map's index and colour bitmaps.
//!
//! Both entry points return rows from the top-left corner whichever way the
//! file stores them. The shipped map bitmaps declare a `biSizeImage` two bytes
//! larger than their pixels occupy, so the declared size is never used as an
//! offset; only the file's own length bounds a read.

use std::io;

use super::{IndexedImage, Rgba8Image, invalid, reserved};

/// DIB header sizes the game's bitmaps use: BITMAPINFOHEADER through V5.
const DIB_SIZES: [u32; 5] = [40, 52, 56, 108, 124];

fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated BMP header"))?
            .try_into()
            .unwrap(),
    ))
}

fn u16_at(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid("truncated BMP header"))?
            .try_into()
            .unwrap(),
    ))
}

/// The parts of a BI_RGB Windows BMP header both decoders need.
struct Header {
    width: u32,
    height: u32,
    /// True when the file stores the top row first, that is, a negative height.
    top_down: bool,
    /// Offset and stride of the pixel array, padded to four bytes per row.
    pixels: usize,
    row_bytes: usize,
    stride: usize,
    /// Start and declared entry count of the color table.
    palette: usize,
    palette_entries: usize,
}

/// The validated 24-bit bitmap extent without allocating its decoded pixels.
pub fn rgb_dimensions(input: &[u8]) -> io::Result<[u32; 2]> {
    rgb_dimensions_prefix(input, input.len() as u64)
}

/// Validate dimensions using the complete DIB header and full verified file
/// length, without retaining or decoding the pixel array.
pub fn rgb_dimensions_prefix(prefix: &[u8], length: u64) -> io::Result<[u32; 2]> {
    let header = header(prefix, 24)?;
    let total = header
        .stride
        .checked_mul(header.height as usize)
        .and_then(|bytes| bytes.checked_add(header.pixels))
        .ok_or_else(|| invalid("BMP pixel storage overflows"))?;
    if total as u64 > length {
        return Err(invalid("truncated BMP pixel array"));
    }
    Ok([header.width, header.height])
}

fn header(input: &[u8], bits: u16) -> io::Result<Header> {
    if input.get(..2) != Some(b"BM") {
        return Err(invalid("file does not start with the BMP magic"));
    }
    let pixels = usize::try_from(u32_at(input, 10)?)
        .map_err(|_| invalid("BMP pixel offset exceeds address space"))?;
    let dib_size = u32_at(input, 14)?;
    if !DIB_SIZES.contains(&dib_size) {
        return Err(invalid(format!(
            "unsupported BMP DIB header size {dib_size}"
        )));
    }
    let dib_end = 14 + dib_size as usize;
    if dib_end > pixels || dib_end > input.len() {
        return Err(invalid(
            "BMP needs a complete DIB header before its color table",
        ));
    }
    let signed_width = u32_at(input, 18)? as i32;
    let signed_height = u32_at(input, 22)? as i32;
    if signed_width <= 0 || signed_height == 0 || signed_height == i32::MIN {
        return Err(invalid(
            "BMP dimensions must be nonzero and width must be positive",
        ));
    }
    if u16_at(input, 26)? != 1 {
        return Err(invalid("BMP must have exactly one plane"));
    }
    let stored_bits = u16_at(input, 28)?;
    if stored_bits != bits {
        return Err(invalid(format!(
            "BMP has {stored_bits}-bit pixels where {bits}-bit were required"
        )));
    }
    if u32_at(input, 30)? != 0 {
        return Err(invalid("BMP must use BI_RGB, uncompressed pixels"));
    }
    let width = signed_width as u32;
    let height = signed_height.unsigned_abs();
    let row_bits = (width as usize)
        .checked_mul(usize::from(bits))
        .ok_or_else(|| invalid("BMP row size overflows"))?;
    let stride = row_bits
        .checked_add(31)
        .map(|bits| bits / 32 * 4)
        .ok_or_else(|| invalid("BMP row stride overflows"))?;
    let declared = u32_at(input, 46)? as usize;
    if declared > 256 {
        return Err(invalid(format!(
            "BMP color table declares {declared} entries, more than 256"
        )));
    }
    let available = (pixels - dib_end) / 4;
    let palette_entries = match declared {
        0 if bits <= 8 => (1_usize << bits).min(available),
        0 => 0,
        count => count,
    };
    if palette_entries > available {
        return Err(invalid(
            "BMP color table does not fit before the pixel array",
        ));
    }
    Ok(Header {
        width,
        height,
        top_down: signed_height < 0,
        pixels,
        row_bytes: row_bits.div_ceil(8),
        stride,
        palette: dib_end,
        palette_entries,
    })
}

impl Header {
    /// Yields each stored row in top-to-bottom display order.
    fn rows<'a>(&self, input: &'a [u8]) -> io::Result<Vec<&'a [u8]>> {
        let total = self
            .stride
            .checked_mul(self.height as usize)
            .and_then(|bytes| bytes.checked_add(self.pixels))
            .ok_or_else(|| invalid("BMP pixel storage overflows"))?;
        if total > input.len() {
            return Err(invalid("truncated BMP pixel array"));
        }
        Ok((0..self.height as usize)
            .map(|y| {
                let stored = if self.top_down {
                    y
                } else {
                    self.height as usize - 1 - y
                };
                let start = self.pixels + stored * self.stride;
                &input[start..start + self.row_bytes]
            })
            .collect())
    }

    fn count(&self) -> io::Result<usize> {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| invalid("BMP pixel count overflows"))
    }
}

/// Decodes an 8-bit BI_RGB BMP into palette indices and an RGBA color table.
///
/// Entries the color table does not declare stay all zero; declared entries are
/// opaque, because the fourth byte of a BMP palette entry is reserved.
pub fn decode_indexed(input: &[u8]) -> io::Result<IndexedImage> {
    let header = header(input, 8)?;
    let mut palette = [[0_u8; 4]; 256];
    for (index, entry) in palette.iter_mut().take(header.palette_entries).enumerate() {
        let start = header.palette + index * 4;
        let stored = input
            .get(start..start + 4)
            .ok_or_else(|| invalid("truncated BMP color table"))?;
        *entry = [stored[2], stored[1], stored[0], 255];
    }
    let mut indices = reserved(header.count()?, "BMP index image")?;
    for row in header.rows(input)? {
        indices.extend_from_slice(row);
    }
    Ok(IndexedImage {
        width: header.width,
        height: header.height,
        indices,
        palette,
    })
}

/// Decodes a 24-bit BI_RGB BMP into opaque RGBA8 pixels.
pub fn decode_rgb(input: &[u8]) -> io::Result<Rgba8Image> {
    let header = header(input, 24)?;
    let mut pixels = reserved(
        header
            .count()?
            .checked_mul(4)
            .ok_or_else(|| invalid("BMP pixel count overflows"))?,
        "BMP color image",
    )?;
    for row in header.rows(input)? {
        for pixel in row.chunks_exact(3) {
            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    Ok(Rgba8Image {
        width: header.width,
        height: header.height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap(
        dib_size: u32,
        width: i32,
        height: i32,
        bits: u16,
        palette: &[[u8; 4]],
        declared_entries: u32,
        pixels: &[u8],
    ) -> Vec<u8> {
        // Always leave room for the fields this helper writes, even when the
        // declared DIB size is one the decoder must refuse.
        let header_bytes = (14 + dib_size as usize).max(54);
        let offset = header_bytes + palette.len() * 4;
        let mut bytes = vec![0_u8; header_bytes];
        bytes[..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&((offset + pixels.len()) as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[14..18].copy_from_slice(&dib_size.to_le_bytes());
        bytes[18..22].copy_from_slice(&width.to_le_bytes());
        bytes[22..26].copy_from_slice(&height.to_le_bytes());
        bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&bits.to_le_bytes());
        // A deliberately wrong biSizeImage, as the shipped map bitmaps carry.
        bytes[34..38].copy_from_slice(&(pixels.len() as u32 + 2).to_le_bytes());
        bytes[46..50].copy_from_slice(&declared_entries.to_le_bytes());
        for entry in palette {
            bytes.extend_from_slice(entry);
        }
        bytes.extend_from_slice(pixels);
        bytes
    }

    #[test]
    fn rgb_extent_validates_the_same_header_and_rows_without_decoding() {
        let bytes = bitmap(40, 2, 2, 24, &[], 0, &[0; 16]);
        assert_eq!(rgb_dimensions(&bytes).unwrap(), [2, 2]);
        assert!(rgb_dimensions(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn indexed_rows_pad_to_four_bytes_and_come_out_top_left_first() {
        let palette = [[9_u8, 8, 7, 0], [1, 2, 3, 0]];
        // Width 3 pads each row to 4 bytes; the fourth byte is not a pixel.
        let bottom_up = bitmap(40, 3, 2, 8, &palette, 2, &[1, 1, 0, 77, 0, 1, 0, 77]);
        let image = decode_indexed(&bottom_up).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(image.indices, vec![0, 1, 0, 1, 1, 0]);
        assert_eq!(image.palette[0], [7, 8, 9, 255]);
        assert_eq!(image.palette[1], [3, 2, 1, 255]);
        assert_eq!(image.palette[2], [0, 0, 0, 0]);
        let top_down = bitmap(40, 3, -2, 8, &palette, 2, &[0, 1, 0, 77, 1, 1, 0, 77]);
        assert_eq!(decode_indexed(&top_down).unwrap(), image);
    }

    #[test]
    fn every_supported_dib_header_size_is_accepted_and_others_are_named() {
        for size in DIB_SIZES {
            let bytes = bitmap(size, 1, 1, 8, &[[1, 2, 3, 0]], 1, &[0, 0, 0, 0]);
            assert_eq!(decode_indexed(&bytes).unwrap().indices, vec![0]);
        }
        let bytes = bitmap(12, 1, 1, 8, &[[1, 2, 3, 0]], 1, &[0, 0, 0, 0]);
        assert!(
            decode_indexed(&bytes)
                .unwrap_err()
                .to_string()
                .contains("DIB header size 12")
        );
    }

    #[test]
    fn an_undeclared_color_count_fills_the_table_from_the_available_bytes() {
        let palette: Vec<[u8; 4]> = (0..256).map(|i| [i as u8, 0, 0, 0]).collect();
        let bytes = bitmap(40, 1, 1, 8, &palette, 0, &[5, 0, 0, 0]);
        let image = decode_indexed(&bytes).unwrap();
        assert_eq!(image.palette[255], [0, 0, 255, 255]);
        // A short table still fills only what the file actually stores.
        let bytes = bitmap(40, 1, 1, 8, &palette[..3], 0, &[2, 0, 0, 0]);
        let image = decode_indexed(&bytes).unwrap();
        assert_eq!(image.palette[2], [0, 0, 2, 255]);
        assert_eq!(image.palette[3], [0, 0, 0, 0]);
    }

    #[test]
    fn twenty_four_bit_pixels_become_opaque_rgba() {
        // Two 24-bit pixels occupy six bytes; each row pads to eight.
        let bytes = bitmap(
            40,
            2,
            2,
            24,
            &[],
            0,
            &[1, 2, 3, 4, 5, 6, 99, 99, 7, 8, 9, 10, 11, 12, 99, 99],
        );
        let image = decode_rgb(&bytes).unwrap();
        assert_eq!(
            image.pixels,
            vec![9, 8, 7, 255, 12, 11, 10, 255, 3, 2, 1, 255, 6, 5, 4, 255]
        );
        assert!(decode_rgb(&bitmap(40, 1, 1, 8, &[[0; 4]], 1, &[0; 4])).is_err());
        assert!(decode_indexed(&bytes).is_err());
    }

    #[test]
    fn malformed_and_compressed_bitmaps_are_named() {
        let good = bitmap(40, 2, 2, 8, &[[0; 4], [1; 4]], 2, &[0, 1, 0, 0, 1, 0, 0, 0]);
        for length in 0..good.len() {
            assert!(decode_indexed(&good[..length]).is_err());
        }
        for (offset, value) in [
            (18, 0_u32),
            (22, 0),
            (22, i32::MIN as u32),
            (30, 1),
            (46, 300),
        ] {
            let mut bytes = good.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_indexed(&bytes).is_err());
        }
        let mut planes = good.clone();
        planes[26..28].copy_from_slice(&2_u16.to_le_bytes());
        assert!(decode_indexed(&planes).is_err());
        let mut short_table = good.clone();
        short_table[46..50].copy_from_slice(&9_u32.to_le_bytes());
        assert!(
            decode_indexed(&short_table)
                .unwrap_err()
                .to_string()
                .contains("does not fit")
        );
        let mut inside_header = good;
        inside_header[10..14].copy_from_slice(&20_u32.to_le_bytes());
        assert!(decode_indexed(&inside_header).is_err());
        assert!(decode_indexed(b"ZZ").is_err());
    }

    #[test]
    fn a_declared_image_size_larger_than_the_pixels_is_still_read() {
        // Every shipped 8-bit map bitmap declares two extra bytes.
        let bytes = bitmap(124, 4, 1, 8, &[[0; 4]], 1, &[0, 0, 0, 0]);
        assert_eq!(decode_indexed(&bytes).unwrap().indices.len(), 4);
    }

    #[test]
    fn indexed_images_expand_through_their_palette() {
        let bytes = bitmap(40, 2, 1, 8, &[[1, 2, 3, 0], [4, 5, 6, 0]], 2, &[1, 0, 0, 0]);
        assert_eq!(
            decode_indexed(&bytes).unwrap().to_rgba8().pixels,
            vec![6, 5, 4, 255, 3, 2, 1, 255]
        );
    }
}
