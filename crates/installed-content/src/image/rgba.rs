//! CPU expansion of the shared DDS decoder's BC1/2/3 blocks and 16-bit surfaces for UI artwork.
use super::{
    Rgba8Image,
    dds::{Texture, TextureFormat},
    invalid,
};
use std::io;

/// The base level of a two-dimensional texture as RGBA8. Its edges are those the
/// decoder admitted: the native decoder refuses any past its cited device limit.
/// Straight RGBA8 is moved, not copied; 16-bit surfaces expand as the device
/// samples them ([`TextureFormat::sample_16`]).
pub fn dds_base(mut texture: Texture) -> io::Result<Rgba8Image> {
    if texture.faces != 1 {
        return Err(invalid("UI artwork must be a two-dimensional texture"));
    }
    let format = texture.format;
    let mip = texture.mips.swap_remove(0).swap_remove(0);
    let texels = mip.width as usize * mip.height as usize;
    if format == TextureFormat::Rgba8 {
        return Ok(Rgba8Image {
            width: mip.width,
            height: mip.height,
            pixels: mip.data,
        });
    }
    if format.sample_16(0).is_some() {
        if mip.data.len() != texels * 2 {
            return Err(invalid(format!(
                "{format:?} UI artwork holds {} bytes for {}x{} texels",
                mip.data.len(),
                mip.width,
                mip.height
            )));
        }
        let pixels = mip
            .data
            .chunks_exact(2)
            .flat_map(|word| {
                let texel = format.sample_16(u16::from_le_bytes([word[0], word[1]]));
                texel
                    .unwrap_or_default()
                    .map(|value| (value * 255.0).round() as u8)
            })
            .collect();
        return Ok(Rgba8Image {
            width: mip.width,
            height: mip.height,
            pixels,
        });
    }
    if matches!(format, TextureFormat::Bc7 { .. }) {
        return Err(invalid("BC7 UI artwork has no CPU expansion"));
    }
    let size = format
        .block_bytes()
        .ok_or_else(|| invalid(format!("{format:?} UI artwork has no CPU expansion")))?;
    let mut image = Rgba8Image {
        width: mip.width,
        height: mip.height,
        pixels: vec![0; texels * 4],
    };
    for (index, block) in mip.data.chunks_exact(size).enumerate() {
        let mut rgba = [[0_u8; 4]; 16];
        let colors = if size == 8 { block } else { &block[8..] };
        let c0 = u16::from_le_bytes([colors[0], colors[1]]);
        let c1 = u16::from_le_bytes([colors[2], colors[3]]);
        let rgb = |c: u16| -> [u8; 4] {
            let r = ((c >> 11) & 31) as u8;
            let g = ((c >> 5) & 63) as u8;
            let b = (c & 31) as u8;
            [
                (r << 3) | (r >> 2),
                (g << 2) | (g >> 4),
                (b << 3) | (b >> 2),
                255,
            ]
        };
        let mut palette = [rgb(c0), rgb(c1), [0; 4], [0; 4]];
        let opaque = c0 > c1 || format != TextureFormat::Bc1;
        for channel in [0, 1, 2] {
            let a = u16::from(palette[0][channel]);
            let b = u16::from(palette[1][channel]);
            palette[2][channel] = if opaque {
                ((2 * a + b) / 3) as u8
            } else {
                ((a + b) / 2) as u8
            };
            palette[3][channel] = if opaque { ((a + 2 * b) / 3) as u8 } else { 0 };
        }
        palette[2][3] = 255;
        palette[3][3] = if opaque { 255 } else { 0 };
        let indices = u32::from_le_bytes(colors[4..8].try_into().unwrap());
        for (i, pixel) in rgba.iter_mut().enumerate() {
            *pixel = palette[((indices >> (i * 2)) & 3) as usize];
        }
        if format == TextureFormat::Bc2 {
            let alpha = u64::from_le_bytes(block[..8].try_into().unwrap());
            for (i, pixel) in rgba.iter_mut().enumerate() {
                pixel[3] = ((alpha >> (i * 4)) & 15) as u8 * 17;
            }
        } else if format == TextureFormat::Bc3 {
            let mut alpha = [0_u8; 8];
            alpha[0] = block[0];
            alpha[1] = block[1];
            let n = if alpha[0] > alpha[1] { 7 } else { 5 };
            for i in 1..n {
                alpha[i + 1] = (((n - i) as u16 * u16::from(alpha[0])
                    + i as u16 * u16::from(alpha[1]))
                    / n as u16) as u8;
            }
            if n == 5 {
                alpha[6] = 0;
                alpha[7] = 255;
            }
            let indices = block[2..8]
                .iter()
                .enumerate()
                .fold(0_u64, |bits, (i, b)| bits | (u64::from(*b) << (8 * i)));
            for (i, pixel) in rgba.iter_mut().enumerate() {
                pixel[3] = alpha[((indices >> (i * 3)) & 7) as usize];
            }
        }
        let x = index as u32 % mip.width.div_ceil(4) * 4;
        let y = index as u32 / mip.width.div_ceil(4) * 4;
        for dy in 0..4 {
            for dx in 0..4 {
                if x + dx < mip.width && y + dy < mip.height {
                    let at = (((y + dy) * mip.width + x + dx) * 4) as usize;
                    image.pixels[at..at + 4].copy_from_slice(&rgba[(dy * 4 + dx) as usize]);
                }
            }
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::super::dds::MipLevel;
    use super::*;
    fn image(format: TextureFormat, data: Vec<u8>) -> Rgba8Image {
        dds_base(Texture {
            format,
            width: 3,
            height: 2,
            faces: 1,
            mips: vec![vec![MipLevel {
                width: 3,
                height: 2,
                data,
            }]],
        })
        .unwrap()
    }
    #[test]
    fn partial_blocks_and_alpha_modes() {
        let red = [0, 248, 0, 0, 0, 0, 0, 0];
        assert!(
            image(TextureFormat::Bc1, red.to_vec())
                .pixels
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 255])
        );
        assert!(
            image(TextureFormat::Bc1, vec![0, 0, 0, 248, 255, 255, 255, 255])
                .pixels
                .chunks_exact(4)
                .all(|p| p == [0, 0, 0, 0])
        );
        let mut bc2 = vec![0x88; 8];
        bc2.extend(red);
        assert!(
            image(TextureFormat::Bc2, bc2)
                .pixels
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 136])
        );
        let mut bc3 = vec![64, 0, 0, 0, 0, 0, 0, 0];
        bc3.extend(red);
        assert!(
            image(TextureFormat::Bc3, bc3)
                .pixels
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 64])
        );
    }

    #[test]
    fn sixteen_bit_surfaces_expand_as_sampled() {
        // 5-6-5 pure red, then 1-5-5-5 half green with alpha clear.
        let red = 0xF800_u16.to_le_bytes().repeat(6);
        assert!(
            image(TextureFormat::B5G6R5, red)
                .pixels
                .chunks_exact(4)
                .all(|p| p == [255, 0, 0, 255])
        );
        let green = (16_u16 << 5).to_le_bytes().repeat(6);
        assert!(
            image(TextureFormat::B5G5R5A1, green)
                .pixels
                .chunks_exact(4)
                .all(|p| p == [0, 132, 0, 0])
        );
    }
}
