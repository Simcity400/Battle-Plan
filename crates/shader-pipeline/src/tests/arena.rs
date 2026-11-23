use super::*;

fn program(source: &str) -> content_reader_gui::pdxshader::program::Program {
    content_reader_gui::pdxshader::program::read_with(b"arena.shader", &|path| {
        Some(if path == b"arena.shader" {
            source.as_bytes().to_vec()
        } else {
            Vec::new()
        })
    })
    .unwrap()
}

fn texel(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4]) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("arena test texel"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
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
        texture.as_image_copy(),
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn cached_groups_and_arena_offsets_follow_each_draw_across_frames() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let program = program(
        r#"VertexStruct I { float3 position : POSITION;
};
VertexStruct O { float4 position : SV_POSITION;
};
VertexShader = { MainCode V [[ O main(I v) { O o; o.position = float4(v.position, 1); return o; } ]] }
PixelShader = {
ConstantBuffer(2,0) { float4 Tint;
};
Samplers = { Colour = { Index = 0 MagFilter = Point MinFilter = Point MipFilter = Point } }
MainCode P [[ float4 main(O v) : SV_TARGET { return Colour_Texture.SampleLevel(Colour_Sampler, float2(0.5, 0.5), 0) * Tint; } ]]
}
Effect Test { VertexShader = V PixelShader = P }
"#,
    );
    let compiled = Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
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
    let red = texel(&device, &queue, [255, 0, 0, 255]);
    let green = texel(&device, &queue, [0, 255, 0, 255]);
    let rectangle = |x0: f32, y0: f32, x1: f32, y1: f32| {
        [(x0, y1), (x1, y1), (x1, y0), (x0, y1), (x1, y0), (x0, y0)].map(|(x, y)| [x, y, 0_f32])
    };
    let shapes = [
        rectangle(-1., -1., 0., 1.),
        rectangle(0., 0., 1., 1.),
        rectangle(0., -1., 1., 0.),
    ];
    let opaque = [1_f32; 4];
    let black = [0., 0., 0., 1_f32];
    let frame =
        |draws: [(&wgpu::TextureView, [f32; 4]); 3], packed: Option<bool>, repeated: bool| {
            let prepared: Vec<_> = draws
                .iter()
                .zip(if repeated {
                    [&shapes[0]; 3]
                } else {
                    [&shapes[0], &shapes[1], &shapes[2]]
                })
                .map(|((view, tint), shape)| {
                    let bindings = pipeline
                        .bindings(
                            &arena,
                            &BTreeMap::from([(binding(2, Stage::Pixel), bytemuck::bytes_of(tint))]),
                            &BTreeMap::from([(binding(0, Stage::Pixel), (*view).clone())]),
                        )
                        .unwrap();
                    let vertices = arena.vertices(bytemuck::cast_slice(shape)).unwrap();
                    let draw = pipeline
                        .prepare(
                            &arena,
                            &BTreeMap::from([(binding(2, Stage::Pixel), bytemuck::bytes_of(tint))]),
                            &BTreeMap::from([(binding(0, Stage::Pixel), (*view).clone())]),
                            &crate::vertices::Vertices {
                                declaration: crate::vertices::Declaration::Sprite,
                                bytes: bytemuck::cast_slice(shape),
                                stride: 12,
                                count: 6,
                            },
                        )
                        .unwrap();
                    (bindings, vertices, draw)
                })
                .collect();
            render_with(&device, &queue, &arena, |pass| {
                pass.set_pipeline(&pipeline.pipeline);
                let mut previous = None;
                let mut previous_draw = None;
                for (bindings, vertices, draw) in &prepared {
                    if let Some(incremental) = packed {
                        if incremental {
                            pipeline.draw_after(pass, draw, previous_draw);
                        } else {
                            pipeline.draw(pass, draw);
                        }
                        previous_draw = Some(draw);
                    } else {
                        bindings.bind_after(pass, previous);
                        pass.set_vertex_buffer(0, vertices.slice());
                        pass.draw(0..6, 0..1);
                        previous = Some(bindings);
                    }
                }
            })
        };
    let pixel = |pixels: &[u8], x: usize, y: usize| -> [u8; 4] {
        pixels[(y * 16 + x) * 4..][..4].try_into().unwrap()
    };
    let check = |pixels: &[u8], expected: [[u8; 4]; 3]| {
        for (x, y, index) in [(2, 8, 0), (12, 2, 1), (12, 13, 2)] {
            assert_eq!(
                pixel(pixels, x, y),
                expected[index],
                "draw {index} at {x},{y}"
            );
        }
    };
    let inputs = [(&red, opaque), (&green, opaque), (&green, black)];
    let first = frame(inputs, None, false);
    assert_eq!(frame(inputs, Some(false), false), first);
    assert_eq!(frame(inputs, Some(true), false), first);
    check(&first, [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 0, 255]]);
    let repeated = frame(inputs, Some(false), true);
    assert_eq!(frame(inputs, Some(true), true), repeated);
    assert_eq!(pixel(&repeated, 2, 8), [0, 0, 0, 255]);
    let inputs = [(&green, opaque), (&red, black), (&red, opaque)];
    let second = frame(inputs, None, false);
    assert_eq!(frame(inputs, Some(false), false), second);
    assert_eq!(frame(inputs, Some(true), false), second);
    check(
        &second,
        [[0, 255, 0, 255], [0, 0, 0, 255], [255, 0, 0, 255]],
    );
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn programs_past_the_dynamic_offset_limit_are_refused_by_name() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let (device, _) = device();
    let limit = device
        .limits()
        .max_dynamic_uniform_buffers_per_pipeline_layout;
    let blocks = limit + 1;
    let declarations: String = (0..blocks)
        .map(|slot| format!("ConstantBuffer({slot},0) {{ float4 Value{slot};\n}};\n"))
        .collect();
    let sum: Vec<_> = (0..blocks).map(|slot| format!("Value{slot}")).collect();
    let program = program(&format!(
        r#"VertexStruct I {{ float3 position : POSITION;
}};
VertexStruct O {{ float4 position : SV_POSITION;
}};
VertexShader = {{ MainCode V [[ O main(I v) {{ O o; o.position = float4(v.position, 1); return o; }} ]] }}
PixelShader = {{
{declarations}
MainCode P [[ float4 main(O v) : SV_TARGET {{ return {}; }} ]]
}}
Effect Test {{ VertexShader = V PixelShader = P }}
"#,
        sum.join(" + ")
    ));
    let compiled = Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    assert_eq!(compiled.pixel.buffers.len(), blocks as usize);
    let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
    let error = gpu::Pipeline::with_state(
        &device,
        &compiled,
        &program,
        &program.techniques[0],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        &state,
    )
    .err()
    .expect("refused");
    assert!(
        error.contains(&format!(
            "{blocks} constant buffers exceed this device's {limit} dynamically offset buffers"
        )),
        "{error}"
    );
}

