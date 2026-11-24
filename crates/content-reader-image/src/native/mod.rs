
mod engine_tga;
mod libjpeg;
mod libjpeg_idct;
mod load_dds;
mod standard;

use super::dds::{MipLevel, Texture, TextureFormat};
use std::cell::Cell;

const DEVICE_LIMIT: u32 = 16384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Undecodable(String),
    Undefined(String),
    Device(String),
    OverBudget(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decode {
    pub force_pow2_textures: bool,
}

thread_local! {
    static TEXEL_BUDGET: Cell<Option<(u64, bool)>> = const { Cell::new(None) };
}

pub fn decode_texture_image_within(
    name: &[u8],
    bytes: impl Into<crate::Bytes>,
    decode: Decode,
    max_texels: u64,
) -> Result<Texture, Failure> {
    struct Restore(Option<(u64, bool)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEXEL_BUDGET.set(self.0);
        }
    }
    let _restore = Restore(TEXEL_BUDGET.replace(Some((max_texels, decode.force_pow2_textures))));
    decode_texture_image(name, bytes, decode)
}

pub fn decode_texture_image(name: &[u8], bytes: impl Into<crate::Bytes>, decode: Decode) -> Result<Texture, Failure> {
    let bytes = bytes.into();
    let mut steps = Vec::new();
    if bytes.len() > 3 && bytes.starts_with(b"DDS ") {
        match load_dds::load(&bytes)? {
            Some((texture, zero)) => return finish(texture, zero, decode),
            None => steps.push("DDS decoding refuses its DXT2/DXT4 code".to_owned()),
        }
    } else {
        steps.push("DDS probe finds no `DDS ` magic".to_owned());
    }
    if names_tga(name) {
        match engine_tga::load(&bytes)? {
            Ok((texture, zero)) => return finish(texture, zero, decode),
            Err(message) => steps.push(format!("Targa decoding: \"{message}\"")),
        }
    }
    match standard::load(&bytes, names_tga(name))? {
        Ok((texture, zero)) => finish(texture, zero, decode),
        Err(reason) => {
            steps.push(reason);
            Err(Failure::Undecodable(steps.join("; ")))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Zero {
    Opaque,
    Transparent,
}

impl Zero {
    fn texel(self) -> [u8; 4] {
        match self {
            Self::Opaque => [0, 0, 0, 255],
            Self::Transparent => [0, 0, 0, 0],
        }
    }

    pub(super) fn of_32_bits(rgba: &[u8]) -> Self {
        if rgba.chunks_exact(4).all(|texel| texel[3] == 0xFF) {
            Self::Opaque
        } else {
            Self::Transparent
        }
    }
}

fn names_tga(name: &[u8]) -> bool {
    name.len() > 3 && name[name.len() - 4..].eq_ignore_ascii_case(b".tga")
}

fn finish(texture: Texture, zero: Zero, decode: Decode) -> Result<Texture, Failure> {
    let texture = if decode.force_pow2_textures {
        pad_power_of_two(texture, zero)?
    } else {
        texture
    };
    if texture.width == 0 || texture.height == 0 {
        return Err(zero_extent(texture.width, texture.height));
    }
    oversized(texture.width, texture.height)?;
    Ok(texture)
}

fn power_of_two(side: u32) -> u32 {
    let mut power = 1_u32;
    while power < side {
        power *= 2;
    }
    power
}

pub(super) fn pad_power_of_two(mut texture: Texture, zero: Zero) -> Result<Texture, Failure> {
    let (width, height) = (power_of_two(texture.width), power_of_two(texture.height));
    if (width, height) == (texture.width, texture.height) {
        return Ok(texture);
    }
    let (unit_side, unit) = texture.format.unit();
    let units = |texels: u32| (texels as usize).div_ceil(unit_side as usize);
    for face in &mut texture.mips {
        for (index, level) in face.iter_mut().enumerate() {
            let shift = |side: u32| side.checked_shr(index as u32).unwrap_or(0);
            let (padded_width, padded_height) = (shift(width), shift(height));
            if padded_width == 0 || padded_height == 0 {
                return Err(Failure::Undefined(format!(
                    "force_pow2_textures pads a {}x{} image to {width}x{height}, whose level \
                     {index} is {padded_width}x{padded_height}: power-of-two padding \
                     allocates it no texels, and texture creation uploads a texel from that \
                     empty buffer",
                    texture.width, texture.height
                )));
            }
            let (rows, row_bytes, pitch) = (
                units(level.height),
                units(level.width) * unit,
                units(padded_width) * unit,
            );
            let size = pitch * units(padded_height);
            let mut data = match texture.format {
                TextureFormat::Rgba8 => zero.texel().repeat(size / 4),
                _ => vec![0; size],
            };
            for row in 0..rows {
                data[row * pitch..row * pitch + row_bytes]
                    .copy_from_slice(&level.data[row * row_bytes..(row + 1) * row_bytes]);
            }
            *level = MipLevel {
                width: padded_width,
                height: padded_height,
                data: data.into(),
            };
        }
    }
    texture.width = width;
    texture.height = height;
    Ok(texture)
}

fn zero_extent(width: u32, height: u32) -> Failure {
    Failure::Undecodable(format!(
        "decodes to a {width}x{height} image, which texture creation refuses"
    ))
}

fn oversized(width: u32, height: u32) -> Result<(), Failure> {
    if width > DEVICE_LIMIT || height > DEVICE_LIMIT {
        return Err(Failure::Device(format!(
            "a {width}x{height} texture, past the {DEVICE_LIMIT} texels a feature-level-11 device \
             creates: whether texture creation succeeds depends on the device"
        )));
    }
    if let Some((max_texels, padded)) = TEXEL_BUDGET.get() {
        let [kept_width, kept_height] = if padded {
            [power_of_two(width), power_of_two(height)]
        } else {
            [width, height]
        };
        if u64::from(kept_width) * u64::from(kept_height) > max_texels {
            return Err(Failure::OverBudget(format!(
                "a {width}x{height} image ({kept_width}x{kept_height} as decoded) exceeds the \
                 {max_texels}-texel decode budget"
            )));
        }
    }
    Ok(())
}

fn rgba(width: u32, height: u32, pixels: Vec<u8>) -> Texture {
    Texture::from_rgba8(super::Rgba8Image {
        width,
        height,
        pixels,
    })
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests_support {
    pub use super::load_dds::tests::dx10;

    pub fn png_rgba(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = ::png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(::png::ColorType::Rgba);
        encoder.set_depth(::png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels)
            .unwrap();
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{tests_support::png_rgba, *};
    use sha2::{Digest, Sha256};

    #[test]
    fn jpeg_forms_keep_the_ijg_output() {
        let cases: [(&str, &[u8], u32, u32, &str); 5] = [
            (
                "adobe_cmyk.jpg",
                include_bytes!("testdata/adobe_cmyk.jpg"),
                9,
                7,
                "7d4250e42ffb241bd1073298172218da999a181226349948fe455c5d5df103cb",
            ),
            (
                "arithmetic.jpg",
                include_bytes!("testdata/arithmetic.jpg"),
                13,
                11,
                "55a65a90588123cc8dff52e42bf761e81f22dcd53de87f7bc752e7d66be1ce4c",
            ),
            (
                "baseline_420.jpg",
                include_bytes!("testdata/baseline_420.jpg"),
                13,
                11,
                "0ae4244c09d810f0556cdd7553871352eb7d7a63a3ccdd82eec7960263d2eb66",
            ),
            (
                "progressive.jpg",
                include_bytes!("testdata/progressive.jpg"),
                13,
                11,
                "55a65a90588123cc8dff52e42bf761e81f22dcd53de87f7bc752e7d66be1ce4c",
            ),
            (
                "smartscale_4.jpg",
                include_bytes!("testdata/smartscale_4.jpg"),
                13,
                11,
                "28507c23ed019ea69a994201d15c49da96a3b03c0b66316b0f82232b6adb5f0b",
            ),
        ];
        for (name, bytes, width, height, digest) in cases {
            let texture = decode_texture_image(name.as_bytes(), bytes, Decode::default()).unwrap();
            assert_eq!((texture.width, texture.height), (width, height), "{name}");
            let hash: String = Sha256::digest(&texture.base().data)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(hash, digest, "{name}");
        }
    }

    fn pixels(texture: &Texture) -> &[u8] {
        assert_eq!(texture.format, TextureFormat::Rgba8);
        &texture.base().data
    }

    #[test]
    fn png_bytes_decode_under_every_name() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 40];
        let file = png_rgba(2, 2, &rgba);
        for name in [&b"gfx/mod/a.png"[..], b"gfx/mod/a.TGA", b"gfx/mod/a.dds"] {
            let texture = decode_texture_image(name, &file, Decode::default()).unwrap();
            assert_eq!((texture.width, texture.height), (2, 2));
            assert_eq!(pixels(&texture), rgba);
        }
    }

    #[test]
    fn a_texel_budget_refuses_from_the_header_before_decoding() {
        let file = png_rgba(5, 5, &[7; 100]);
        let name = b"gfx/mod/a.png";
        let padded = Decode {
            force_pow2_textures: true,
        };
        assert!(decode_texture_image_within(name, &file, Decode::default(), 25).is_ok());
        assert!(matches!(
            decode_texture_image_within(name, &file, padded, 25),
            Err(Failure::OverBudget(why)) if why.contains("5x5 image (8x8 as decoded)")
        ));
        assert!(decode_texture_image_within(name, &file, padded, 64).is_ok());
        assert!(TEXEL_BUDGET.get().is_none());
        let mut claim = file.clone();
        claim[16..24].copy_from_slice(&[0, 0, 0x3E, 0x80, 0, 0, 0x3E, 0x80]);
        let crc = crc32(&claim[12..29]);
        claim[29..33].copy_from_slice(&crc.to_be_bytes());
        assert!(matches!(
            decode_texture_image_within(name, &claim, Decode::default(), 64 << 20),
            Err(Failure::OverBudget(why)) if why.contains("16000x16000")
        ));
    }

    fn crc32(bytes: &[u8]) -> u32 {
        !bytes.iter().fold(!0_u32, |crc, &byte| {
            (0..8).fold(crc ^ u32::from(byte), |crc, _| {
                (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg())
            })
        })
    }

    #[test]
    fn decoder_failures_are_named_apart_from_undefined_outcomes() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0];
        assert!(matches!(
            decode_texture_image(b"gfx/a.dds", &jpeg, Decode::default()),
            Err(Failure::Undecodable(why)) if why.contains("libjpeg")
        ));
        assert!(matches!(
            decode_texture_image(b"gfx/a.dds", &[0, 0, 1], Decode::default()),
            Err(Failure::Undecodable(_))
        ));
        let garbage = [
            0x13, 7, 0x55, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        ];
        for name in [&b"gfx/a.tga"[..], b"gfx/a.dds"] {
            assert!(matches!(
                decode_texture_image(name, &garbage, Decode::default()),
                Err(Failure::Undecodable(_))
            ));
        }
    }

    fn bmp(width: i32, height: i32, bpp: u16, compression: u32, pixels: &[u8]) -> Vec<u8> {
        let offset = 14 + 40 + 8;
        let mut file = b"BM".to_vec();
        file.extend_from_slice(&((offset + pixels.len()) as u32).to_le_bytes());
        file.extend_from_slice(&[0; 4]);
        file.extend_from_slice(&(offset as u32).to_le_bytes());
        file.extend_from_slice(&40_u32.to_le_bytes());
        file.extend_from_slice(&width.to_le_bytes());
        file.extend_from_slice(&height.to_le_bytes());
        file.extend_from_slice(&1_u16.to_le_bytes());
        file.extend_from_slice(&bpp.to_le_bytes());
        file.extend_from_slice(&compression.to_le_bytes());
        file.extend_from_slice(&[0; 12]);
        file.extend_from_slice(&2_u32.to_le_bytes());
        file.extend_from_slice(&0_u32.to_le_bytes());
        file.extend_from_slice(&[10, 20, 30, 0, 40, 50, 60, 0]);
        file.extend_from_slice(pixels);
        file
    }

    #[test]
    fn bmp_fallback_decodes_palette_entries_and_refuses_invalid_compression() {
        let decoded = decode_texture_image(
            b"gfx/a.dds",
            &bmp(2, 1, 8, 0, &[0, 1, 0, 0]),
            Decode::default(),
        )
        .unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(pixels(&decoded), [30, 20, 10, 255, 60, 50, 40, 255]);
        assert!(matches!(
            decode_texture_image(b"gfx/a.bmp", bmp(5, 1, 1, 1, &[5, 1]), Decode::default()),
            Err(Failure::Undecodable(_))
        ));
    }

    #[test]
    fn force_pow2_pads_to_the_next_power_of_two() {
        let pixels: Vec<u8> = (0..15_u8).flat_map(|i| [i, i + 1, i + 2, 128]).collect();
        let png = png_rgba(3, 5, &pixels);
        let off = decode_texture_image(b"gfx/a.png", &png, Decode::default()).unwrap();
        assert_eq!((off.width, off.height), (3, 5));
        let on = Decode {
            force_pow2_textures: true,
        };
        let padded = decode_texture_image(b"gfx/a.png", &png, on).unwrap();
        assert_eq!((padded.width, padded.height), (4, 8));
        let data = &padded.base().data;
        assert_eq!(data.len(), 4 * 8 * 4);
        for y in 0..8 {
            for x in 0..4 {
                let texel = &data[(y * 4 + x) * 4..(y * 4 + x + 1) * 4];
                if x < 3 && y < 5 {
                    assert_eq!(texel, &pixels[(y * 3 + x) * 4..(y * 3 + x + 1) * 4]);
                } else {
                    assert_eq!(texel, [0, 0, 0, 0]);
                }
            }
        }
        let mut tga = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 5, 0, 24, 0x20];
        tga.extend((0..15_u8).flat_map(|i| [i, i, i]));
        let padded = decode_texture_image(b"gfx/a.tga", &tga, on).unwrap();
        assert_eq!((padded.width, padded.height), (4, 8));
        assert_eq!(&padded.base().data[3 * 4..4 * 4], [0, 0, 0, 255]);
        assert_eq!(&padded.base().data[..4], [0, 0, 0, 255]);
        assert_eq!(&padded.base().data[4..8], [1, 1, 1, 255]);
        let square = png_rgba(2, 2, &[7; 16]);
        assert_eq!(
            decode_texture_image(b"gfx/b.png", &square, on).unwrap(),
            decode_texture_image(b"gfx/b.png", &square, Decode::default()).unwrap()
        );
        let mut empty = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 24, 0x20];
        empty.extend([0; 3]);
        assert!(matches!(
            decode_texture_image(b"gfx/c.tga", &empty, Decode::default()),
            Err(Failure::Undecodable(_))
        ));
        let widened = decode_texture_image(b"gfx/c.tga", &empty, on).unwrap();
        assert_eq!((widened.width, widened.height), (1, 4));
        assert_eq!(widened.base().data, [0, 0, 0, 255].repeat(4));
    }

    #[test]
    fn padding_keeps_the_chain_and_pads_blocks_whole() {
        let level = |width: u32, height: u32, data: Vec<u8>| MipLevel {
            width,
            height,
            data: data.into(),
        };
        let texture = Texture {
            format: TextureFormat::Rgba8,
            width: 3,
            height: 5,
            faces: 1,
            mips: vec![vec![
                level(3, 5, [1, 1, 1, 1].repeat(15)),
                level(1, 2, [2, 2, 2, 2].repeat(2)),
            ]],
        };
        let padded = pad_power_of_two(texture, Zero::Opaque).unwrap();
        let sides: Vec<_> = padded.mips[0].iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sides, [(4, 8), (2, 4)]);
        assert_eq!(
            padded.mips[0][1].data,
            [[2, 2, 2, 2], [0, 0, 0, 255], [2, 2, 2, 2], [0, 0, 0, 255]]
                .concat()
                .into_iter()
                .chain([0, 0, 0, 255].repeat(4))
                .collect::<Vec<_>>()
        );
        let blocks = Texture {
            format: TextureFormat::Bc1,
            width: 12,
            height: 4,
            faces: 1,
            mips: vec![vec![level(12, 4, [0xAB; 24].to_vec())]],
        };
        let padded = pad_power_of_two(blocks, Zero::Transparent).unwrap();
        assert_eq!((padded.width, padded.height), (16, 4));
        let mut expected = vec![0xAB; 24];
        expected.extend([0; 8]);
        assert_eq!(padded.base().data, expected);
        let words = Texture {
            format: TextureFormat::B5G6R5,
            width: 3,
            height: 1,
            faces: 1,
            mips: vec![vec![level(3, 1, vec![1, 2, 3, 4, 5, 6])]],
        };
        let padded = pad_power_of_two(words, Zero::Opaque).unwrap();
        assert_eq!(padded.base().data, [1, 2, 3, 4, 5, 6, 0, 0]);
        assert_eq!(padded.format.sample_16(0), Some([0.0, 0.0, 0.0, 1.0]));
        let thin = Texture {
            format: TextureFormat::Rgba8,
            width: 0,
            height: 3,
            faces: 1,
            mips: vec![vec![level(0, 3, Vec::new()), level(0, 1, Vec::new())]],
        };
        assert!(matches!(
            pad_power_of_two(thin, Zero::Opaque),
            Err(Failure::Undefined(why)) if why.contains("power-of-two padding")
        ));
    }
}
