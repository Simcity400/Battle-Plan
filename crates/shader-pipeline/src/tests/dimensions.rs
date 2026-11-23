use super::*;

#[test]
fn cube_sampling_of_2d_srv_is_zero_without_changing_later_2d_draws() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let program = content_reader_gui::pdxshader::program::read_with(b"dimension.shader", &|path| {
        Some(if path == b"dimension.shader" {
            b"PixelShader={Samplers={Image={Index=7 MagFilter=Point MinFilter=Point MipFilter=Point}}} Effect Cube{} Effect Plane{}".to_vec()
        } else { Vec::new() })
    }).unwrap();
    let shader = |wgsl: String| Shader {
        wgsl,
        independent_draws: false,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    };
    let vertex = shader("@vertex fn main(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{var p=array<vec2<f32>,3>(vec2(-1.,-1.),vec2(3.,-1.),vec2(-1.,3.));return vec4(p[i],0.,1.);}".into());
    let image = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("synthetic six-wide 2D strip"),
        size: wgpu::Extent3d {
            width: 1536,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        image.as_image_copy(),
        &[64, 128, 192, 255].repeat(1536 * 256),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(1536 * 4),
            rows_per_image: None,
        },
        image.size(),
    );
    let view = image.create_view(&Default::default());
    let cube = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("synthetic valid cube"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        cube.as_image_copy(),
        &[64, 128, 192, 255].repeat(6),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        cube.size(),
    );
    let cube_view = cube.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    let layer = gpu::TextureBinding {
        view: cube.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: 3,
            array_layer_count: Some(1),
            ..Default::default()
        }),
        format: wgpu::TextureFormat::Rgba8Unorm,
        array: Some((
            cube.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            3,
        )),
    };
    for slot in [7, 9] {
        let key = binding(slot, Stage::Pixel);
        for (kind, declaration, coordinates, supplied, expected) in [
            (
                TextureKind::Cube,
                "texture_cube<f32>",
                "vec3(1.,0.,0.)",
                gpu::TextureBinding::from(view.clone()),
                [0, 0, 0, 0],
            ),
            (
                TextureKind::D2,
                "texture_2d<f32>",
                "vec2(0.5,0.5)",
                gpu::TextureBinding::from(view.clone()),
                [64, 128, 192, 255],
            ),
            (
                TextureKind::Cube,
                "texture_cube<f32>",
                "vec3(1.,0.,0.)",
                gpu::TextureBinding::from(cube_view.clone()),
                [64, 128, 192, 255],
            ),
            (
                TextureKind::Cube,
                "texture_cube<f32>",
                "vec3(1.,0.,0.)",
                layer.clone(),
                [0, 0, 0, 0],
            ),
            (
                TextureKind::D2,
                "texture_2d<f32>",
                "vec2(0.5,0.5)",
                layer.clone(),
                [64, 128, 192, 255],
            ),
        ] {
            let mut pixel = shader(format!(
                "@group(1) @binding({key}) var image:{declaration}; @group(2) @binding({key}) var samp:sampler; @fragment fn main()->@location(0) vec4<f32>{{return textureSampleLevel(image,samp,{coordinates},0.);}}"
            ));
            pixel.textures = vec![(key, kind)];
            pixel.samplers = vec![key];
            pixel.sampling_pairs = vec![(key, key)];
            let compiled = Compiled {
                vertex: vertex.clone(),
                pixel,
            };
            let technique = &program.techniques[usize::from(kind == TextureKind::D2)];
            let mut state = gpu::State::authored(&program, technique).unwrap();
            let sampler = state.samplers.values().next().unwrap().clone();
            state.samplers = BTreeMap::from([(key, sampler)]);
            let mut pass = gpu::Pass::packed(&compiled, &state, wgpu::TextureFormat::Rgba8Unorm, 1);
            pass.vertices.clear();
            let pipeline =
                gpu::Pipeline::for_pass(&device, &compiled, &program, technique, &state, &pass)
                    .unwrap();
            let bindings = pipeline
                .bind_with(
                    &arena,
                    |_| None,
                    |slot| (slot == key).then(|| pipeline.sampled_texture(slot, &supplied)),
                )
                .unwrap();
            let pixels = render_with(&device, &queue, &arena, |pass| {
                bindings.bind(pass);
                pass.draw(0..3, 0..1);
            });
            assert!(
                pixels.chunks_exact(4).all(|pixel| pixel == expected),
                "slot {slot} {kind:?}"
            );
        }
    }
}

#[test]
fn texture_binding_errors_name_the_effect_and_technique() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let (device, _queue) = device();
    let arena = gpu::Arena::new(&device);
    let program = content_reader_gui::pdxshader::program::read_with(b"named.shader", &|path| {
        Some(if path == b"named.shader" {
            b"Effect Water{}".to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap();
    let shader = |wgsl: &str| Shader {
        wgsl: wgsl.into(),
        independent_draws: false,
        buffers: Vec::new(),
        textures: Vec::new(),
        samplers: Vec::new(),
        sampling_pairs: Vec::new(),
        inputs: Vec::new(),
        clock: Default::default(),
    };
    let mut pixel = shader(
        "@group(1) @binding(15) var image:texture_3d<f32>; @fragment fn main()->@location(0) vec4<f32>{return textureLoad(image,vec3<i32>(0),0);}",
    );
    pixel.textures = vec![(15, TextureKind::D3)];
    let compiled = Compiled {
        vertex: shader(
            "@vertex fn main()->@builtin(position) vec4<f32>{return vec4(0.,0.,0.,1.);}",
        ),
        pixel,
    };
    let pipeline = gpu::Pipeline::new(
        &device,
        &compiled,
        &program,
        &program.techniques[0],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
    )
    .unwrap();
    let image = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wrong dimension"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let reason = pipeline
        .bindings(
            &arena,
            &BTreeMap::<u32, Vec<u8>>::new(),
            &BTreeMap::from([(15, image.create_view(&Default::default()))]),
        )
        .err()
        .unwrap();
    assert!(
        reason.contains("named.shader") && reason.contains("Water") && reason.contains("15"),
        "{reason}"
    );
}
