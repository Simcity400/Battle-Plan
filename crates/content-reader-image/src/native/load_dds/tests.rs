use super::*;

fn header(
    width: u32,
    height: u32,
    mips: u32,
    flags: u32,
    fourcc: &[u8; 4],
    bits: u32,
    masks: [u32; 4],
) -> Vec<u8> {
    let mut bytes = vec![0_u8; HEADER];
    bytes[..4].copy_from_slice(b"DDS ");
    let mut put =
        |offset: usize, value: u32| bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    put(4, 124);
    put(12, height);
    put(16, width);
    put(28, mips);
    put(76, 32);
    put(80, flags);
    put(84, u32::from_le_bytes(*fourcc));
    put(88, bits);
    for (index, mask) in masks.into_iter().enumerate() {
        put(92 + index * 4, mask);
    }
    bytes
}

#[test]
fn block_extents_round_up_to_whole_blocks() {
    let mut file = header(5, 3, 9, 4, b"DXT5", 0, [0; 4]);
    file.extend(std::iter::repeat_n(0xAB, 16 * 2 + 16 + 16));
    let texture = load(&file).unwrap().unwrap().0;
    assert_eq!((texture.width, texture.height), (8, 4));
    let sides: Vec<_> = texture.mips[0]
        .iter()
        .map(|mip| (mip.width, mip.height))
        .collect();
    assert_eq!(sides, [(8, 4), (4, 2), (2, 1)]);
    assert_eq!(load(&header(4, 4, 1, 4, b"DXT2", 0, [0; 4])).unwrap(), None);
}

#[test]
fn a_cube_map_holds_six_chains_in_face_order() {
    let mut file = header(6, 5, 2, 4, b"DXT1", 0, [0; 4]);
    file[112..116].copy_from_slice(&0xFE00_u32.to_le_bytes());
    for face in 0..6_u8 {
        file.extend(std::iter::repeat_n(face * 2, 4 * 8));
        file.extend(std::iter::repeat_n(face * 2 + 1, 8));
    }
    let texture = load(&file).unwrap().unwrap().0;
    assert_eq!((texture.faces, texture.width, texture.height), (6, 8, 8));
    for (face, levels) in texture.mips.iter().enumerate() {
        let sides: Vec<_> = levels.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sides, [(8, 8), (4, 4)]);
        assert!(levels[0].data.iter().all(|&b| b == face as u8 * 2));
        assert!(levels[1].data.iter().all(|&b| b == face as u8 * 2 + 1));
    }
    file.pop();
    assert!(matches!(load(&file), Err(Failure::Undefined(why)) if why.contains("face 5")));
}

#[test]
fn a_zero_extent_cube_keeps_two_empty_levels_a_face() {
    let mut file = header(0, 0, u32::MAX, 4, b"DXT1", 0, [0; 4]);
    file[112..116].copy_from_slice(&0xFE00_u32.to_le_bytes());
    let texture = load(&file).unwrap().unwrap().0;
    assert_eq!((texture.faces, texture.width, texture.height), (6, 0, 0));
    assert!(texture.mips.iter().all(|levels| levels.len() == 2));
    let decoded = |force_pow2_textures| {
        super::super::decode_texture_image(
            b"cube.dds",
            &file,
            super::super::Decode {
                force_pow2_textures,
            },
        )
    };
    assert!(matches!(decoded(false), Err(Failure::Undecodable(why)) if why.contains("0x0")));
    assert!(matches!(decoded(true), Err(Failure::Undefined(why)) if why.contains("level 1")));
}

pub fn dx10(width: u32, height: u32, mips: u32, dxgi: u32) -> Vec<u8> {
    let mut file = header(width, height, mips, 4, b"DX10", 0, [0; 4]);
    file[8..12].copy_from_slice(&0x2_0000_u32.to_le_bytes());
    for word in [dxgi, 3, 0, 1, 0] {
        file.extend(word.to_le_bytes());
    }
    file
}

#[test]
fn bc7_keeps_logical_mips_and_exact_payload_offsets() {
    for (dxgi, srgb) in [(98, false), (99, true)] {
        let mut file = dx10(10, 6, 4, dxgi);
        let expected = [
            (10, 6, 96, 148),
            (5, 3, 32, 244),
            (2, 1, 16, 276),
            (1, 1, 16, 292),
        ];
        for (level, &(_, _, size, offset)) in expected.iter().enumerate() {
            assert_eq!(file.len(), offset);
            file.extend(std::iter::repeat_n(level as u8 + 1, size));
        }
        let (texture, zero) = load(&file).unwrap().unwrap();
        assert_eq!((texture.width, texture.height, texture.faces), (10, 6, 1));
        assert_eq!(texture.format, TextureFormat::Bc7 { srgb });
        assert_eq!(zero, Zero::Transparent);
        assert_eq!(texture.mip_count(), 4);
        for (mip, &(width, height, size, offset)) in texture.mips[0].iter().zip(&expected) {
            assert_eq!(
                (mip.width, mip.height, mip.data.len()),
                (width, height, size)
            );
            assert_eq!(mip.data, file[offset..offset + size]);
        }
    }
}

