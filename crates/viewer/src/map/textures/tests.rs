use installed_content::map::{CountryColors, ProvinceDefinition};

use super::*;

const POW2: Decode = Decode {
    force_pow2_textures: true,
};

/// A DDS header for `width` x `height` over `mips` levels: 32-bit BGRA, or `fourcc` blocks.
fn dds_header(width: u32, height: u32, mips: u32, fourcc: Option<&[u8; 4]>) -> Vec<u8> {
    let mut bytes = vec![0_u8; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    let (flags, code, bits) = match fourcc {
        Some(code) => (4, u32::from_le_bytes(*code), 0),
        None => (0x41, 0, 32),
    };
    for (offset, value) in [
        (4, 124),
        (12, height),
        (16, width),
        (28, mips),
        (76, 32),
        (80, flags),
        (84, code),
        (88, bits),
        (92, 0xFF_0000),
        (96, 0xFF00),
        (100, 0xFF),
        (104, 0xFF00_0000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
    }
    bytes
}

/// A 32-bit uncompressed DDS of `width` x `height` texels, every byte `fill`.
fn dds_file(width: u32, height: u32, fill: u8) -> Vec<u8> {
    let mut bytes = dds_header(width, height, 1, None);
    bytes.extend(std::iter::repeat_n(fill, (width * height * 4) as usize));
    bytes
}

fn dx10_file(width: u32, height: u32, mips: u32, dxgi: u32) -> Vec<u8> {
    let mut file = dds_header(width, height, mips, Some(b"DX10"));
    file[8..12].copy_from_slice(&0x2_0000_u32.to_le_bytes());
    for word in [dxgi, 3, 0, 1, 0] {
        file.extend(word.to_le_bytes());
    }
    file
}

#[test]
fn bc7_map_and_gui_share_logical_mips_and_header_colour_space() {
    for (dxgi, format) in [
        (98, wgpu::TextureFormat::Bc7RgbaUnorm),
        (99, wgpu::TextureFormat::Bc7RgbaUnormSrgb),
    ] {
        let mut file = dx10_file(10, 6, 4, dxgi);
        for (level, size) in [96, 32, 16, 16].into_iter().enumerate() {
            file.extend(std::iter::repeat_n(level as u8 + 1, size));
        }
        let read = files(vec![("gfx/test.dds", file.clone())]);
        let texture = load_file("gfx/test.dds", &read, Decode::default()).unwrap();
        let gui = resources::texture(b"gfx/test.dds", &read, Decode::default())
            .unwrap()
            .unwrap();
        assert_eq!(texture, gui.image);
        assert_eq!(
            (texture.width, texture.height, texture.mip_count()),
            (10, 6, 4)
        );
        for requested in [ColorSpace::Linear, ColorSpace::Srgb] {
            assert_eq!(format_of(texture.format, requested).unwrap(), format);
        }
        // A short chain names the resource as well as the affected BC7 level.
        file.pop();
        let read = files(vec![("gfx/test.dds", file)]);
        let why = load_file("gfx/test.dds", &read, Decode::default())
            .unwrap_err()
            .to_string();
        assert!(why.contains("gfx/test.dds") && why.contains("BC7") && why.contains("level 3"));
    }
}

/// BC5 and BC6H are still the engine's BGRA fall-through, including its first-level
/// overrun failure, through the same map resolver. The old parallel DDS path refused their
/// dxgiFormat instead of reaching this native outcome.
#[test]
fn other_dx10_formats_keep_the_native_bgra_outcome_by_file_name() {
    for dxgi in [83, 95] {
        let mut file = dx10_file(4, 4, 1, dxgi);
        file.extend([0; 16]);
        let read = files(vec![("gfx/other.dds", file)]);
        let why = load_file("gfx/other.dds", &read, Decode::default())
            .unwrap_err()
            .to_string();
        assert!(why.contains("gfx/other.dds") && why.contains("first level runs past the file"));
        assert!(!why.contains("BC7"));

        let mut file = dx10_file(1, 1, 1, dxgi);
        file.extend([1, 2, 3, 4]);
        let read = files(vec![("gfx/other.dds", file)]);
        let texture = load_file("gfx/other.dds", &read, Decode::default()).unwrap();
        assert_eq!(texture.format, dds::TextureFormat::Rgba8);
        assert_eq!(texture.base().data, [3, 2, 1, 4]);
    }
}

/// Reads the named files, every other path absent.
fn files(files: Vec<(&'static str, Vec<u8>)>) -> impl Fn(&[u8]) -> io::Result<Vec<u8>> {
    move |path| {
        files
            .iter()
            .find(|(name, _)| name.as_bytes() == path)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

/// A texture file the passes load pads to powers of two only under the client's
/// `force_pow2_textures`, the image in the top-left corner and the rest the 32-bit format's
/// zeroed texel, transparent black; a power-of-two file is the same either way. A name no
/// layer holds is `NotFound`.
#[test]
fn texture_files_pad_only_under_force_pow2_textures() {
    let read = files(vec![
        ("map/a.dds", dds_file(3, 2, 9)),
        ("map/b.dds", dds_file(4, 2, 9)),
    ]);
    let off = load_file("map/a.dds", &read, Decode::default()).unwrap();
    assert_eq!((off.width, off.height), (3, 2));
    let on = load_file("map/a.dds", &read, POW2).unwrap();
    assert_eq!((on.width, on.height), (4, 2));
    for row in on.base().data.chunks_exact(16) {
        assert_eq!(row[..12], [9; 12]);
        assert_eq!(row[12..], [0; 4]);
    }
    assert_eq!(
        load_file("map/b.dds", &read, POW2).unwrap(),
        load_file("map/b.dds", &read, Decode::default()).unwrap()
    );
    let missing = load_file("map/c.dds", &read, Decode::default()).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
}

/// Synthetic 256-by-73 DXT5 fixture with nine mips. Block extent rounds to 256-by-76; later levels
/// halve that extent, placing level two at byte 24704. Power-of-two decoding pads levels to
/// 256-by-128 and below.
#[test]
fn a_non_block_aligned_mip_chain_loads_as_the_game_reads_it() {
    let sides = [
        (256, 76),
        (128, 38),
        (64, 19),
        (32, 9),
        (16, 4),
        (8, 2),
        (4, 1),
    ];
    let mut file = dds_header(256, 73, 9, Some(b"DXT5"));
    let mut offsets = Vec::new();
    for (level, (width, height)) in sides.iter().enumerate() {
        offsets.push(file.len());
        let bytes = (*width as usize).div_ceil(4) * (*height as usize).div_ceil(4) * 16;
        file.extend(std::iter::repeat_n(level as u8 + 1, bytes));
    }
    assert_eq!(offsets[2], 24704);
    let read = files(vec![("gfx/models/units/FAL.dds", file.clone())]);
    let loaded = load_file("gfx/models/units/FAL.dds", &read, Decode::default()).unwrap();
    assert_eq!((loaded.width, loaded.height), (256, 76));
    let levels: Vec<_> = loaded.mips[0].iter().map(|l| (l.width, l.height)).collect();
    assert_eq!(levels, sides);
    for (level, mip) in loaded.mips[0].iter().enumerate() {
        assert!(
            mip.data.iter().all(|&byte| byte == level as u8 + 1),
            "level {level}"
        );
    }
    let padded = load_file("gfx/models/units/FAL.dds", &read, POW2).unwrap();
    let levels: Vec<_> = padded.mips[0].iter().map(|l| (l.width, l.height)).collect();
    assert_eq!(
        levels,
        [
            (256, 128),
            (128, 64),
            (64, 32),
            (32, 16),
            (16, 8),
            (8, 4),
            (4, 2)
        ]
    );
    // Level 2's five block rows are the game's, then zeroed blocks.
    let row = 16 * 16;
    assert!(
        padded.mips[0][2].data[..5 * row]
            .iter()
            .all(|&byte| byte == 3)
    );
    assert!(
        padded.mips[0][2].data[5 * row..]
            .iter()
            .all(|&byte| byte == 0)
    );
}

/// An 8-bit `width` x `height` bitmap whose indices are `indices` (rows top first) over a
/// grey palette.
fn indexed_bmp(width: u32, height: u32, indices: &[u8]) -> Vec<u8> {
    let stride = (width as usize).div_ceil(4) * 4;
    let pixels = 54 + 1024;
    let mut bytes = Vec::new();
    bytes.extend(b"BM");
    bytes.extend(u32::to_le_bytes((pixels + stride * height as usize) as u32));
    bytes.extend([0; 4]);
    bytes.extend(u32::to_le_bytes(pixels as u32));
    for value in [40, width, height] {
        bytes.extend(u32::to_le_bytes(value));
    }
    bytes.extend(u16::to_le_bytes(1));
    bytes.extend(u16::to_le_bytes(8));
    bytes.extend([0; 24]);
    for grey in 0..=255_u8 {
        bytes.extend([grey, grey, grey, 0]);
    }
    for row in indices.chunks_exact(width as usize).rev() {
        bytes.extend(row);
        bytes.extend(std::iter::repeat_n(0, stride - row.len()));
    }
    bytes
}

/// The heightmap the passes sample loads as a texture file: over a grey palette it is the
/// vertex heights as they are, and under `force_pow2_textures` it pads, its padding reading 0.
#[test]
fn the_sampled_heightmap_loads_as_a_texture_file() {
    let file = indexed_bmp(3, 2, &[1, 2, 3, 4, 5, 6]);
    let heightmap = bmp::decode_indexed(&file).unwrap();
    let read = files(vec![("map/heightmap.bmp", file)]);
    let sampled = |decode| {
        sampled_heights(
            &load_image("map/heightmap.bmp", &read, decode).unwrap(),
            &heightmap,
        )
    };
    assert_eq!(sampled(Decode::default()), None);
    assert_eq!(sampled(POW2), Some(([4, 2], vec![1, 2, 3, 0, 4, 5, 6, 0])));
}

/// A province image one six-by-six block wide and two tall, so one texel
/// covers one province and the block arithmetic is visible.
///
/// Province 1 fills the northern half of the image and province 2 the
/// southern one; `kinds` says which of them is land.
fn two_block_map(kinds: [ProvinceKind; 2]) -> MapData {
    let mut province_ids = vec![1u32; (FOW_BLOCK * FOW_BLOCK) as usize];
    province_ids.extend(std::iter::repeat_n(2u32, (FOW_BLOCK * FOW_BLOCK) as usize));
    let definition = |id: u32, kind: ProvinceKind| {
        (
            id,
            ProvinceDefinition {
                id,
                rgb: [id as u8; 3],
                kind,
                continent: 0,
            },
        )
    };
    MapData {
        width: FOW_BLOCK,
        height: FOW_BLOCK * 2,
        province_ids,
        definitions: [definition(1, kinds[0]), definition(2, kinds[1])]
            .into_iter()
            .collect(),
        country_colors: CountryColors::default(),
    }
}

/// Complement the truncated block mean: an unseen province byte zero becomes texture fog byte 255.
///
/// Row 0 of the result is world `z = 0`, which is the *last* row of the
/// province image, so province 2 — the image's southern half — is the first
/// texel.
#[test]
fn the_intel_texel_is_the_complement_of_the_block_mean() {
    let map = two_block_map([ProvinceKind::Land, ProvinceKind::Land]);
    let visibility: BTreeMap<u32, u8> = [(1, 0), (2, 250)].into_iter().collect();
    let built = intel_map(&map, Some(&visibility)).unwrap();
    assert!(built.fog);
    assert_eq!(built.texels, vec![5, 255]);
}

/// A province the hour does not record contributes no fog rather than the
/// recorded zero that would fog it whole; `super::super::tables` says so.
#[test]
fn an_unrecorded_province_is_not_a_recorded_zero() {
    let map = two_block_map([ProvinceKind::Land, ProvinceKind::Land]);
    let partial: BTreeMap<u32, u8> = [(2, 0)].into_iter().collect();
    assert_eq!(
        intel_map(&map, Some(&partial)).unwrap().texels,
        vec![255, 0]
    );
}

/// Sea and lake pixels contribute 0xff independently of the visibility array.
#[test]
fn sea_and_lake_provinces_never_take_fog() {
    let map = two_block_map([ProvinceKind::Sea, ProvinceKind::Lake]);
    let visibility: BTreeMap<u32, u8> = [(1, 0), (2, 0)].into_iter().collect();
    assert_eq!(
        intel_map(&map, Some(&visibility)).unwrap().texels,
        vec![0, 0]
    );
}

/// No visibility input produces an all-clear grid without fog drawing.
#[test]
fn no_recorded_visibility_is_the_engines_fog_off_branch() {
    let map = two_block_map([ProvinceKind::Land, ProvinceKind::Land]);
    let built = intel_map(&map, None).unwrap();
    assert!(!built.fog);
    assert_eq!(built.texels, vec![0, 0]);
}

fn identity_lookup() -> [u8; 256] {
    let mut lookup = [0u8; 256];
    for (index, value) in lookup.iter_mut().enumerate() {
        *value = index as u8;
    }
    lookup
}

#[test]
fn the_id_map_is_bottom_up_and_names_the_four_blend_neighbours() {
    // Two rows of three, bitmap order (row 0 is north, world z = 1).
    let indices = [1u8, 2, 3, 4, 5, 6];
    let map = terrain_id_map(3, 2, &indices, &identity_lookup()).unwrap();
    let texel = |x: usize, z: usize| &map[(z * 3 + x) * 4..][..4];
    // World z = 0 is bitmap row 1: 4, 5, 6. World z = 1 is row 0: 1, 2, 3.
    // At (x = 0, z = 0): base 4, right 5, up 1, up-right 2.
    assert_eq!(texel(0, 0), [1, 5, 4, 2]);
    // x wraps: the right neighbour of column 2 is column 0.
    assert_eq!(texel(2, 0), [3, 4, 6, 1]);
    // z clamps at the top row, so up and up-right repeat the row itself.
    assert_eq!(texel(0, 1), [1, 2, 1, 2]);
}

#[test]
fn a_uniform_neighbourhood_sets_the_all_same_flag() {
    let indices = [7u8; 9];
    let map = terrain_id_map(3, 3, &indices, &identity_lookup()).unwrap();
    for texel in map.chunks_exact(4) {
        assert_eq!(texel, [7, 7, 107, 7]);
    }
    // `calculate_map_tex_index` recovers the tile and the flag.
    let raw = 107.0f32;
    let all_same = (raw - 98.0).clamp(0.0, 1.0);
    assert_eq!(all_same, 1.0);
    assert_eq!(raw - all_same * 100.0, 7.0);
}

/// `common/terrain`'s `lakes` entry carries `texture = 255` whatever the
/// map does with palette index 14, so the warning has to be about the
/// bitmap: raised when the index is painted somewhere and silent when it is
/// not, which is the shipped map's case.
#[test]
fn the_atlas_warnings_are_raised_only_for_indices_the_bitmap_paints() {
    use installed_content::terrain::TerrainType;

    let kind = |atlas_index| TerrainType {
        name: "t".into(),
        category: None,
        atlas_index,
        perm_snow: false,
        spawn_city: false,
    };
    let mut definitions = TerrainDefinitions::default();
    definitions.palette.insert(1, kind(4));
    definitions.palette.insert(14, kind(255));
    let image = |indices: &[u8]| IndexedImage {
        width: indices.len() as u32,
        height: 1,
        indices: indices.to_vec(),
        palette: [[0; 4]; 256],
    };

    let mut quiet = Vec::new();
    let lookup = atlas_lookup(&definitions, &image(&[1, 1, 1]), &mut quiet);
    assert_eq!(lookup[1], 4);
    assert!(quiet.is_empty(), "{quiet:?}");

    let mut loud = Vec::new();
    let lookup = atlas_lookup(&definitions, &image(&[1, 14, 9, 9]), &mut loud);
    // Both fall back to tile 0, and each warning sizes itself in pixels.
    assert_eq!(lookup[14], 0);
    assert_eq!(lookup[9], 0);
    assert_eq!(loud.len(), 2);
    assert!(loud[0].contains("9 on 2 pixels"), "{}", loud[0]);
    assert!(
        loud[0].contains("over 2 of the bitmap's 4 pixels"),
        "{}",
        loud[0]
    );
    assert!(loud[1].contains("index 14 texture 255"), "{}", loud[1]);
    assert!(loud[1].contains("its 1 pixels"), "{}", loud[1]);
}

#[test]
fn a_mismatched_bitmap_is_refused() {
    assert!(terrain_id_map(3, 2, &[1, 2, 3], &identity_lookup()).is_err());
    assert!(terrain_id_map(0, 0, &[], &identity_lookup()).is_err());
}

#[test]
fn the_largest_tile_index_survives_the_all_same_encoding() {
    let mut lookup = [0u8; 256];
    lookup[1] = (ATLAS_TILES - 1) as u8;
    let flagged = terrain_id_map(2, 2, &[1, 1, 1, 1], &lookup).unwrap();
    for texel in flagged.chunks_exact(4) {
        assert_eq!(u32::from(texel[2]), ATLAS_TILES - 1 + 100);
        let raw = f32::from(texel[2]);
        let all_same = (raw - 98.0).clamp(0.0, 1.0);
        assert_eq!(raw - all_same * 100.0, f32::from(texel[0]));
    }
    // An unflagged index must stay clear of the shader's 99 threshold.
    let mixed = terrain_id_map(2, 2, &[1, 0, 1, 1], &lookup).unwrap();
    assert!(mixed.chunks_exact(4).all(|texel| texel[2] < 99));
}

/// A 16-bit texture keeps the engine's normalisation on the device: every channel goes up
/// as the `f32` the device's `_UNORM` conversion makes (`c / 31`, `c / 63`), exactly; an
/// effect's threshold between that and the half float (`0.0322575`, between `1/31` and
/// `0.0322570801`) sees the exact value; the format is `Rgba32Float` in linear space, and
/// there is no sRGB one.
#[test]
fn sixteen_bit_textures_go_up_at_shader_precision() {
    for format in [dds::TextureFormat::B5G6R5, dds::TextureFormat::B5G5R5A1] {
        assert_eq!(
            format_of(format, ColorSpace::Linear).unwrap(),
            wgpu::TextureFormat::Rgba32Float
        );
        assert!(format_of(format, ColorSpace::Srgb).is_err());
    }
    let exact = |c: u16, max: u16| f32::from(c) / f32::from(max);
    // 5-6-5: red 1, green 1, blue 31. 5-5-5-1: red 1, green 0, blue 30, alpha 1.
    let cases = [
        (
            dds::TextureFormat::B5G6R5,
            0x083F_u16,
            [exact(1, 31), exact(1, 63), 1.0, 1.0],
        ),
        (
            dds::TextureFormat::B5G5R5A1,
            0x841E,
            [exact(1, 31), 0.0, exact(30, 31), 1.0],
        ),
    ];
    for (format, word, expected) in cases {
        let texels = sixteen_bit_texels(format, &word.to_le_bytes()).unwrap();
        let channels: Vec<f32> = texels
            .chunks_exact(4)
            .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
            .collect();
        assert_eq!(channels, expected, "{format:?}");
        // The threshold-sensitive effect: a half float of 1/31 falls below it.
        assert!(channels[0] >= 0.032_257_5, "{format:?}");
    }
    assert_eq!(
        sixteen_bit_texels(dds::TextureFormat::Rgba8, &[1, 2, 3, 4]),
        None
    );
}

#[test]
#[ignore = "requires an adapter; checks cube-map upload refusal"]
fn a_nonsquare_cube_is_refused_by_name_before_gpu_validation() {
    let _serial = super::super::political_gpu::tests::serial();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&crate::render::device_descriptor(&adapter)))
            .unwrap();
    for height in [4, 8] {
        let texture = dds::Texture {
            format: dds::TextureFormat::Rgba8,
            width: 8,
            height,
            faces: 6,
            mips: vec![
                vec![dds::MipLevel {
                    width: 8,
                    height,
                    data: vec![0; (8 * height * 4) as usize],
                }];
                6
            ],
        };
        let result = upload(
            &device,
            &queue,
            "reflection.dds",
            &texture,
            ColorSpace::Linear,
        );
        if height == 8 {
            assert!(result.is_ok());
        } else {
            let reason = result.err().unwrap().to_string();
            assert!(reason.contains("reflection.dds"), "{reason}");
            assert!(reason.contains("square cube-map faces"), "{reason}");
        }
    }
}

/// On the viewer's device a 16-bit texture samples as the game's: `textureLoad` returns
/// `c / 31` and `c / 63` exactly as `f32`s, arithmetic preserves their precision,
/// and a threshold between `1/31` and its half float (`0.0322575`) passes. A
/// device without `FLOAT32_FILTERABLE` also keeps the exact upload.
#[test]
#[ignore = "requires an adapter; samples 16-bit texels on the device"]
fn sixteen_bit_texels_sample_exactly_on_the_device() {
    let _serial = super::super::political_gpu::tests::serial();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    println!("TEXTURES adapter {:?}", adapter.get_info());
    let mut descriptor = crate::render::device_descriptor(&adapter);
    descriptor.required_features =
        descriptor.required_features - wgpu::Features::FLOAT32_FILTERABLE;
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    // Red, green and blue 1, then white.
    let texture = dds::Texture {
        format: dds::TextureFormat::B5G6R5,
        width: 2,
        height: 1,
        faces: 1,
        mips: vec![vec![dds::MipLevel {
            width: 2,
            height: 1,
            data: [0x0821_u16, 0xFFFF].map(u16::to_le_bytes).concat(),
        }]],
    };
    let bound = upload(&device, &queue, "565", &texture, ColorSpace::Linear).unwrap();
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("16-bit probe"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("16-bit probe out"),
        size: 48,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("16-bit probe read"),
        size: 48,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("16-bit probe"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&bound.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: out.as_entire_binding(),
            },
        ],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("16-bit probe"),
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var page: texture_2d<f32>;
             @group(0) @binding(2) var<storage, read_write> out: array<f32, 12>;
             @compute @workgroup_size(1)
             fn main() {
                 let texel = textureLoad(page, vec2<i32>(0, 0), 0);
                 let blend = (texel + textureLoad(page, vec2<i32>(1, 0), 0)) * 0.5;
                 for (var i = 0; i < 4; i++) {
                     out[i] = texel[i];
                     out[4 + i] = blend[i];
                 }
                 out[8] = select(0.0, 1.0, texel.r >= 0.0322575);
             }"
            .into(),
        ),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("16-bit probe"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("16-bit probe"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("16-bit probe"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out, 0, &read, 0, 48);
    queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let values: Vec<f32> = slice
        .get_mapped_range()
        .unwrap()
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
        .collect();
    let exact = |c: u16, max: u16| f32::from(c) / f32::from(max);
    assert_eq!(values[..4], [exact(1, 31), exact(1, 63), exact(1, 31), 1.0]);
    for (channel, texel) in [exact(1, 31), exact(1, 63), exact(1, 31), 1.0]
        .into_iter()
        .enumerate()
    {
        let blend = (texel + 1.0) / 2.0;
        assert!(
            (values[4 + channel] - blend).abs() <= 1e-6,
            "channel {channel}: {} against {blend}",
            values[4 + channel]
        );
    }
    assert_eq!(values[8], 1.0);
}
