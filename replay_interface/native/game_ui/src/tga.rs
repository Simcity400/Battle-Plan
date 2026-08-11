use crate::dds::RgbaImage;
use crate::{ErrorKind, GameUiError, Result};
use std::fs;
use std::path::Path;

const TGA_HEADER_LEN: usize = 18;
/// Upper bound on header-claimed pixel count (8192x8192), mirroring
/// `dds::DDS_MAX_PIXELS`. TGA and DDS are adjacent arms of one texture switch
/// (`assets::AssetResolver::decode_texture`), so they must agree on allocation
/// policy: a header-claimed size that DDS refuses must not be one TGA honours.
///
/// The length preflight below is necessary but NOT sufficient on its own: RLE
/// amplifies, so a 67 MB `image_type = 11, bpp = 8, 65535x65535` file clears
/// the minimum-encoded-length check and then asks for a 17 GB output buffer.
/// `vec![0u8; len]` has no fallible form — a refused allocation aborts the
/// process — so the claim must be refused before the buffer exists.
///
/// Measured headroom: the largest TGA shipped by the game 1.19.2 is 4096x2048
/// (8.4 M pixels, `gfx/fonts/japanese/garamond_24.tga`), 8x under this cap.
const TGA_MAX_PIXELS: usize = 1 << 26;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TgaImage {
    pub image: RgbaImage,
    pub format: String,
}

#[derive(Clone, Debug)]
struct Header {
    id_len: usize,
    color_map_type: u8,
    image_type: u8,
    width: usize,
    height: usize,
    bits_per_pixel: u8,
    descriptor: u8,
}

pub fn decode_file(path: &Path) -> Result<TgaImage> {
    let data = fs::read(path)?;
    decode_bytes(&data).map_err(|err| add_path(err, path))
}

pub fn decode_bytes(data: &[u8]) -> Result<TgaImage> {
    let header = Header::parse(data)?;
    if header.color_map_type != 0 {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("tga_colormap".to_string()),
            "color-mapped TGA textures are not supported",
        ));
    }
    if header.width == 0 || header.height == 0 {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("tga_dimensions".to_string()),
            format!("invalid TGA dimensions {}x{}", header.width, header.height),
        ));
    }
    if header
        .width
        .checked_mul(header.height)
        .is_none_or(|pixels| pixels > TGA_MAX_PIXELS)
    {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("tga_dimensions".to_string()),
            format!(
                "TGA dimensions {}x{} exceed the {TGA_MAX_PIXELS}-pixel cap",
                header.width, header.height
            ),
        ));
    }

    let pixel_bytes = match (header.image_type, header.bits_per_pixel) {
        (2 | 10, 16) => 2,
        (2 | 10, 24) => 3,
        (2 | 10, 32) => 4,
        (3 | 11, 8) => 1,
        _ => {
            return Err(GameUiError::new(
                ErrorKind::InvalidTexture("tga_format".to_string()),
                format!(
                    "unsupported TGA type {} with {} bits per pixel",
                    header.image_type, header.bits_per_pixel
                ),
            ));
        }
    };

    let offset = TGA_HEADER_LEN + header.id_len;
    if data.len() < offset {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("tga_truncated".to_string()),
            "TGA header/id field is truncated",
        ));
    }

    // Validate the header-claimed pixel payload BEFORE allocating the output
    // buffer: a truncated 18-byte file claiming 65535x65535 must be a typed
    // refusal, never a ~17 GB allocation. Checked arithmetic is defensive
    // (u16 dimensions cannot overflow usize on 64-bit) but keeps every
    // header-derived size expression checked, per the fail-closed doctrine.
    let overflow = || {
        GameUiError::new(
            ErrorKind::InvalidTexture("tga_dimensions".to_string()),
            format!(
                "TGA size arithmetic overflows for {}x{}",
                header.width, header.height
            ),
        )
    };
    let total = header
        .width
        .checked_mul(header.height)
        .ok_or_else(overflow)?;
    let out_len = total.checked_mul(4).ok_or_else(overflow)?;
    match header.image_type {
        2 | 3 => {
            let needed = total
                .checked_mul(pixel_bytes)
                .and_then(|bytes| bytes.checked_add(offset))
                .ok_or_else(overflow)?;
            require_len(data, needed, "TGA pixel data")?;
        }
        10 | 11 => {
            // Best-case RLE: every packet is a run covering 128 pixels, so
            // the payload is at least ceil(total/128) * (header + one pixel)
            // bytes. Per-packet bounds checks in decode_rle remain the
            // authoritative enforcement during decode.
            let min_encoded = total
                .div_ceil(128)
                .checked_mul(1 + pixel_bytes)
                .and_then(|bytes| bytes.checked_add(offset))
                .ok_or_else(overflow)?;
            require_len(data, min_encoded, "TGA RLE pixel data")?;
        }
        _ => unreachable!("image type checked above"),
    }

    let mut pixels = vec![0u8; out_len];
    match header.image_type {
        2 | 3 => decode_uncompressed(data, &header, offset, pixel_bytes, &mut pixels)?,
        10 | 11 => decode_rle(data, &header, offset, pixel_bytes, &mut pixels)?,
        _ => unreachable!("image type checked above"),
    }

    Ok(TgaImage {
        image: RgbaImage {
            width: header.width,
            height: header.height,
            pixels,
        },
        format: format!("TGA{} type {}", header.bits_per_pixel, header.image_type),
    })
}

