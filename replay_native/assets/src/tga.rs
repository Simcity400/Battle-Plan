//! Minimal TGA decoder — enough for the game country FLAGS (and any uncompressed/RLE
//! truecolor TGA art the resolver hands back as `.tga`). the game flags are
//! uncompressed 32-bit BGRA (image type 2); we also support RLE (type 10) and
//! 24-bit, and honour the origin bit so the image is returned TOP-DOWN RGBA8 —
//! the layout the renderer's `upload_rgba8_image` expects.
//!
//! Intentionally small: no colour-mapped (paletted) support, no 16-bit — the game
//! flags do not use them. A format we cannot handle returns an `Err`, never a
//! fabricated image.

/// A decoded image: tightly-packed top-down RGBA8.
#[derive(Debug, Clone)]
pub struct TgaImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, top-left origin, RGBA order.
    pub rgba: Vec<u8>,
}

/// Decode a TGA byte buffer to top-down RGBA8.
pub fn decode(bytes: &[u8]) -> Result<TgaImage, String> {
    if bytes.len() < 18 {
        return Err("tga: too short for a header".to_string());
    }
    let id_len = bytes[0] as usize;
    let cmap_type = bytes[1];
    let img_type = bytes[2];
    // bytes[3..8] = colour-map spec (ignored; we reject mapped images below).
    let width = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let height = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    let bpp = bytes[16];
    let descriptor = bytes[17];

    if cmap_type != 0 {
        return Err("tga: colour-mapped images unsupported".to_string());
    }
    let rle = match img_type {
        2 => false, // uncompressed truecolor
        10 => true, // RLE truecolor
        other => return Err(format!("tga: unsupported image type {other}")),
    };
    if bpp != 32 && bpp != 24 {
        return Err(format!("tga: unsupported bpp {bpp} (need 24/32)"));
    }
    let channels = (bpp / 8) as usize;
    let top_origin = (descriptor & 0x20) != 0;

    // Pixel data begins after the header + the image-id field. `id_len` is attacker-
    // controlled (bytes[0], 0..=255); a truncated/malformed TGA where `18 + id_len`
    // exceeds the file length would otherwise panic at the slice below (start > len).
    // Guard it and return an Err instead of panicking (NO fabricated image).
    let data_start = 18usize + id_len;
    if bytes.len() < data_start {
        return Err(format!(
            "tga: header claims {id_len}-byte image-id but file is only {} bytes",
            bytes.len()
        ));
    }
    let data = &bytes[data_start..];
    let px_count = (width as usize) * (height as usize);
    if px_count == 0 {
        return Err("tga: zero-size image".to_string());
    }

    // Read the raw BGRA(8/24) pixels into an RGBA8 buffer in FILE order first.
    let mut raw = vec![0u8; px_count * 4];
    if rle {
        decode_rle(data, channels, px_count, &mut raw)?;
    } else {
        decode_raw(data, channels, px_count, &mut raw)?;
    }

    // The file stores rows bottom-up unless the origin bit is set. Produce a
    // top-down buffer.
    let row = (width as usize) * 4;
    let mut rgba = vec![0u8; px_count * 4];
    for y in 0..height as usize {
        let src_y = if top_origin {
            y
        } else {
            height as usize - 1 - y
        };
        let s = src_y * row;
        let d = y * row;
        rgba[d..d + row].copy_from_slice(&raw[s..s + row]);
    }

    Ok(TgaImage {
        width,
        height,
        rgba,
    })
}

/// Convert one BGRA/BGR source pixel into RGBA8 at `out[..4]`.
fn put_pixel(src: &[u8], channels: usize, out: &mut [u8]) {
    // TGA truecolor is little-endian BGRA / BGR.
    let b = src[0];
    let g = src[1];
    let r = src[2];
    let a = if channels == 4 { src[3] } else { 255 };
    out[0] = r;
    out[1] = g;
    out[2] = b;
    out[3] = a;
}

