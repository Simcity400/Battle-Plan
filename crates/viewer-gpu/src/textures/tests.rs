use crate::gpu_contract::device_descriptor;
#[cfg(test)]
use crate::test_support::serial;
use content_reader_map::map::{CountryColors, ProvinceDefinition};

use super::*;
use std::collections::BTreeMap;

const POW2: Decode = Decode {
    force_pow2_textures: true,
};

#[test]
fn native_missing_and_oversized_map_textures_sample_zero() {
    for content in [
        Vec::new(),
        vec![("map/large.dds", vec![0; 16 * 1024 * 1024 + 1])],
    ] {
        let read = files(content);
        let mut warnings = Vec::new();
        let texture = load_map_texture(
            "map/large.dds",
            &read,
            Decode::default(),
            ColorSpace::Linear,
            &mut warnings,
        )
        .unwrap();
        assert!(texture.unbound);
        let texture = texture.texture;
        assert_eq!(texture.base().data, [0; 4]);
        assert_eq!((texture.width, texture.height, texture.faces), (1, 1, 1));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("map/large.dds") && warnings[0].contains("unbound"));
    }
}

#[test]
fn unknown_map_texture_outcomes_are_not_replaced_with_zero() {
    let mut bytes = dx10_file(4, 4, 1, 83);
    bytes.extend([0; 16]);
    let read = files(vec![("map/unknown.dds", bytes)]);
    let mut warnings = Vec::new();
    let error = load_map_texture(
        "map/unknown.dds",
        &read,
        Decode::default(),
        ColorSpace::Linear,
        &mut warnings,
    )
    .unwrap_err();
    assert!(error.to_string().contains("map/unknown.dds"));
    assert!(warnings.is_empty());
    let denied = |_: &[u8]| Err::<Vec<u8>, _>(io::Error::from(io::ErrorKind::PermissionDenied));
    assert_eq!(
        load_map_texture(
            "map/denied.dds",
            &denied,
            Decode::default(),
            ColorSpace::Linear,
            &mut warnings
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(warnings.is_empty());
}

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
        file.pop();
        let read = files(vec![("gfx/test.dds", file)]);
        let why = load_file("gfx/test.dds", &read, Decode::default())
            .unwrap_err()
            .to_string();
        assert!(why.contains("gfx/test.dds") && why.contains("BC7") && why.contains("level 3"));
    }
}

#[test]
fn unsupported_dx10_formats_are_refused_by_name() {
    for dxgi in [83, 95] {
        for (side, texels) in [(4, 64), (1, 4)] {
            let mut file = dx10_file(side, side, 1, dxgi);
            file.extend(std::iter::repeat_n(1, texels));
            let read = files(vec![("gfx/other.dds", file)]);
            let why = load_file("gfx/other.dds", &read, Decode::default())
                .unwrap_err()
                .to_string();
            assert!(
                why.contains("gfx/other.dds") && why.contains(&format!("unsupported DXGI format {dxgi}")),
                "{why}"
            );
        }
    }
}

