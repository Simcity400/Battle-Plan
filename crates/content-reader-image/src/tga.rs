
use std::io;

use super::{Rgba8Image, invalid, reserved};

const HEADER: usize = 18;
const RIGHT_TO_LEFT: u8 = 0x10;
const TOP_TO_BOTTOM: u8 = 0x20;

fn pixel(bytes: &[u8], depth: u8) -> [u8; 4] {
    match depth {
        8 => [bytes[0], bytes[0], bytes[0], 255],
        16 => {
            let value = u16::from_le_bytes([bytes[0], bytes[1]]);
            let expand = |channel: u16| ((channel * 255 + 15) / 31) as u8;
            [
                expand((value >> 10) & 0x1f),
                expand((value >> 5) & 0x1f),
                expand(value & 0x1f),
                255,
            ]
        }
        24 => [bytes[2], bytes[1], bytes[0], 255],
        _ => [bytes[2], bytes[1], bytes[0], bytes[3]],
    }
}

fn unpack(input: &[u8], unit: usize, count: usize) -> io::Result<Vec<u8>> {
    let total = count
        .checked_mul(unit)
        .ok_or_else(|| invalid("Targa pixel count overflows"))?;
    let mut out = reserved(total, "Targa image")?;
    let mut offset = 0;
    while out.len() < total {
        let packet = *input
            .get(offset)
            .ok_or_else(|| invalid("truncated Targa run-length packet"))?;
        offset += 1;
        let run = usize::from(packet & 0x7f) + 1;
        let taken = if packet & 0x80 == 0 { run } else { 1 };
        let end = offset
            .checked_add(taken * unit)
            .ok_or_else(|| invalid("Targa packet offset overflows"))?;
        let source = input
            .get(offset..end)
            .ok_or_else(|| invalid("truncated Targa run-length data"))?;
        if out.len() + run * unit > total {
            return Err(invalid("Targa run-length data overruns the image"));
        }
        if packet & 0x80 == 0 {
            out.extend_from_slice(source);
        } else {
            for _ in 0..run {
                out.extend_from_slice(source);
            }
        }
        offset = end;
    }
    Ok(out)
}