fn decode_raw(data: &[u8], channels: usize, px: usize, out: &mut [u8]) -> Result<(), String> {
    let need = px * channels;
    if data.len() < need {
        return Err(format!(
            "tga: truncated pixel data ({} < {need})",
            data.len()
        ));
    }
    for i in 0..px {
        put_pixel(&data[i * channels..], channels, &mut out[i * 4..]);
    }
    Ok(())
}

fn decode_rle(data: &[u8], channels: usize, px: usize, out: &mut [u8]) -> Result<(), String> {
    let mut di = 0usize; // index into compressed data
    let mut pi = 0usize; // pixel index written
    while pi < px {
        if di >= data.len() {
            return Err("tga: RLE stream ended early".to_string());
        }
        let packet = data[di];
        di += 1;
        let count = ((packet & 0x7F) as usize) + 1;
        if (packet & 0x80) != 0 {
            // RLE packet: one pixel repeated `count` times.
            if di + channels > data.len() {
                return Err("tga: RLE run pixel truncated".to_string());
            }
            for _ in 0..count {
                if pi >= px {
                    break;
                }
                put_pixel(&data[di..], channels, &mut out[pi * 4..]);
                pi += 1;
            }
            di += channels;
        } else {
            // Raw packet: `count` literal pixels.
            if di + count * channels > data.len() {
                return Err("tga: RLE raw run truncated".to_string());
            }
            for _ in 0..count {
                if pi >= px {
                    break;
                }
                put_pixel(&data[di..], channels, &mut out[pi * 4..]);
                di += channels;
                pi += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny uncompressed 32-bit bottom-up TGA: 2x1, pixel0=red, pixel1=blue.
    fn tiny_tga() -> Vec<u8> {
        let mut b = vec![0u8; 18];
        b[2] = 2; // uncompressed truecolor
        b[12] = 2; // width lo
        b[14] = 1; // height lo
        b[16] = 32; // bpp
        b[17] = 0x00; // bottom-origin, 0 alpha-attr bits in descriptor low nibble irrelevant here
                      // file order (bottom row, left->right): BGRA
                      // pixel0 = red  -> B=0,G=0,R=255,A=255
        b.extend_from_slice(&[0, 0, 255, 255]);
        // pixel1 = blue -> B=255,G=0,R=0,A=255
        b.extend_from_slice(&[255, 0, 0, 255]);
        b
    }

    #[test]
    fn decodes_uncompressed_bgra() {
        let img = decode(&tiny_tga()).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        // pixel0 red, pixel1 blue, in RGBA
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&img.rgba[4..8], &[0, 0, 255, 255]);
    }

    #[test]
    fn rejects_paletted() {
        let mut b = tiny_tga();
        b[1] = 1; // cmap_type = 1
        assert!(decode(&b).is_err());
    }

    /// BUG A (Codex-confirmed): a header that claims a non-zero image-id length but whose file
    /// is too short to contain it must return an Err, NEVER panic on the `&bytes[18+id_len..]`
    /// slice (start > len). Previously this panicked.
    #[test]
    fn truncated_image_id_does_not_panic() {
        // Exactly an 18-byte header (valid type/bpp) with id_len=5 → 18+5=23 > 18 = len.
        let mut b = vec![0u8; 18];
        b[0] = 5; // id_len = 5 bytes of image-id that do not exist
        b[2] = 2; // uncompressed truecolor
        b[12] = 2; // width 2
        b[14] = 1; // height 1
        b[16] = 32; // bpp
        let r = decode(&b);
        assert!(r.is_err(), "truncated image-id must Err, not panic");
        // A header whose image-id IS present but pixel data is then absent also Errs cleanly.
        let mut b2 = vec![0u8; 18 + 5];
        b2[0] = 5;
        b2[2] = 2;
        b2[12] = 2;
        b2[14] = 1;
        b2[16] = 32;
        assert!(decode(&b2).is_err(), "missing pixel data must Err");
    }

    /// A header shorter than 18 bytes Errs (not panic) — the basic length guard.
    #[test]
    fn too_short_header_errs() {
        assert!(decode(&[0u8; 5]).is_err());
        assert!(decode(&[]).is_err());
    }
}