fn files(files: Vec<(&'static str, Vec<u8>)>) -> impl Fn(&[u8]) -> io::Result<Vec<u8>> {
    move |path| {
        files
            .iter()
            .find(|(name, _)| name.as_bytes() == path)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

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

#[test]
fn the_intel_texel_is_the_complement_of_the_block_mean() {
    let map = two_block_map([ProvinceKind::Land, ProvinceKind::Land]);
    let visibility: BTreeMap<u32, u8> = [(1, 0), (2, 250)].into_iter().collect();
    let built = IntelCells::new(&map).unwrap().intel_map(Some(&visibility));
    assert!(built.fog);
    assert_eq!(built.texels, vec![5, 255]);
}

#[test]
fn an_unrecorded_province_is_not_a_recorded_zero() {
    let map = two_block_map([ProvinceKind::Land, ProvinceKind::Land]);
    let partial: BTreeMap<u32, u8> = [(2, 0)].into_iter().collect();
    assert_eq!(
        IntelCells::new(&map)
            .unwrap()
            .intel_map(Some(&partial))
            .texels,
        vec![255, 0]
    );
}

#[test]
fn sea_and_lake_provinces_never_take_fog() {
    let map = two_block_map([ProvinceKind::Sea, ProvinceKind::Lake]);
    let visibility: BTreeMap<u32, u8> = [(1, 0), (2, 0)].into_iter().collect();
    assert_eq!(
        IntelCells::new(&map)
            .unwrap()
            .intel_map(Some(&visibility))
            .texels,
        vec![0, 0]
    );
}

#[test]
fn no_recorded_visibility_is_the_engines_fog_off_branch() {
    let map = two_block_map([ProvinceKind::Land, ProvinceKind::Land]);
    let built = clear_intel_map(&map).unwrap();
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
    let indices = [1u8, 2, 3, 4, 5, 6];
    let map = terrain_id_map(3, 2, &indices, &identity_lookup()).unwrap();
    let texel = |x: usize, z: usize| &map[(z * 3 + x) * 4..][..4];
    assert_eq!(texel(0, 0), [1, 5, 4, 2]);
    assert_eq!(texel(2, 0), [3, 6, 6, 3]);
    assert_eq!(texel(0, 1), [1, 2, 1, 2]);
    assert_eq!(texel(2, 1), [3, 3, 103, 3]);
}

#[test]
fn a_uniform_neighbourhood_sets_the_all_same_flag() {
    let indices = [7u8; 9];
    let map = terrain_id_map(3, 3, &indices, &identity_lookup()).unwrap();
    for texel in map.chunks_exact(4) {
        assert_eq!(texel, [7, 7, 107, 7]);
    }
    let raw = 107.0f32;
    let all_same = (raw - 98.0).clamp(0.0, 1.0);
    assert_eq!(all_same, 1.0);
    assert_eq!(raw - all_same * 100.0, 7.0);
}

#[test]
fn declared_atlas_indices_keep_native_bytes_and_only_unmapped_indices_warn() {
    use content_reader_key_names::terrain::TerrainType;

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
    definitions.palette.insert(2, kind(529));
    let image = |indices: &[u8]| IndexedImage {
        width: indices.len() as u32,
        height: 1,
        indices: indices.into(),
        palette: [[0; 4]; 256],
    };

    let mut quiet = Vec::new();
    let lookup = atlas_lookup(&definitions, &image(&[1, 1, 1]), &mut quiet);
    assert_eq!(lookup[1], 4);
    assert!(quiet.is_empty(), "{quiet:?}");

    let mut loud = Vec::new();
    let lookup = atlas_lookup(&definitions, &image(&[1, 14, 9, 9, 2]), &mut loud);
    assert_eq!(lookup[14], 255);
    assert_eq!(lookup[2], 17);
    assert_eq!(lookup[9], 0);
    assert_eq!(loud.len(), 1);
    assert!(loud[0].contains("9 on 2 pixels"), "{}", loud[0]);
    assert!(
        loud[0].contains("over 2 of the bitmap's 5 pixels"),
        "{}",
        loud[0]
    );
}

#[test]
fn a_mismatched_bitmap_is_refused() {
    assert!(terrain_id_map(3, 2, &[1, 2, 3], &identity_lookup()).is_err());
    assert!(terrain_id_map(0, 0, &[], &identity_lookup()).is_err());
}

#[test]
fn all_texture_bytes_keep_native_uniform_encoding_and_mixed_neighbours() {
    let mut lookup = [0u8; 256];
    for value in 0..=255u8 {
        lookup[1] = value;
        let flagged = terrain_id_map(2, 2, &[1, 1, 1, 1], &lookup).unwrap();
        for texel in flagged.chunks_exact(4) {
            let packed = ((u16::from(value) + 100) % 256) as u8;
            assert_eq!(texel, [value, value, packed, value]);
        }
        if value != 0 {
            let mixed = terrain_id_map(2, 2, &[1, 0, 1, 1], &lookup).unwrap();
            assert_eq!(&mixed[..4], [value, value, value, 0]);
        }
    }
}

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
    let _serial = serial();
    let instance = shader_pipeline::gpu::instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&device_descriptor(&adapter))).unwrap();
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
                    data: (vec![0; (8 * height * 4) as usize]).into(),
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

#[test]
#[ignore = "requires an adapter; samples 16-bit texels on the device"]
fn sixteen_bit_texels_sample_exactly_on_the_device() {
    let _serial = serial();
    let instance = shader_pipeline::gpu::instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    println!("TEXTURES adapter {:?}", adapter.get_info());
    let mut descriptor = device_descriptor(&adapter);
    descriptor.required_features =
        descriptor.required_features - wgpu::Features::FLOAT32_FILTERABLE;
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    let texture = dds::Texture {
        format: dds::TextureFormat::B5G6R5,
        width: 2,
        height: 1,
        faces: 1,
        mips: vec![vec![dds::MipLevel {
            width: 2,
            height: 1,
            data: ([0x0821_u16, 0xFFFF].map(u16::to_le_bytes).concat()).into(),
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
use content_reader_image::bmp;

#[test]
fn upload_refusals_are_known_without_a_device() {
    let texture = |format, width, height, faces| dds::Texture {
        format,
        width,
        height,
        faces,
        mips: vec![
            vec![dds::MipLevel {
                width,
                height,
                data: (Vec::new()).into(),
            }];
            faces as usize
        ],
    };
    let packed = texture(dds::TextureFormat::B5G6R5, 4, 4, 1);
    assert!(check_upload("packed", &packed, ColorSpace::Linear).is_ok());
    assert!(check_upload("packed", &packed, ColorSpace::Srgb).is_err());
    let cube = texture(dds::TextureFormat::Rgba8, 8, 4, 6);
    assert!(check_upload("cube", &cube, ColorSpace::Srgb).is_err());
    let faces = texture(dds::TextureFormat::Rgba8, 4, 4, 3);
    assert!(check_upload("faces", &faces, ColorSpace::Srgb).is_err());
    let plain = texture(dds::TextureFormat::Bc1, 8, 8, 1);
    assert!(check_upload("plain", &plain, ColorSpace::Srgb).is_ok());
}

#[test]
fn deferred_textures_drop_cpu_copies_and_reload_through_mod_stack() {
    let root = std::env::temp_dir().join(format!(
        "bp_deferred_tex_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let base = root.join("base");
    let mod_dir = root.join("mod");
    std::fs::create_dir_all(base.join("map/terrain")).unwrap();
    std::fs::create_dir_all(mod_dir.join("map/terrain")).unwrap();
    let base_tga = viewer_runtime::test_support::test_tga_rgba8(4, 4);
    let mut mod_tga = viewer_runtime::test_support::test_tga_rgba8(4, 4);
    mod_tga[17] |= 0x20;
    mod_tga[18..22].copy_from_slice(&[9, 8, 7, 255]);
    std::fs::write(base.join("map/terrain/test.tga"), &base_tga).unwrap();
    std::fs::write(mod_dir.join("map/terrain/test.tga"), &mod_tga).unwrap();

    let layers = content_reader_layers::ResolvedLayers::from_folders(&[
        (base.as_path(), &[][..]),
        (mod_dir.as_path(), &[][..]),
    ])
    .unwrap();
    let _generation = content_reader_layers::content::Generation::enter();
    let loading = layers.loading();
    let deferred = DeferredTexture::load_artwork(
        &layers,
        "map/terrain/test.tga".into(),
        Decode::default(),
        ColorSpace::Srgb,
    )
    .unwrap();
    assert!(!deferred.is_held());
    assert!(
        !layers
            .content_counts()
            .products
            .iter()
            .any(|product| product.parser == "native texture decoding")
    );
    let reloaded = deferred.decode().unwrap();
    assert_eq!((reloaded.width, reloaded.height), (4, 4));
    assert_eq!(&reloaded.base().data[..4], &[7, 8, 9, 255]);
    assert!(
        !layers
            .content_counts()
            .products
            .iter()
            .any(|product| product.parser == "native texture decoding")
    );

    drop(loading);
    std::fs::remove_file(mod_dir.join("map/terrain/test.tga")).unwrap();
    std::fs::remove_file(base.join("map/terrain/test.tga")).unwrap();
    let missing = deferred.decode().unwrap_err().to_string();
    assert!(missing.contains("map/terrain/test.tga"), "{missing}");

    let _ = std::fs::remove_dir_all(&root);
}