fn decode_uncompressed(
    data: &[u8],
    header: &Header,
    mut offset: usize,
    pixel_bytes: usize,
    out: &mut [u8],
) -> Result<()> {
    // Checked like every other header-derived size in this file: the pixel cap
    // already makes overflow impossible, but no size expression is left to
    // wrap silently if that cap ever moves.
    let needed = header
        .width
        .checked_mul(header.height)
        .and_then(|pixels| pixels.checked_mul(pixel_bytes))
        .and_then(|bytes| bytes.checked_add(offset))
        .ok_or_else(|| {
            GameUiError::new(
                ErrorKind::InvalidTexture("tga_dimensions".to_string()),
                format!(
                    "TGA size arithmetic overflows for {}x{}",
                    header.width, header.height
                ),
            )
        })?;
    require_len(data, needed, "TGA pixel data")?;
    for index in 0..header.width * header.height {
        let rgba = decode_pixel(
            &data[offset..offset + pixel_bytes],
            header.bits_per_pixel,
            header.attribute_bits(),
        );
        offset += pixel_bytes;
        write_oriented_pixel(out, header, index, rgba);
    }
    Ok(())
}

fn decode_rle(
    data: &[u8],
    header: &Header,
    mut offset: usize,
    pixel_bytes: usize,
    out: &mut [u8],
) -> Result<()> {
    let mut index = 0usize;
    let total = header.width * header.height;
    while index < total {
        let packet = *data.get(offset).ok_or_else(|| {
            GameUiError::new(
                ErrorKind::InvalidTexture("tga_truncated".to_string()),
                "TGA RLE packet header is truncated",
            )
        })?;
        offset += 1;
        let count = usize::from(packet & 0x7f) + 1;
        if packet & 0x80 != 0 {
            require_len(data, offset + pixel_bytes, "TGA RLE pixel")?;
            let rgba = decode_pixel(
                &data[offset..offset + pixel_bytes],
                header.bits_per_pixel,
                header.attribute_bits(),
            );
            offset += pixel_bytes;
            for _ in 0..count {
                if index >= total {
                    return Err(GameUiError::new(
                        ErrorKind::InvalidTexture("tga_rle".to_string()),
                        "TGA RLE packet overruns image bounds",
                    ));
                }
                write_oriented_pixel(out, header, index, rgba);
                index += 1;
            }
        } else {
            require_len(data, offset + count * pixel_bytes, "TGA raw RLE packet")?;
            for _ in 0..count {
                let rgba = decode_pixel(
                    &data[offset..offset + pixel_bytes],
                    header.bits_per_pixel,
                    header.attribute_bits(),
                );
                offset += pixel_bytes;
                if index >= total {
                    return Err(GameUiError::new(
                        ErrorKind::InvalidTexture("tga_rle".to_string()),
                        "TGA raw packet overruns image bounds",
                    ));
                }
                write_oriented_pixel(out, header, index, rgba);
                index += 1;
            }
        }
    }
    Ok(())
}

fn decode_pixel(bytes: &[u8], bits_per_pixel: u8, attribute_bits: u8) -> [u8; 4] {
    match bits_per_pixel {
        8 => [bytes[0], bytes[0], bytes[0], 255],
        16 => {
            let raw = u16::from_le_bytes([bytes[0], bytes[1]]);
            let b = ((u32::from(raw & 0x1f) * 255 + 15) / 31) as u8;
            let g = ((u32::from((raw >> 5) & 0x1f) * 255 + 15) / 31) as u8;
            let r = ((u32::from((raw >> 10) & 0x1f) * 255 + 15) / 31) as u8;
            // TGA image-descriptor bits 3-0 are the attribute-bits-per-pixel
            // count. The 16-bit high bit is alpha only when the file says it
            // carries one; when the count is 0 the bit is undefined padding
            // and the pixel is opaque. Both branches previously returned 255,
            // discarding every transparent 5-5-5-1 pixel (R11 C10(b)).
            let a = if attribute_bits == 0 || raw & 0x8000 != 0 {
                255
            } else {
                0
            };
            [r, g, b, a]
        }
        24 => [bytes[2], bytes[1], bytes[0], 255],
        32 => [bytes[2], bytes[1], bytes[0], bytes[3]],
        _ => unreachable!("bits per pixel checked by caller"),
    }
}