#[test]
#[ignore = "requires a shader compiler and GPU"]
fn resolved_stream_pass_still_checks_changed_constant_ranges_and_missing_views() {
    crate::tests::initialize_runtime_tables();
    let _serial = GPU_TESTS.lock().unwrap_or_else(|error| error.into_inner());
    let program = program(
        r#"VertexStruct I { float3 position : POSITION;
};
VertexStruct O { float4 position : SV_POSITION;
};
VertexShader = { MainCode V [[ O main(I v) { O o; o.position = float4(v.position, 1); return o; } ]] }
PixelShader = {
ConstantBuffer(2,0) { float4 Tint;
};
Samplers = { Colour = { Index = 0 MagFilter = Point MinFilter = Point MipFilter = Point } }
MainCode P [[ float4 main(O v) : SV_TARGET { return Colour_Texture.SampleLevel(Colour_Sampler, float2(0.5, 0.5), 0) * Tint; } ]]
}
Effect Test { VertexShader = V PixelShader = P }
"#,
    );
    let compiled = Compiler::discover()
        .unwrap()
        .compile(&program, &program.techniques[0], &[])
        .unwrap();
    let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
    let pass = gpu::Pass::packed(&compiled, &state, wgpu::TextureFormat::Rgba8Unorm, 1);
    let effect = library::Effect::from_compiled(
        std::sync::Arc::new(program),
        vec![],
        vec![Ok(std::sync::Arc::new(compiled))],
    );
    let selection = effect.select("Test").unwrap().unwrap();
    let (device, queue) = device();
    let arena = gpu::Arena::new(&device);
    let mut stream = stream::Stream::default();
    stream.select_program(&selection, &pass.vertices);
    stream.bind_texture(
        0,
        Stage::Pixel,
        Some(texel(&device, &queue, [255; 4])),
        state.samplers.get(&binding(0, Stage::Pixel)),
    );
    stream.bind_constants(2, Stage::Pixel, Some(&[0; 16]));
    stream.prepare_pass(&arena, "valid", &pass).unwrap();
    stream.bind_partial_constants(
        2,
        Stage::Pixel,
        Some(&[0; 16]),
        std::slice::from_ref(&(0..4)),
    );
    let reason = stream
        .prepare_pass(&arena, "unknown bytes", &pass)
        .err()
        .unwrap();
    assert!(reason.contains("unavailable bytes"), "{reason}");
    stream.bind_constants(2, Stage::Pixel, Some(&[0; 16]));
    stream.forget_texture(0, Stage::Pixel, None);
    let reason = stream
        .prepare_pass(&arena, "missing texture", &pass)
        .err()
        .unwrap();
    assert!(reason.contains("has not been supplied"), "{reason}");
    assert_eq!(
        stream.cache.created, 1,
        "resource changes do not rebuild a pipeline"
    );
}