#[test]
fn bc7_force_pow2_copies_logical_block_rows_at_the_padded_pitch() {
    use super::super::{Decode, decode_texture_image};

    for (dxgi, srgb) in [(98, false), (99, true)] {
        let mut file = dx10(10, 6, 4, dxgi);
        let data: Vec<Vec<u8>> = [96, 32, 16, 16]
            .into_iter()
            .enumerate()
            .map(|(level, size)| {
                (0..size)
                    .map(|byte| (level * 32 + byte + 1) as u8)
                    .collect()
            })
            .collect();
        for bytes in &data {
            file.extend(bytes);
        }
        let padded = decode_texture_image(
            b"gfx/padded-bc7.dds",
            &file,
            Decode {
                force_pow2_textures: true,
            },
        )
        .unwrap();
        assert_eq!((padded.width, padded.height, padded.faces), (16, 8, 1));
        assert_eq!(padded.format, TextureFormat::Bc7 { srgb });
        assert_eq!(padded.mip_count(), 4);
        let expected = [
            ((10, 6), (16, 8)),
            ((5, 3), (8, 4)),
            ((2, 1), (4, 2)),
            ((1, 1), (2, 1)),
        ];
        let unpadded =
            decode_texture_image(b"gfx/padded-bc7.dds", &file, Decode::default()).unwrap();
        for (index, &(logical_extent, padded_extent)) in expected.iter().enumerate() {
            let original = &unpadded.mips[0][index];
            assert_eq!((original.width, original.height), logical_extent);
            assert_eq!(original.data, data[index]);
            let mip = &padded.mips[0][index];
            assert_eq!((mip.width, mip.height), padded_extent);
            let source_pitch = original.width.div_ceil(4) as usize * 16;
            let padded_pitch = mip.width.div_ceil(4) as usize * 16;
            let rows = mip.height.div_ceil(4) as usize;
            assert_eq!(mip.data.len(), padded_pitch * rows);
            for (row, bytes) in mip.data.chunks_exact(padded_pitch).enumerate() {
                assert_eq!(
                    bytes[..source_pitch],
                    data[index][row * source_pitch..(row + 1) * source_pitch],
                    "level {index}, row {row}"
                );
                assert!(bytes[source_pitch..].iter().all(|&byte| byte == 0));
            }
        }
    }
}

#[test]
fn bc7_uses_the_mip_count_only_when_the_header_declares_it() {
    for (flags, count, levels) in [(0, 4, 1), (0x2_0000, 0, 1), (0x2_0000, 2, 2)] {
        let mut file = dx10(10, 6, count, 98);
        file[8..12].copy_from_slice(&u32::to_le_bytes(flags));
        file.extend([1; 96]);
        file.extend([2; 32]);
        let texture = load(&file).unwrap().unwrap().0;
        assert_eq!(texture.mip_count(), levels);
    }
}

#[test]
fn bc7_refuses_every_truncated_declared_level_by_name() {
    let mut file = dx10(10, 6, 4, 98);
    file.extend([0; 160]);
    for (length, level) in [(243, 0), (275, 1), (291, 2), (307, 3)] {
        let error = load(&file[..length]).unwrap_err();
        assert!(
            matches!(error, Failure::Undecodable(ref why)
                if why.contains("DX10 BC7 DDS")
                    && why.contains("truncated declared mip chain")
                    && why.contains(&format!("level {level}"))),
            "{error:?}"
        );
    }
    assert!(matches!(load(&file[..132]), Err(Failure::Undecodable(why))
        if why.contains("BC7") && why.contains("truncated DX10 header")));
}

#[test]
fn bc7_refuses_unreached_cube_array_and_volume_forms_by_name() {
    let file = dx10(10, 6, 4, 98);
    for (offset, value, reason) in [
        (112, CUBE_MAP, "cube maps"),
        (136, 4, "cube maps"),
        (140, 2, "array size 2"),
        (132, 4, "resource dimension 4"),
    ] {
        let mut invalid = file.clone();
        invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(matches!(load(&invalid), Err(Failure::Undecodable(why))
            if why.contains("BC7") && why.contains(reason)));
    }
}

#[test]
fn a_32_bit_surface_is_bgra_whatever_its_masks() {
    let mut file = header(2, 2, 5, 0x40, b"\0\0\0\0", 32, [0xFF, 0xFF00, 0xFF_0000, 0]);
    file.extend([
        1, 2, 3, 0, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 9, 9, 9,
    ]);
    let texture = load(&file).unwrap().unwrap().0;
    assert_eq!(texture.mips[0].len(), 1);
    assert_eq!(
        texture.base().data,
        [3, 2, 1, 0, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15]
    );
}