pub fn decode(input: &[u8]) -> io::Result<Rgba8Image> {
    let head = input
        .get(..HEADER)
        .ok_or_else(|| invalid("truncated Targa header"))?;
    let id_length = usize::from(head[0]);
    let color_map = head[1];
    let image_type = head[2];
    let width = u32::from(u16::from_le_bytes([head[12], head[13]]));
    let height = u32::from(u16::from_le_bytes([head[14], head[15]]));
    let depth = head[16];
    let descriptor = head[17];
    if color_map != 0 {
        return Err(invalid("colour-mapped Targa images are not supported"));
    }
    let greyscale = match image_type {
        2 | 10 => false,
        3 | 11 => true,
        other => {
            return Err(invalid(format!(
                "unsupported Targa image type {other}; only 2, 3, 10 and 11 are read"
            )));
        }
    };
    if !matches!(depth, 8 | 16 | 24 | 32) {
        return Err(invalid(format!("unsupported Targa pixel depth {depth}")));
    }
    if greyscale && depth != 8 {
        return Err(invalid(format!(
            "greyscale Targa images must be 8-bit, not {depth}-bit"
        )));
    }
    if !greyscale && depth == 8 {
        return Err(invalid("true-colour Targa images must be at least 16-bit"));
    }
    if width == 0 || height == 0 {
        return Err(invalid("Targa dimensions must be nonzero"));
    }
    let unit = usize::from(depth) / 8;
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| invalid("Targa pixel count overflows"))?;
    let start = HEADER
        .checked_add(id_length)
        .ok_or_else(|| invalid("Targa identification field overflows"))?;
    let body = input
        .get(start..)
        .ok_or_else(|| invalid("truncated Targa identification field"))?;
    let stored = if image_type >= 9 {
        unpack(body, unit, count)?
    } else {
        let end = count
            .checked_mul(unit)
            .ok_or_else(|| invalid("Targa pixel array overflows"))?;
        body.get(..end)
            .ok_or_else(|| invalid("truncated Targa pixel array"))?
            .to_vec()
    };

    let top_down = descriptor & TOP_TO_BOTTOM != 0;
    let mirrored = descriptor & RIGHT_TO_LEFT != 0;
    let mut pixels = reserved(count * 4, "Targa image")?;
    for y in 0..height as usize {
        let row = if top_down { y } else { height as usize - 1 - y };
        for x in 0..width as usize {
            let column = if mirrored { width as usize - 1 - x } else { x };
            let offset = (row * width as usize + column) * unit;
            pixels.extend_from_slice(&pixel(&stored[offset..offset + unit], depth));
        }
    }
    Ok(Rgba8Image {
        width,
        height,
        pixels,
    })
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
        let mut bytes = vec![0_u8; HEADER];
        bytes[2] = image_type;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = depth;
        bytes[17] = descriptor;
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn both_origins_and_mirroring_produce_top_left_rows() {
        let top = [1_u8, 2, 3, 4, 5, 6];
        let bottom = [7_u8, 8, 9, 10, 11, 12];
        let expected = vec![3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255];
        let stored_bottom_up = [bottom, top].concat();
        assert_eq!(
            decode(&targa(2, 2, 2, 24, 0, &stored_bottom_up))
                .unwrap()
                .pixels,
            expected
        );
        let stored_top_down = [top, bottom].concat();
        assert_eq!(
            decode(&targa(2, 2, 2, 24, TOP_TO_BOTTOM, &stored_top_down))
                .unwrap()
                .pixels,
            expected
        );
        let mirrored = [[4_u8, 5, 6], [1, 2, 3], [10, 11, 12], [7, 8, 9]].concat();
        assert_eq!(
            decode(&targa(
                2,
                2,
                2,
                24,
                TOP_TO_BOTTOM | RIGHT_TO_LEFT,
                &mirrored
            ))
            .unwrap()
            .pixels,
            expected
        );
    }

    #[test]
    fn run_length_packets_repeat_and_copy_across_rows() {
        let body = [0x82, 1, 2, 3, 255, 0x00, 8, 8, 8, 8];
        let image = decode(&targa(10, 2, 2, 32, TOP_TO_BOTTOM, &body)).unwrap();
        assert_eq!(
            image.pixels,
            vec![3, 2, 1, 255, 3, 2, 1, 255, 3, 2, 1, 255, 8, 8, 8, 8]
        );
        let image = decode(&targa(11, 2, 1, 8, TOP_TO_BOTTOM, &[0x81, 7])).unwrap();
        assert_eq!(image.pixels, vec![7, 7, 7, 255, 7, 7, 7, 255]);
        assert!(
            decode(&targa(10, 2, 1, 32, TOP_TO_BOTTOM, &[0x84, 1, 2, 3, 4]))
                .unwrap_err()
                .to_string()
                .contains("overruns")
        );
        assert!(decode(&targa(10, 2, 1, 32, TOP_TO_BOTTOM, &[0x81, 1, 2])).is_err());
    }

    #[test]
    fn sixteen_bit_pixels_expand_to_opaque_x1r5g5b5() {
        let white = 0x7fff_u16.to_le_bytes();
        let blue = 0x001f_u16.to_le_bytes();
        let body = [white, blue].concat();
        let expected = vec![255, 255, 255, 255, 0, 0, 255, 255];
        assert_eq!(
            decode(&targa(2, 2, 1, 16, TOP_TO_BOTTOM | 1, &body))
                .unwrap()
                .pixels,
            expected
        );
        assert_eq!(
            decode(&targa(2, 2, 1, 16, TOP_TO_BOTTOM, &body))
                .unwrap()
                .pixels,
            expected
        );
    }

    #[test]
    fn the_identification_field_is_skipped_and_a_footer_is_ignored() {
        let mut bytes = targa(2, 1, 1, 32, TOP_TO_BOTTOM, b"");
        bytes[0] = 5;
        bytes.extend_from_slice(b"NOTES");
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        bytes.extend_from_slice(&[0; 18]);
        bytes.extend_from_slice(b"TRUEVISION-XFILE.\0");
        assert_eq!(decode(&bytes).unwrap().pixels, vec![3, 2, 1, 4]);
    }

    #[test]
    fn unsupported_and_malformed_targas_are_named() {
        let good = targa(2, 2, 1, 32, TOP_TO_BOTTOM, &[1, 2, 3, 4, 5, 6, 7, 8]);
        for length in 0..good.len() {
            assert!(decode(&good[..length]).is_err());
        }
        assert!(
            decode(&targa(1, 1, 1, 8, 0, &[0]))
                .unwrap_err()
                .to_string()
                .contains("image type 1")
        );
        assert!(
            decode(&targa(2, 1, 1, 15, 0, &[0, 0]))
                .unwrap_err()
                .to_string()
                .contains("pixel depth 15")
        );
        assert!(decode(&targa(3, 1, 1, 24, 0, &[0; 3])).is_err());
        assert!(decode(&targa(2, 1, 1, 8, 0, &[0])).is_err());
        assert!(decode(&targa(2, 0, 1, 32, 0, &[])).is_err());
        let mut mapped = good;
        mapped[1] = 1;
        assert!(
            decode(&mapped)
                .unwrap_err()
                .to_string()
                .contains("colour-mapped")
        );
    }
}
