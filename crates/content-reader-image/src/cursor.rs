
use super::{Rgba8Image, invalid};
use std::io;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimatedCursor {
    pub frames: Vec<Cursor>,
    pub steps: Vec<(u32, u32)>,
}

fn chunks(input: &[u8]) -> io::Result<Vec<(&[u8], &[u8])>> {
    let mut found = Vec::new();
    let mut at = 0;
    while at + 8 <= input.len() {
        let size = u32_at(input, at + 4)? as usize;
        let body = at + 8;
        let after = body
            .checked_add(size)
            .filter(|&after| after <= input.len())
            .ok_or_else(|| invalid("animated cursor chunk is truncated"))?;
        found.push((&input[at..at + 4], &input[body..after]));
        at = after + (size & 1);
    }
    Ok(found)
}

pub fn decode_ani(input: &[u8]) -> io::Result<AnimatedCursor> {
    if input.get(0..4) != Some(b"RIFF") || input.get(8..12) != Some(b"ACON") {
        return Err(invalid("is not an animated cursor (RIFF ACON) file"));
    }
    let end = (u32_at(input, 4)? as usize)
        .checked_add(8)
        .filter(|&end| end <= input.len())
        .ok_or_else(|| invalid("animated cursor is truncated"))?;
    let (mut header, mut rate, mut sequence, mut icons) = (None, None, None, Vec::new());
    for (id, body) in chunks(&input[12..end])? {
        match id {
            b"anih" => header = Some(body),
            b"rate" => rate = Some(body),
            b"seq " => sequence = Some(body),
            b"LIST" if body.starts_with(b"fram") => icons.extend(
                chunks(&body[4..])?
                    .into_iter()
                    .filter(|(id, _)| *id == b"icon")
                    .map(|(_, icon)| icon),
            ),
            _ => {}
        }
    }
    let header = header.ok_or_else(|| invalid("animated cursor has no anih header"))?;
    if header.len() < 36 {
        return Err(invalid("animated cursor anih header is truncated"));
    }
    let (frames, steps) = (u32_at(header, 4)?, u32_at(header, 8)?);
    let (rate_default, flags) = (u32_at(header, 28)?, u32_at(header, 32)?);
    if flags & 1 == 0 {
        return Err(invalid("animated cursor stores raw bitmaps, not cursor resources"));
    }
    if icons.len() != frames as usize {
        return Err(invalid(format!(
            "animated cursor declares {frames} frames and holds {}",
            icons.len()
        )));
    }
    let entries = |chunk: Option<&[u8]>, what: &str| -> io::Result<Option<Vec<u32>>> {
        chunk
            .map(|bytes| {
                if bytes.len() < steps as usize * 4 {
                    return Err(invalid(format!(
                        "animated cursor {what} chunk is shorter than its {steps} steps"
                    )));
                }
                (0..steps as usize).map(|step| u32_at(bytes, step * 4)).collect()
            })
            .transpose()
    };
    let rates = entries(rate, "rate")?;
    let order = if flags & 2 != 0 {
        Some(entries(sequence, "seq")?.ok_or_else(|| {
            invalid("animated cursor declares a sequence and has no seq chunk")
        })?)
    } else {
        None
    };
    let steps = (0..steps as usize)
        .map(|step| {
            let frame = order.as_ref().map_or(step as u32, |order| order[step]);
            let jiffies = rates.as_ref().map_or(rate_default, |rates| rates[step]);
            if frame >= frames {
                return Err(invalid(format!(
                    "animated cursor step {step} shows frame {frame} of {frames}"
                )));
            }
            if jiffies == 0 {
                return Err(invalid(format!("animated cursor step {step} shows for no time")));
            }
            Ok((frame, jiffies))
        })
        .collect::<io::Result<Vec<_>>>()?;
    if steps.is_empty() {
        return Err(invalid("animated cursor has no steps"));
    }
    let frames = icons
        .into_iter()
        .enumerate()
        .map(|(at, icon)| {
            decode(icon).map_err(|error| invalid(format!("animated cursor frame {at} {error}")))
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(AnimatedCursor { frames, steps })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cur(colour: [u8; 4], hotspot: [u16; 2]) -> Vec<u8> {
        let mut image = Vec::new();
        for value in [40u32, 1, 2] {
            image.extend_from_slice(&value.to_le_bytes());
        }
        image.extend_from_slice(&1u16.to_le_bytes());
        image.extend_from_slice(&32u16.to_le_bytes());
        image.extend_from_slice(&[0; 24]);
        image.extend_from_slice(&[colour[2], colour[1], colour[0], colour[3]]);
        image.extend_from_slice(&[0; 4]);
        let mut file = Vec::new();
        for value in [0u16, 2, 1] {
            file.extend_from_slice(&value.to_le_bytes());
        }
        file.extend_from_slice(&[1, 1, 0, 0]);
        file.extend_from_slice(&hotspot[0].to_le_bytes());
        file.extend_from_slice(&hotspot[1].to_le_bytes());
        file.extend_from_slice(&(image.len() as u32).to_le_bytes());
        file.extend_from_slice(&22u32.to_le_bytes());
        file.extend_from_slice(&image);
        file
    }

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_le_bytes()).collect()
    }

    fn ani(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body = chunks.concat();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(b"ACON");
        out.extend_from_slice(&body);
        out
    }

    fn frames(icons: &[Vec<u8>]) -> Vec<u8> {
        let mut body = b"fram".to_vec();
        for icon in icons {
            body.extend_from_slice(&chunk(b"icon", icon));
        }
        chunk(b"LIST", &body)
    }

    #[test]
    fn rate_and_sequence_give_each_step_its_frame_and_time_wherever_they_lie() {
        let file = ani(&[
            chunk(b"anih", &words(&[36, 2, 3, 0, 0, 0, 0, 8, 3])),
            frames(&[cur([255, 0, 0, 255], [3, 4]), cur([0, 0, 255, 128], [0, 0])]),
            chunk(b"rate", &words(&[5, 6, 7])),
            chunk(b"seq ", &words(&[1, 0, 1])),
        ]);
        let animated = decode_ani(&file).unwrap();
        assert_eq!(animated.steps, [(1, 5), (0, 6), (1, 7)]);
        assert_eq!(animated.frames[0].hotspot, [3, 4]);
        assert_eq!(animated.frames[0].image.pixels, [255, 0, 0, 255]);
        assert_eq!(animated.frames[1].image.pixels, [0, 0, 255, 128]);
    }

    #[test]
    fn without_rate_or_sequence_frames_play_in_order_at_the_header_rate() {
        let file = ani(&[
            chunk(b"anih", &words(&[36, 2, 2, 0, 0, 0, 0, 8, 1])),
            frames(&[cur([1, 2, 3, 255], [0, 0]), cur([4, 5, 6, 255], [0, 0])]),
        ]);
        assert_eq!(decode_ani(&file).unwrap().steps, [(0, 8), (1, 8)]);
    }

    #[test]
    fn a_missing_header_a_truncation_or_a_frame_out_of_range_is_refused() {
        let icon = || frames(&[cur([1, 2, 3, 255], [0, 0])]);
        let headless = ani(&[icon()]);
        assert!(decode_ani(&headless).unwrap_err().to_string().contains("no anih"));
        let whole = ani(&[chunk(b"anih", &words(&[36, 1, 1, 0, 0, 0, 0, 8, 1])), icon()]);
        assert!(decode_ani(&whole).is_ok());
        assert!(decode_ani(&whole[..whole.len() - 3]).is_err());
        let beyond = ani(&[
            chunk(b"anih", &words(&[36, 1, 1, 0, 0, 0, 0, 8, 3])),
            icon(),
            chunk(b"seq ", &words(&[1])),
        ]);
        let refused = decode_ani(&beyond).unwrap_err().to_string();
        assert!(refused.contains("frame 1 of 1"), "{refused}");
    }
}
