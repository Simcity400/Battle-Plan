//! CPU RGBA8 decoding for division-template icon preparation.
use installed_content::image::{Rgba8Image, dds};
use std::io;

fn rgb565(value: u16) -> [u8; 4] {
    let [r, g, b] = [
        (value >> 11) as u8,
        ((value >> 5) & 63) as u8,
        (value & 31) as u8,
    ];
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
        255,
    ]
}

fn block_pixels(block: &[u8], format: dds::TextureFormat) -> [[u8; 4]; 16] {
    use dds::TextureFormat::{Bc1, Bc2};
    let offset = if format == Bc1 { 0 } else { 8 };
    let first = u16::from_le_bytes(block[offset..offset + 2].try_into().unwrap());
    let second = u16::from_le_bytes(block[offset + 2..offset + 4].try_into().unwrap());
    let mut colors = [rgb565(first), rgb565(second), [0; 4], [0; 4]];
    if format == Bc1 && first <= second {
        colors[2] = std::array::from_fn(|c| {
            ((u16::from(colors[0][c]) + u16::from(colors[1][c])) / 2) as u8
        });
    } else {
        colors[2] = std::array::from_fn(|c| {
            ((2 * u16::from(colors[0][c]) + u16::from(colors[1][c])) / 3) as u8
        });
        colors[3] = std::array::from_fn(|c| {
            ((u16::from(colors[0][c]) + 2 * u16::from(colors[1][c])) / 3) as u8
        });
    }
    let indices = u32::from_le_bytes(block[offset + 4..offset + 8].try_into().unwrap());
    let mut pixels = std::array::from_fn(|i| colors[((indices >> (2 * i)) & 3) as usize]);
    if format == Bc1 {
        return pixels;
    }
    if format == Bc2 {
        let alpha = u64::from_le_bytes(block[..8].try_into().unwrap());
        for (i, pixel) in pixels.iter_mut().enumerate() {
            pixel[3] = (((alpha >> (4 * i)) & 15) * 17) as u8;
        }
    } else {
        let mut alpha = [block[0], block[1], 0, 0, 0, 0, 0, 255];
        let steps = if alpha[0] > alpha[1] { 7 } else { 5 };
        for i in 1..steps {
            alpha[i + 1] = (((steps - i) as u16 * u16::from(alpha[0])
                + i as u16 * u16::from(alpha[1]))
                / steps as u16) as u8;
        }
        let indices = block[2..8]
            .iter()
            .enumerate()
            .fold(0_u64, |v, (i, byte)| v | (u64::from(*byte) << (8 * i)));
        for (i, pixel) in pixels.iter_mut().enumerate() {
            pixel[3] = alpha[((indices >> (3 * i)) & 7) as usize];
        }
    }
    pixels
}

/// Decodes a BC1/BC2/BC3 texture's base level to RGBA8 at its exact extent.
/// Formats without a supported CPU expansion are refused by name.
pub(crate) fn rgba8(texture: &dds::Texture) -> io::Result<dds::Texture> {
    let block_size = match texture.format {
        dds::TextureFormat::Bc1 => 8,
        dds::TextureFormat::Bc2 | dds::TextureFormat::Bc3 => 16,
        dds::TextureFormat::Rgba8 => return Ok(texture.clone()),
        dds::TextureFormat::Bc7 { .. } => {
            return Err(super::invalid("BC7 has no counter-icon CPU expansion"));
        }
        dds::TextureFormat::B5G6R5 | dds::TextureFormat::B5G5R5A1 => {
            return Err(super::invalid(
                "16-bit format has no counter-icon CPU expansion",
            ));
        }
    };
    if texture.faces != 1 {
        return Err(super::invalid(
            "a cube texture cannot be decoded to one image",
        ));
    }
    let length = (texture.width as usize)
        .checked_mul(texture.height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| super::invalid("texture size overflows"))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|_| super::invalid("texture exceeds available memory"))?;
    pixels.resize(length, 0);
    let columns = texture.width.div_ceil(4);
    for (index, block) in texture.base().data.chunks_exact(block_size).enumerate() {
        for (i, pixel) in block_pixels(block, texture.format).into_iter().enumerate() {
            let x = (index as u32 % columns) * 4 + i as u32 % 4;
            let y = (index as u32 / columns) * 4 + i as u32 / 4;
            if x < texture.width && y < texture.height {
                let offset = ((y * texture.width + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&pixel);
            }
        }
    }
    Ok(dds::Texture::from_rgba8(Rgba8Image {
        width: texture.width,
        height: texture.height,
        pixels,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dds::TextureFormat::{Bc1, Bc2, Bc3};

    #[test]
    fn bc1_transparency_and_bc2_explicit_alpha_have_distinct_rules() {
        let mut block = [0; 16];
        block[10..12].copy_from_slice(&0xffff_u16.to_le_bytes());
        block[12..].fill(0xff);
        assert_eq!(block_pixels(&block[8..], Bc1)[0], [0; 4]);
        block[..8].fill(0xff);
        assert_eq!(block_pixels(&block, Bc2)[0], [170, 170, 170, 255]);
        block[..8].fill(0);
        assert_eq!(block_pixels(&block, Bc2)[0], [170, 170, 170, 0]);
    }

    #[test]
    fn bc3_short_alpha_palette_keeps_zero_and_opaque_endpoints() {
        let mut block = [0; 16];
        block[1] = 255;
        block[2] = 6 | (7 << 3);
        let pixels = block_pixels(&block, Bc3);
        assert_eq!(pixels[0][3], 0);
        assert_eq!(pixels[1][3], 255);
    }

    #[test]
    fn odd_extent_discards_padding_without_stretching_sprite() {
        let texture = dds::Texture {
            format: Bc1,
            width: 2,
            height: 1,
            faces: 1,
            mips: vec![vec![dds::MipLevel {
                width: 2,
                height: 1,
                data: vec![0, 248, 224, 7, 4, 0, 0, 0],
            }]],
        };
        let result = rgba8(&texture).unwrap();
        assert_eq!((result.width, result.height), (2, 1));
        assert_eq!(result.base().data, [255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn formats_without_cpu_expansion_are_refused() {
        for format in [
            dds::TextureFormat::B5G6R5,
            dds::TextureFormat::B5G5R5A1,
            dds::TextureFormat::Bc7 { srgb: false },
        ] {
            let texture = dds::Texture {
                format,
                width: 4,
                height: 4,
                faces: 1,
                mips: vec![vec![dds::MipLevel {
                    width: 4,
                    height: 4,
                    data: vec![0; format.level_bytes(4, 4).unwrap()],
                }]],
            };
            assert!(rgba8(&texture).is_err(), "{format:?}");
        }
    }
}
