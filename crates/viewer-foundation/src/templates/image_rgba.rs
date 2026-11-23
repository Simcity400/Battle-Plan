use content_reader_image::{Rgba8Image, dds, rgba::bc_block};
use std::io;
use viewer_runtime::errors::invalid;

pub(super) fn rgba8(texture: &dds::Texture) -> io::Result<dds::Texture> {
    let block_size = match texture.format {
        dds::TextureFormat::Bc1 => 8,
        dds::TextureFormat::Bc2 | dds::TextureFormat::Bc3 => 16,
        dds::TextureFormat::Rgba8 => return Ok(texture.clone()),
        dds::TextureFormat::Bc7 { .. } => {
            return Err(invalid("BC7 has no counter-icon CPU expansion"));
        }
        dds::TextureFormat::B5G6R5 | dds::TextureFormat::B5G5R5A1 => {
            return Err(invalid("16-bit format has no counter-icon CPU expansion"));
        }
    };
    if texture.faces != 1 {
        return Err(invalid("a cube texture cannot be decoded to one image"));
    }
    let length = (texture.width as usize)
        .checked_mul(texture.height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| invalid("texture size overflows"))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|_| invalid("texture exceeds available memory"))?;
    pixels.resize(length, 0);
    let columns = texture.width.div_ceil(4);
    for (index, block) in texture.base().data.chunks_exact(block_size).enumerate() {
        for (i, pixel) in bc_block(block, texture.format).into_iter().enumerate() {
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
        assert_eq!(bc_block(&block[8..], Bc1)[0], [0; 4]);
        block[..8].fill(0xff);
        assert_eq!(bc_block(&block, Bc2)[0], [170, 170, 170, 255]);
        block[..8].fill(0);
        assert_eq!(bc_block(&block, Bc2)[0], [170, 170, 170, 0]);
    }

    #[test]
    fn bc3_short_alpha_palette_keeps_zero_and_opaque_endpoints() {
        let mut block = [0; 16];
        block[1] = 255;
        block[2] = 6 | (7 << 3);
        let pixels = bc_block(&block, Bc3);
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
                data: (vec![0, 248, 224, 7, 4, 0, 0, 0]).into(),
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
                    data: (vec![0; format.level_bytes(4, 4).unwrap()]).into(),
                }]],
            };
            assert!(rgba8(&texture).is_err(), "{format:?}");
        }
    }
}