fn write_oriented_pixel(out: &mut [u8], header: &Header, src_index: usize, rgba: [u8; 4]) {
    let src_x = src_index % header.width;
    let src_y = src_index / header.width;
    let x = if header.descriptor & 0x10 != 0 {
        header.width - 1 - src_x
    } else {
        src_x
    };
    let y = if header.descriptor & 0x20 != 0 {
        src_y
    } else {
        header.height - 1 - src_y
    };
    let dst = (y * header.width + x) * 4;
    out[dst..dst + 4].copy_from_slice(&rgba);
}

impl Header {
    /// Image-descriptor bits 3-0: attribute (alpha) bits per pixel.
    fn attribute_bits(&self) -> u8 {
        self.descriptor & 0x0f
    }

    fn parse(data: &[u8]) -> Result<Self> {
        require_len(data, TGA_HEADER_LEN, "TGA header")?;
        Ok(Self {
            id_len: usize::from(data[0]),
            color_map_type: data[1],
            image_type: data[2],
            width: u16::from_le_bytes([data[12], data[13]]) as usize,
            height: u16::from_le_bytes([data[14], data[15]]) as usize,
            bits_per_pixel: data[16],
            descriptor: data[17],
        })
    }
}

fn require_len(data: &[u8], needed: usize, label: &str) -> Result<()> {
    if data.len() < needed {
        return Err(GameUiError::new(
            ErrorKind::InvalidTexture("tga_truncated".to_string()),
            format!("{label} is truncated: {} bytes, need {needed}", data.len()),
        ));
    }
    Ok(())
}

