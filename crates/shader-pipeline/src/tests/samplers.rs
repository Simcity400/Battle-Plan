use super::*;

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn point_float32_uses_actual_sampler_pairs_without_filterable_feature() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    assert!(
        !device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    );
    let program = content_reader_gui::pdxshader::program::read_with(b"point.shader", &|path| {
        Some(if path == b"point.shader" {
            br#"VertexStruct I {
float3 position : POSITION;
float2 uv : TEXCOORD0;
};
VertexStruct O {
float4 position : SV_POSITION;
};
VertexShader = { MainCode V [[ O main(I v) { O o; o.position=float4(v.position,1); return o; } ]] }
PixelShader = {
Samplers = {
 A = { Index = 0 MagFilter = Linear MinFilter = Linear MipFilter = Linear }
 B = { Index = 4 MagFilter = Point MinFilter = Point MipFilter = Point }
}
MainCode P [[ float4 main(O v) : SV_TARGET {
 return (A_Texture.SampleLevel(B_Sampler,float2(0.25,0.5),0) + B_Texture.SampleLevel(A_Sampler,float2(0.5,0.5),0)) * 0.5;
} ]]
}
Effect Test { VertexShader = V PixelShader = P }
"#.to_vec()
        } else { Vec::new() })
    }).unwrap();
    let compiled = Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    assert_eq!(
        compiled.pixel.sampling_pairs,
        vec![
            (binding(0, Stage::Pixel), binding(4, Stage::Pixel)),
            (binding(4, Stage::Pixel), binding(0, Stage::Pixel))
        ]
    );
    let mut state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
    let create = |format, width| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mixed sampler pair texture"),
            size: wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let floats = create(wgpu::TextureFormat::Rgba32Float, 2);
    queue.write_texture(
        floats.as_image_copy(),
        bytemuck::cast_slice(&[0.75_f32, 0.25, 0.5, 1., 0., 0., 0., 0.]),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(32),
            rows_per_image: None,
        },
        floats.size(),
    );
    let bytes = create(wgpu::TextureFormat::Rgba8Unorm, 1);
    queue.write_texture(
        bytes.as_image_copy(),
        &[0, 0, 0, 255],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: None,
        },
        bytes.size(),
    );
    let textures = BTreeMap::from([
        (
            binding(0, Stage::Pixel),
            floats.create_view(&Default::default()),
        ),
        (
            binding(4, Stage::Pixel),
            bytes.create_view(&Default::default()),
        ),
    ]);
    let pipeline = gpu::Pipeline::with_state(
        &device,
        &compiled,
        &program,
        &program.techniques[0],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        &state,
    )
    .unwrap();
    let vertices: Vec<[f32; 5]> = [0, 1, 2, 0, 2, 3]
        .iter()
        .map(|&i| [[-1., 1., 1., -1.][i], [1., 1., -1., -1.][i], 0., 0., 0.])
        .collect();
    let draw = pipeline
        .prepare(
            &arena,
            &BTreeMap::<u32, Vec<u8>>::new(),
            &textures,
            &vertices::Vertices {
                declaration: vertices::Declaration::Sprite,
                bytes: bytemuck::cast_slice(&vertices),
                stride: 20,
                count: 6,
            },
        )
        .unwrap();
    let pixels = render(&device, &queue, &arena, &pipeline, &draw);
    assert!(
        pixels
            .chunks_exact(4)
            .all(|pixel| pixel == [96, 32, 64, 255]),
        "pixel values: {:?}",
        pixels
            .chunks_exact(4)
            .collect::<std::collections::BTreeSet<_>>()
    );
    state
        .samplers
        .get_mut(&binding(4, Stage::Pixel))
        .unwrap()
        .min = content_reader_gui::pdxshader::program::Filter::Linear;
    let filtered = gpu::Pipeline::with_state(
        &device,
        &compiled,
        &program,
        &program.techniques[0],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        &state,
    )
    .unwrap();
    filtered
        .bindings(&arena, &BTreeMap::<u32, Vec<u8>>::new(), &textures)
        .unwrap();
    println!(
        "PASS point float32: 256 exact pixels without FLOAT32_FILTERABLE; cross-register sampler pairs; linear float32 bindings accepted"
    );
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn linear_float32_matches_bilinear_and_mip_addressing_without_feature() {
    crate::tests::initialize_runtime_tables();
    use content_reader_gui::pdxshader::{Address, program::Filter};
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let compiler = Compiler::discover().unwrap();
    let (device, queue) = device();
    assert!(
        !device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
    );
    let arena = gpu::Arena::new(&device);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("exact 16-bit channel values"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 3,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let texel = |l: u32, x: i32, y: i32| {
        [
            (x as f32 + l as f32 + 1.) / 31.,
            (y as f32 + 1.) / 63.,
            (x + y + 3) as f32 / 31.,
            1.,
        ]
    };
    for level in 0..3 {
        let n = 4 >> level;
        let data: Vec<f32> = (0..n * n)
            .flat_map(|p| texel(level, p % n, p / n))
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                mip_level: level,
                ..texture.as_image_copy()
            },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(n as u32 * 16),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: n as u32,
                height: n as u32,
                depth_or_array_layers: 1,
            },
        );
    }
    let textures = BTreeMap::from([(
        binding(0, Stage::Pixel),
        texture.create_view(&Default::default()),
    )]);
    let vertices: Vec<[f32; 5]> = [0, 1, 2, 0, 2, 3]
        .iter()
        .map(|&i| {
            [
                [-1., 1., 1., -1.][i],
                [1., 1., -1., -1.][i],
                0.,
                [-2., 2., 2., -2.][i],
                [-2., -2., 2., 2.][i],
            ]
        })
        .collect();
    let address = |mode, i: i32, n: i32| match mode {
        Address::Wrap => Some(i.rem_euclid(n)),
        Address::Mirror => {
            let p = i.rem_euclid(2 * n);
            Some(if p < n { p } else { 2 * n - p - 1 })
        }
        Address::Clamp => Some(i.clamp(0, n - 1)),
        Address::MirrorOnce => Some((if i < 0 { -i - 1 } else { i }).clamp(0, n - 1)),
        Address::Border => (0..n).contains(&i).then_some(i),
    };
    for (method, args, bias, offset) in [
        ("Sample", "", 0.375, [0, 0]),
        ("SampleBias", ", 0.375", 0., [1, -1]),
        ("SampleLevel", ", 0.375", 0., [0, 0]),
        (
            "SampleGrad",
            ", float2(0.25,0), float2(0,0.25)",
            0.375,
            [1, -1],
        ),
    ] {
        let texel_offset = if offset == [0, 0] { "" } else { ", int2(1,-1)" };
        let source = format!(
            r#"
VertexStruct I {{
float3 position : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct O {{
float4 position : SV_POSITION;
float2 uv : TEXCOORD0;
}};
VertexShader = {{ MainCode V [[ O main(I v) {{ O o; o.position=float4(v.position,1); o.uv=v.uv; return o; }} ]] }}
PixelShader = {{
Samplers = {{ Map = {{ Index = 0 MagFilter = Linear MinFilter = Linear MipFilter = Linear }} }}
MainCode P [[ float4 main(O v) : SV_TARGET {{ return Map_Texture.{method}(Map_Sampler, v.uv{args}{texel_offset}); }} ]]
}}
Effect Test {{ VertexShader = V PixelShader = P }}
"#
        );
        let program = content_reader_gui::pdxshader::program::read_with(b"float.shader", &|path| {
            Some(if path == b"float.shader" {
                source.as_bytes().to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let compiled = compiler
            .compile(&program, &program.techniques[0], &[])
            .unwrap();
        for mode in [
            Address::Wrap,
            Address::Mirror,
            Address::Clamp,
            Address::MirrorOnce,
            Address::Border,
        ] {
            if mode == Address::Border
                && !device
                    .features()
                    .contains(wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER)
            {
                println!("SKIP Border: adapter lacks ADDRESS_MODE_CLAMP_TO_BORDER");
                continue;
            }
            for mip in [Filter::Point, Filter::Linear] {
                let mut state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
                let sampler = state.samplers.get_mut(&binding(0, Stage::Pixel)).unwrap();
                sampler.u = mode;
                sampler.v = mode;
                sampler.mip = mip;
                sampler.lod_bias = bias;
                let pipeline = gpu::Pipeline::with_state(
                    &device,
                    &compiled,
                    &program,
                    &program.techniques[0],
                    wgpu::TextureFormat::Rgba32Float,
                    1,
                    &state,
                )
                .unwrap();
                let draw = pipeline
                    .prepare(
                        &arena,
                        &BTreeMap::<u32, Vec<u8>>::new(),
                        &textures,
                        &vertices::Vertices {
                            declaration: vertices::Declaration::Sprite,
                            bytes: bytemuck::cast_slice(&vertices),
                            stride: 20,
                            count: 6,
                        },
                    )
                    .unwrap();
                let bytes = render_format(
                    &device,
                    &queue,
                    &arena,
                    wgpu::TextureFormat::Rgba32Float,
                    |pass| pipeline.draw(pass, &draw),
                );
                for (p, pixel) in bytes.chunks_exact(16).enumerate() {
                    let u = -2. + 4. * ((p % 16) as f32 + 0.5) / 16.;
                    let v = -2. + 4. * ((p / 16) as f32 + 0.5) / 16.;
                    let bilinear = |l: u32| {
                        let n = 4 >> l;
                        let q = [
                            u * n as f32 + offset[0] as f32 - 0.5,
                            v * n as f32 + offset[1] as f32 - 0.5,
                        ];
                        let base = q.map(|a| a.floor() as i32);
                        let f = q.map(|a| a - a.floor());
                        let mut out = [0.; 4];
                        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            if let (Some(x), Some(y)) = (
                                address(mode, base[0] + dx, n),
                                address(mode, base[1] + dy, n),
                            ) {
                                let weight = (if dx == 0 { 1. - f[0] } else { f[0] })
                                    * (if dy == 0 { 1. - f[1] } else { f[1] });
                                for (c, value) in texel(l, x, y).iter().enumerate() {
                                    out[c] += value * weight;
                                }
                            }
                        }
                        out
                    };
                    let low = bilinear(0);
                    let high = bilinear(1);
                    for c in 0..4 {
                        let expected = if mip == Filter::Linear {
                            low[c] * 0.625 + high[c] * 0.375
                        } else {
                            low[c]
                        };
                        let actual =
                            f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap());
                        assert!(
                            (actual - expected).abs() < 1e-6,
                            "{method} {mode:?} {mip:?} pixel {p} channel {c}: {actual} != {expected}"
                        );
                    }
                }
            }
        }
    }
    println!(
        "PASS shader-filtered float32: bilinear/mips and address edges, full-precision readback"
    );
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn samples_native_addressing_and_mip_bias() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let compiler = Compiler::discover().unwrap();
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sampler comparison mips"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 5,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut mips = Vec::new();
    for mip in 0..5 {
        let size = 16 >> mip;
        let bytes: Vec<u8> = (0..size * size)
            .flat_map(|p| {
                [
                    (p % size * 16) as u8,
                    (p / size * 16) as u8,
                    mip as u8 * 50,
                    255,
                ]
            })
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        mips.push(bytes);
    }
    for (address, bias, method, expected_mip) in [
        ("wrap", 0, "Sample", 0),
        ("clamp", 0, "Sample", 0),
        ("mirror_once", 0, "Sample", 0),
        ("clamp", 2, "Sample", 2),
        ("mirror_once", 2, "Sample", 2),
        ("clamp", 2, "SampleLevel", 2),
    ] {
        let lod = if method == "SampleLevel" { ", 0" } else { "" };
        let source = format!(
            r#"
VertexStruct Input {{
float3 position : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct Output {{
float4 position : PDX_POSITION;
float2 uv : TEXCOORD0;
}};
VertexShader = {{ MainCode V [[ Output main(Input v) {{ Output o; o.position=float4(v.position,1); o.uv=v.uv; return o; }} ]] }}
PixelShader = {{
Samplers = {{ Map = {{ Index = 0 MagFilter = Point MinFilter = Point MipFilter = None AddressU = {address} AddressV = Clamp MipMapLodBias = {bias} }} }}
MainCode P [[ float4 main(Output v) : PDX_COLOR {{ return Map_Texture.{method}(Map_Sampler, v.uv{lod}); }} ]]
}}
Effect Test {{ VertexShader = V PixelShader = P }}
"#
        );
        let program = content_reader_gui::pdxshader::program::read_with(b"sampler.shader", &|path| {
            Some(if path == b"sampler.shader" {
                source.as_bytes().to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
        let compiled = compiler
            .compile(&program, &program.techniques[0], &[])
            .unwrap();
        let mut state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
        state
            .samplers
            .get_mut(&binding(0, Stage::Pixel))
            .unwrap()
            .cube = true;
        let pipeline = gpu::Pipeline::with_state(
            &device,
            &compiled,
            &program,
            &program.techniques[0],
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            &state,
        )
        .unwrap();
        let vertices: Vec<[f32; 5]> = [0, 1, 2, 0, 2, 3]
            .iter()
            .map(|&i| {
                [
                    [-1., 1., 1., -1.][i],
                    [1., 1., -1., -1.][i],
                    0.,
                    [-0.5, 0.5, 0.5, -0.5][i],
                    [0., 0., 1., 1.][i],
                ]
            })
            .collect();
        let draw = pipeline
            .prepare(
                &arena,
                &BTreeMap::<u32, Vec<u8>>::new(),
                &BTreeMap::from([(
                    binding(0, Stage::Pixel),
                    texture.create_view(&Default::default()),
                )]),
                &vertices::Vertices {
                    declaration: vertices::Declaration::Sprite,
                    bytes: bytemuck::cast_slice(&vertices),
                    stride: 20,
                    count: vertices.len(),
                },
            )
            .unwrap();
        let pixels = render(&device, &queue, &arena, &pipeline, &draw);
        let size = 16 >> expected_mip;
        for y in 0..16 {
            for x in 0..16 {
                let u = (x as f32 + 0.5) / 16. - 0.5;
                let addressed = match address {
                    "wrap" => u.rem_euclid(1.),
                    "mirror_once" => u.abs(),
                    _ => u.max(0.),
                };
                let tx = ((addressed * size as f32) as usize).min(size - 1);
                let ty = (((y as f32 + 0.5) / 16. * size as f32) as usize).min(size - 1);
                let offset = (ty * size + tx) * 4;
                assert_eq!(
                    &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                    &mips[expected_mip][offset..offset + 4],
                    "{address} bias={bias} {method} pixel {x},{y}"
                );
            }
        }
        println!("PASS sampler {address} bias={bias} {method}: 256 exact pixels");
        let state_sampler = program.sampler(0).unwrap().clone();
        let effect = library::Effect::from_compiled(
            std::sync::Arc::new(program),
            Vec::new(),
            vec![Ok(std::sync::Arc::new(compiled))],
        );
        let mut stream = stream::Stream::default();
        stream
            .bind_effect(
                effect.select("Test").unwrap(),
                vertices::Declaration::Sprite,
            )
            .unwrap();
        stream.bind_texture(
            0,
            Stage::Pixel,
            Some(texture.create_view(&Default::default())),
            Some(&state_sampler),
        );
        stream.bind_texture(0, Stage::Pixel, None, None);
        let submission = stream
            .prepare(
                &arena,
                "explicit null SRV",
                bytemuck::cast_slice(&vertices),
                20,
                vertices.len(),
                wgpu::PrimitiveTopology::TriangleList,
                wgpu::TextureFormat::Rgba8Unorm,
                1,
            )
            .unwrap();
        assert_eq!(
            render(
                &device,
                &queue,
                &arena,
                &submission.pipeline,
                &submission.draw
            ),
            vec![0; 16 * 16 * 4],
            "null SRV, retained sampler"
        );
    }
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn mirror_once_gathers_and_offsets_match_d3d() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let compiler = Compiler::discover().unwrap();
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let texture = |levels: u32| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mirror-once reference"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for level in 0..levels {
            let size = 16 >> level;
            let bytes: Vec<u8> = (0..size * size)
                .flat_map(|p| {
                    [
                        (p % size * 16) as u8,
                        (p / size * 16) as u8,
                        level as u8 * 50,
                        255,
                    ]
                })
                .collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        texture
    };
    let (u0, u1, v0, v1) = (-1.078125_f32, 0.921875, 0.015625, 1.015625);
    let vertices: Vec<[f32; 5]> = [0, 1, 2, 0, 2, 3]
        .iter()
        .map(|&i| {
            [
                [-1., 1., 1., -1.][i],
                [1., 1., -1., -1.][i],
                0.,
                [u0, u1, u1, u0][i],
                [v0, v0, v1, v1][i],
            ]
        })
        .collect();
    let mirror = |i: i32, n: i32| (if i < 0 { -i - 1 } else { i }).clamp(0, n - 1);
    let texel = move |level: u32, x: i32, y: i32| {
        let n = 16 >> level;
        [
            16 * mirror(x, n),
            16 * y.clamp(0, n - 1),
            50 * level as i32,
            255,
        ]
    };
    let at = move |x: usize, y: usize, level: u32| {
        let n = (16 >> level) as f32;
        let u = u0 + (u1 - u0) * (x as f32 + 0.5) / 16.;
        let v = v0 + (v1 - v0) * (y as f32 + 0.5) / 16.;
        (u * n, v * n)
    };
    type Reference = Box<dyn Fn(usize, usize) -> [i32; 4]>;
    let cases: [(&str, &str, &str, u32, Reference); 5] = [
        (
            "gather",
            "MagFilter = Point MinFilter = Point MipFilter = None",
            "Map_Texture.GatherRed(Map_Sampler, v.uv)",
            5,
            Box::new(move |x, y| {
                let (u, _) = at(x, y, 0);
                let x0 = (u - 0.5).floor() as i32;
                let r = |x| 16 * mirror(x, 16);
                [r(x0), r(x0 + 1), r(x0 + 1), r(x0)]
            }),
        ),
        (
            "single-mip offset",
            "MagFilter = Point MinFilter = Point MipFilter = Linear",
            "Map_Texture.Sample(Map_Sampler, v.uv, int2(1, 0))",
            1,
            Box::new(move |x, y| {
                let (u, v) = at(x, y, 0);
                texel(0, u.floor() as i32 + 1, v.floor() as i32)
            }),
        ),
        (
            "point-mip explicit LOD offset",
            "MagFilter = Point MinFilter = Point MipFilter = Point",
            "Map_Texture.SampleLevel(Map_Sampler, v.uv, 1.3, int2(1, 0))",
            5,
            Box::new(move |x, y| {
                let (u, v) = at(x, y, 1);
                texel(1, u.floor() as i32 + 1, v.floor() as i32)
            }),
        ),
        (
            "single-mip minification with point mag",
            "MagFilter = Point MinFilter = Linear MipFilter = Linear MipMapLodBias = 0.5",
            "Map_Texture.Sample(Map_Sampler, v.uv, int2(1, 0))",
            1,
            Box::new(move |x, y| {
                let (u, v) = at(x, y, 0);
                let (x0, y0) = ((u + 0.5).floor() as i32, (v - 0.5).floor() as i32);
                let m = |i| mirror(i, 16);
                let c = |i: i32| i.clamp(0, 15);
                [
                    4 * (3 * m(x0) + m(x0 + 1)),
                    4 * (3 * c(y0) + c(y0 + 1)),
                    0,
                    255,
                ]
            }),
        ),
        (
            "trilinear offset with bias",
            "MagFilter = Point MinFilter = Point MipFilter = Linear MipMapLodBias = 0.5",
            "Map_Texture.Sample(Map_Sampler, v.uv, int2(1, 0))",
            5,
            Box::new(move |x, y| {
                let (u1, v1) = at(x, y, 1);
                let (u2, v2) = at(x, y, 2);
                let a = texel(1, u1.floor() as i32 + 1, v1.floor() as i32);
                let b = texel(2, u2.floor() as i32 + 1, v2.floor() as i32);
                std::array::from_fn(|c| (a[c] + b[c]) / 2)
            }),
        ),
    ];
    for (label, filters, expression, levels, reference) in cases {
        let source = format!(
            r#"
VertexStruct Input {{
float3 position : POSITION;
float2 uv : TEXCOORD0;
}};
VertexStruct Output {{
float4 position : PDX_POSITION;
float2 uv : TEXCOORD0;
}};
VertexShader = {{ MainCode V [[ Output main(Input v) {{ Output o; o.position=float4(v.position,1); o.uv=v.uv; return o; }} ]] }}
PixelShader = {{
Samplers = {{ Map = {{ Index = 0 {filters} AddressU = Mirror_Once AddressV = Clamp }} }}
MainCode P [[ float4 main(Output v) : PDX_COLOR {{ return {expression}; }} ]]
}}
Effect Test {{ VertexShader = V PixelShader = P }}
"#
        );
        let program = content_reader_gui::pdxshader::program::read_with(b"mirror.shader", &|path| {
            Some(if path == b"mirror.shader" {
                source.as_bytes().to_vec()
            } else {
                b"#define PDX_POSITION SV_POSITION\n#define PDX_COLOR SV_TARGET\n".to_vec()
            })
        })
        .unwrap();
        let compiled = compiler
            .compile(&program, &program.techniques[0], &[])
            .unwrap();
        let pipeline = gpu::Pipeline::new(
            &device,
            &compiled,
            &program,
            &program.techniques[0],
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        )
        .unwrap();
        let draw = pipeline
            .prepare(
                &arena,
                &BTreeMap::<u32, Vec<u8>>::new(),
                &BTreeMap::from([(
                    binding(0, Stage::Pixel),
                    texture(levels).create_view(&Default::default()),
                )]),
                &vertices::Vertices {
                    declaration: vertices::Declaration::Sprite,
                    bytes: bytemuck::cast_slice(&vertices),
                    stride: 20,
                    count: vertices.len(),
                },
            )
            .unwrap();
        let pixels = render(&device, &queue, &arena, &pipeline, &draw);
        for y in 0..16 {
            for x in 0..16 {
                let expected = reference(x, y).map(|c| c as u8);
                assert_eq!(
                    &pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4],
                    &expected,
                    "{label} pixel {x},{y}"
                );
            }
        }
        println!("PASS mirror-once {label}: 256 exact pixels");
    }
}