#[test]
fn sixteen_bit_forms() {
    let texels = [0x7FFF_u16, 0x001F].map(u16::to_le_bytes).concat();
    let mut x555 = header(2, 1, 1, 0x40, b"\0\0\0\0", 16, [0x7C00, 0x3E0, 0x1F, 0]);
    x555.extend(&texels);
    let x555 = load(&x555).unwrap().unwrap().0;
    assert_eq!(x555.format, TextureFormat::B5G5R5A1);
    assert_eq!(x555.base().data, [0xFF, 0xFF, 0x1F, 0x80]);
    let mut a1555 = header(
        2,
        1,
        1,
        0x41,
        b"\0\0\0\0",
        16,
        [0x7C00, 0x3E0, 0x1F, 0x8000],
    );
    a1555.extend([0x00_u8, 0x80, 0x10, 0x02]);
    let a1555 = load(&a1555).unwrap().unwrap().0;
    assert_eq!(a1555.format, TextureFormat::B5G5R5A1);
    assert_eq!(a1555.base().data, [0x00, 0x80, 0x10, 0x02]);
    let word = |texture: &Texture, index: usize| {
        let data = &texture.base().data;
        u16::from_le_bytes([data[2 * index], data[2 * index + 1]])
    };
    assert_eq!(
        a1555.format.sample_16(word(&a1555, 1)),
        Some([0.0, 16.0 / 31.0, 16.0 / 31.0, 0.0])
    );
    let mut r565 = header(1, 1, 1, 0x40, b"\0\0\0\0", 16, [0xF800, 0x7E0, 0x1F, 0]);
    r565.extend(0x0821_u16.to_le_bytes());
    let r565 = load(&r565).unwrap().unwrap().0;
    assert_eq!(r565.format, TextureFormat::B5G6R5);
    let sampled = r565.format.sample_16(word(&r565, 0)).unwrap();
    assert_eq!(sampled, [1.0 / 31.0, 1.0 / 63.0, 1.0 / 31.0, 1.0]);
    assert_ne!(sampled[0], 8.0 / 255.0);
    let mut other = header(1, 1, 1, 0x41, b"\0\0\0\0", 16, [0xF00, 0xF0, 0xF, 0xF000]);
    other.extend([0, 0]);
    assert!(matches!(load(&other), Err(Failure::Undefined(_))));
}

#[test]
fn dx10_uncompressed_formats_keep_exact_channel_order() {
    for dxgi in [87, 91] {
        let mut file = dx10(2, 1, 1, dxgi);
        file.extend([10, 20, 30, 40, 50, 60, 70, 80]);
        let (texture, zero) = load(&file).unwrap().unwrap();
        assert_eq!(texture.format, TextureFormat::Rgba8);
        assert_eq!(zero, Zero::Transparent);
        assert_eq!(texture.base().data, [30, 20, 10, 40, 70, 60, 50, 80]);
    }
    for dxgi in [28, 29] {
        let mut file = dx10(2, 1, 1, dxgi);
        file.extend([10, 20, 30, 40, 50, 60, 70, 80]);
        let (texture, zero) = load(&file).unwrap().unwrap();
        assert_eq!(texture.format, TextureFormat::Rgba8);
        assert_eq!(zero, Zero::Transparent);
        assert_eq!(texture.base().data, [10, 20, 30, 40, 50, 60, 70, 80]);
    }
    for dxgi in [88, 93] {
        let mut file = dx10(1, 1, 1, dxgi);
        file.extend([10, 20, 30, 0]);
        let (texture, zero) = load(&file).unwrap().unwrap();
        assert_eq!(texture.format, TextureFormat::Rgba8);
        assert_eq!(zero, Zero::Opaque);
        assert_eq!(texture.base().data, [30, 20, 10, 255]);
    }
}

#[test]
fn dx10_bc_formats_decode_as_blocks_and_unsupported_dxgi_is_refused_by_name() {
    for (dxgi, expected, block_bytes) in [
        (71, TextureFormat::Bc1, 8),
        (72, TextureFormat::Bc1, 8),
        (74, TextureFormat::Bc2, 16),
        (77, TextureFormat::Bc3, 16),
        (78, TextureFormat::Bc3, 16),
    ] {
        let mut file = dx10(4, 4, 1, dxgi);
        file.extend(std::iter::repeat_n(0x5A, block_bytes));
        let (texture, _) = load(&file).unwrap().unwrap();
        assert_eq!(texture.format, expected);
        assert_eq!((texture.width, texture.height), (4, 4));
        assert_eq!(texture.base().data.len(), block_bytes);
    }
    for unsupported in [10, 61, 80, 83, 95] {
        let mut file = dx10(4, 4, 1, unsupported);
        file.extend([0; 64]);
        assert!(
            matches!(load(&file), Err(Failure::Unsupported(ref why)) if why.contains(&format!("DXGI format {unsupported}"))),
            "expected named refusal for DXGI format {unsupported}"
        );
    }
}