fn add_path(mut err: GameUiError, path: &Path) -> GameUiError {
    err.message = format!("{}: {}", path.display(), err.message);
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_uncompressed_top_left_bgra() {
        let mut data = tga_header(2, 2, 1, 32, 0x20 | 8);
        data.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 128]);
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 0, 255, 0, 128]);
    }

    #[test]
    fn decodes_rle_run_packet() {
        let mut data = tga_header(10, 2, 1, 32, 0x20);
        data.push(0x81); // run packet, 2 pixels
        data.extend_from_slice(&[0, 0, 255, 255]); // BGRA -> red
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn decodes_rle_raw_packet() {
        let mut data = tga_header(10, 2, 1, 32, 0x20);
        data.push(0x01); // raw packet, 2 pixels
        data.extend_from_slice(&[255, 0, 0, 255]); // BGRA -> blue
        data.extend_from_slice(&[0, 255, 0, 128]); // BGRA -> green
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![0, 0, 255, 255, 0, 255, 0, 128]);
    }

    #[test]
    fn rle_run_packet_overrunning_image_is_rejected() {
        let mut data = tga_header(10, 1, 1, 32, 0x20);
        data.push(0x81); // run of 2 into a 1-pixel image
        data.extend_from_slice(&[0, 0, 255, 255]);
        let err = decode_bytes(&data).expect_err("run overrun must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("tga_rle".to_string()));
    }

    #[test]
    fn rle_raw_packet_overrunning_image_is_rejected() {
        let mut data = tga_header(10, 1, 1, 32, 0x20);
        data.push(0x01); // raw packet of 2 into a 1-pixel image
        data.extend_from_slice(&[0, 0, 255, 255]);
        data.extend_from_slice(&[0, 255, 0, 255]);
        let err = decode_bytes(&data).expect_err("raw overrun must be rejected");
        assert_eq!(err.kind, ErrorKind::InvalidTexture("tga_rle".to_string()));
    }

    #[test]
    fn decodes_greyscale() {
        let mut data = tga_header(3, 1, 1, 8, 0x20);
        data.push(0x7f);
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![127, 127, 127, 255]);
    }

    #[test]
    fn decodes_16_bit_arrrrrgg_gggbbbbb() {
        let mut data = tga_header(2, 1, 1, 16, 0x20);
        data.extend_from_slice(&0x7c00u16.to_le_bytes()); // r=31 g=0 b=0
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn bottom_left_origin_flips_rows() {
        let mut data = tga_header(2, 1, 2, 32, 0); // no 0x20: bottom row first
        data.extend_from_slice(&[1, 2, 3, 4]); // file row 0 -> output row 1
        data.extend_from_slice(&[5, 6, 7, 8]); // file row 1 -> output row 0
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![7, 6, 5, 8, 3, 2, 1, 4]);
    }

    #[test]
    fn right_to_left_origin_flips_columns() {
        let mut data = tga_header(2, 2, 1, 32, 0x20 | 0x10);
        data.extend_from_slice(&[1, 2, 3, 4]); // file col 0 -> output col 1
        data.extend_from_slice(&[5, 6, 7, 8]); // file col 1 -> output col 0
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![7, 6, 5, 8, 3, 2, 1, 4]);
    }

    #[test]
    fn huge_uncompressed_claim_refuses_before_allocating() {
        // 18-byte file claiming 65535x65535x32bpp: must be a typed refusal,
        // never a ~17 GB zeroed allocation. The pixel cap now refuses this
        // one dimension check earlier than the truncation preflight did.
        let data = tga_header(2, u16::MAX, u16::MAX, 32, 0x20);
        let err = decode_bytes(&data).expect_err("truncated giant claim must be rejected");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("tga_dimensions".to_string())
        );
    }

    #[test]
    fn huge_rle_claim_refuses_before_allocating() {
        let data = tga_header(10, u16::MAX, u16::MAX, 32, 0x20);
        let err = decode_bytes(&data).expect_err("truncated giant RLE claim must be rejected");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("tga_dimensions".to_string())
        );
    }

    #[test]
    fn rle_amplified_giant_claim_is_refused_before_the_output_buffer() {
        // R11 C4: `image_type = 11, bpp = 8, 65535x65535` needs only ~67 MB of
        // input to clear the minimum-encoded-length preflight, and then asks
        // for a 17 GB `vec![0u8; out_len]` — an abort, not an error. Supply a
        // payload large enough to clear that preflight and prove the refusal
        // comes from the pixel cap, before any allocation.
        let total = usize::from(u16::MAX) * usize::from(u16::MAX);
        let min_encoded = total.div_ceil(128) * 2 + TGA_HEADER_LEN;
        let mut data = tga_header(11, u16::MAX, u16::MAX, 8, 0x20);
        data.resize(min_encoded, 0x80); // run packets, one payload byte each
        let err = decode_bytes(&data).expect_err("amplified RLE claim must be refused");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("tga_dimensions".to_string())
        );
        assert!(err.message.contains("pixel cap"), "{err}");
    }

    #[test]
    fn pixel_cap_matches_the_dds_arm_of_the_same_texture_switch() {
        // 8193x8193 is one pixel over 2^26 on both decoders.
        let data = tga_header(2, 8193, 8193, 32, 0x20);
        let err = decode_bytes(&data).expect_err("over-cap claim must be refused");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("tga_dimensions".to_string())
        );
        // The largest TGA the game 1.19.2 ships is 4096x2048; it must still parse
        // past the cap and fail only on its (absent) payload.
        let data = tga_header(2, 4096, 2048, 32, 0x20);
        let err = decode_bytes(&data).expect_err("no payload supplied");
        assert_eq!(
            err.kind,
            ErrorKind::InvalidTexture("tga_truncated".to_string())
        );
    }

    #[test]
    fn sixteen_bit_attribute_bit_drives_alpha_only_when_declared() {
        // descriptor low nibble = 1 attribute bit: the 0x8000 bit is alpha.
        let mut data = tga_header(2, 1, 1, 16, 0x20 | 0x01);
        data.extend_from_slice(&0x7c00u16.to_le_bytes()); // attribute bit clear
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![255, 0, 0, 0]);

        let mut data = tga_header(2, 1, 1, 16, 0x20 | 0x01);
        data.extend_from_slice(&0xfc00u16.to_le_bytes()); // attribute bit set
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![255, 0, 0, 255]);

        // Zero declared attribute bits: the high bit is padding, pixel opaque.
        let mut data = tga_header(2, 1, 1, 16, 0x20);
        data.extend_from_slice(&0x7c00u16.to_le_bytes());
        let image = decode_bytes(&data).unwrap().image;
        assert_eq!(image.pixels, vec![255, 0, 0, 255]);
    }

    fn tga_header(image_type: u8, width: u16, height: u16, bpp: u8, descriptor: u8) -> Vec<u8> {
        let mut data = vec![0u8; TGA_HEADER_LEN];
        data[2] = image_type;
        data[12..14].copy_from_slice(&width.to_le_bytes());
        data[14..16].copy_from_slice(&height.to_le_bytes());
        data[16] = bpp;
        data[17] = descriptor;
        data
    }
}
