//! Offline native bloom ABI fixture. This is not a game-frame equivalence claim.
use std::{collections::BTreeMap, fs, io, path::Path, sync::Arc};

use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{
    ResolvedLayers,
    pdxshader::program::{self, Stage},
};
use serde_json::{Value, json};
use shader_pipeline::{binding, cache::Cache, gpu, library::Effect};

const SIZE: [u32; 2] = [256, 128];
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

fn fail(reason: impl Into<String>) -> io::Error {
    io::Error::other(reason.into())
}

pub(super) fn run(
    layers: &ResolvedLayers,
    defines: &[String],
    cache: &Cache,
    output: &Path,
) -> io::Result<Value> {
    let source = program::read(layers, b"gfx/FX/bloom.shader").map_err(fail)?;
    let index = source
        .techniques
        .iter()
        .position(|t| t.name == "bloom")
        .ok_or_else(|| fail("bloom technique missing"))?;
    let compiled = cache
        .get(&source, &source.techniques[index], defines)
        .map_err(fail)?;
    let mut state = gpu::State::authored(&source, &source.techniques[index]).map_err(fail)?;
    // 142345450's physical-index table; 140FF1D40 binds only PS table[0].
    state
        .samplers
        .retain(|key, _| *key == binding(0, Stage::Pixel));
    let mut pass = gpu::Pass::packed(&compiled, &state, FORMAT, 1);
    // 140FF1410 declaration: one POSITION0, type3/count4. 1424111F0 lowers
    // this to R16G16B16A16_SINT; shader int2 uses the first two lanes.
    let mut attributes = Vec::new();
    for input in &compiled.vertex.inputs {
        if !matches!(
            input.semantic.to_ascii_uppercase().as_str(),
            "POSITION" | "POSITION0"
        ) {
            return Err(fail(format!(
                "native fullscreen declaration has no {}",
                input.semantic
            )));
        }
        attributes.push(wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Sint16x4,
            offset: 0,
            shader_location: input.location,
        });
    }
    pass.vertices = vec![gpu::VertexLayout {
        stride: 8,
        step: wgpu::VertexStepMode::Vertex,
        attributes,
    }];
    // 140B48E50 restores CGraphics+c0; 142224600/1423F7180 create that state.
    pass.primitive.front_face = wgpu::FrontFace::Cw;
    pass.primitive.cull_mode = Some(wgpu::Face::Back);
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .map_err(|e| fail(e.to_string()))?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features() & wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER,
        ..Default::default()
    }))
    .map_err(|e| fail(e.to_string()))?;
    let arena = shader_pipeline::gpu::Arena::new(&device);
    let mut programs =
        vec![Err("technique not selected by this fixture".into()); source.techniques.len()];
    programs[index] = Ok(compiled);
    let effect = Effect::from_compiled(Arc::new(source), defines.to_vec(), programs);
    let selection = effect
        .select("bloom")
        .map_err(fail)?
        .ok_or_else(|| fail("bloom technique missing"))?;
    let pipeline = gpu::Cache::default()
        .get_pass(&device, &selection, &state, &pass)
        .map_err(fail)?;
    // 140FF2E00 uploads first16 bytes from14296E6B0 and the final8 as immediates.
    let vertices: [[i16; 4]; 3] = [[3, -1, 0, 0], [-1, -1, 1, 0], [-1, 3, 2, 0]];
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("native post triangle"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bloom fixture source"),
        size: extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // Asymmetric high-frequency fixture detects Y flips, clamp boundaries and offsets.
    // Native bloom's input is RGBA16F too. Exact half-float palette spans 0..10,
    // so this also exercises HDR values which a UNORM source cannot represent.
    const VALUES: [u16; 8] = [0, 0x2000, 0x3000, 0x3800, 0x3c00, 0x4000, 0x4400, 0x4900];
    let pixels: Vec<u16> = (0..SIZE[1])
        .flat_map(|y| {
            (0..SIZE[0]).flat_map(move |x| {
                [
                    VALUES[((x * 7 + y * 11) % 8) as usize],
                    VALUES[((x ^ y) % 8) as usize],
                    if x < 17 && y > 89 { 0x4900 } else { 0 },
                    0x3c00,
                ]
            })
        })
        .collect();
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&pixels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE[0] * 8),
            rows_per_image: Some(SIZE[1]),
        },
        extent(),
    );
    let view = texture.create_view(&Default::default());
    let old = old_pipeline(&device, include_str!("bloom_port.wgsl"));
    let attribution = [
        (
            "native_triangle",
            old_pipeline(&device, include_str!("bloom_triangle_oracle.wgsl")),
        ),
        (
            "native_offset_varyings",
            old_pipeline(&device, include_str!("bloom_offsets_oracle.wgsl")),
        ),
    ];
    let mut rows = Vec::new();
    for (axis_name, axis) in [("horizontal", 1.0_f32), ("vertical", 0.0)] {
        // 140FF1D40 b2: inv_size, axis, centre weight, four pair weights, four offsets.
        // Synthetic, normalized operands exercise every native field; they are not a captured grade.
        let constants = [
            1.0 / SIZE[0] as f32,
            1.0 / SIZE[1] as f32,
            axis,
            0.2,
            0.1,
            0.1,
            0.1,
            0.1,
            1.3,
            3.7,
            5.25,
            7.8,
        ];
        let bytes = bytemuck::cast_slice(&constants).to_vec();
        let bindings = pipeline
            .bindings(
                &arena,
                &BTreeMap::from([
                    (binding(2, Stage::Vertex), bytes.clone()),
                    (binding(2, Stage::Pixel), bytes),
                ]),
                &BTreeMap::from([(binding(0, Stage::Pixel), view.clone())]),
            )
            .map_err(fail)?;
        let compiled = render(&arena, &queue, |pass| {
            pass.set_pipeline(&pipeline.pipeline);
            bindings.bind(pass);
            pass.set_blend_constant(wgpu::Color::WHITE);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..3, 0..1);
        })?;
        let bind = old_bindings(&device, &old.get_bind_group_layout(0), &view, &constants);
        let port = render(&arena, &queue, |pass| {
            pass.set_pipeline(&old);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        })?;
        fs::write(
            output.join(format!("bloom-{axis_name}-compiled.rgba16f")),
            &compiled,
        )?;
        fs::write(
            output.join(format!("bloom-{axis_name}-port.rgba16f")),
            &port,
        )?;
        let mut attribution_rows = Vec::new();
        for (name, oracle) in &attribution {
            let bind = old_bindings(&device, &oracle.get_bind_group_layout(0), &view, &constants);
            let pixels = render(&arena, &queue, |pass| {
                pass.set_pipeline(oracle);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            })?;
            fs::write(
                output.join(format!("bloom-{axis_name}-{name}.rgba16f")),
                &pixels,
            )?;
            let differing = pixels
                .chunks_exact(2)
                .zip(compiled.chunks_exact(2))
                .filter(|(a, b)| a != b)
                .count();
            attribution_rows
                .push(json!({"oracle":name,"differing_half_samples_from_compiled":differing}));
        }
        let mut differing = 0;
        let mut max_error = 0.0_f32;
        let mut sum_error = 0.0_f64;
        for (a, b) in compiled.chunks_exact(2).zip(port.chunks_exact(2)) {
            let a = u16::from_le_bytes([a[0], a[1]]);
            let b = u16::from_le_bytes([b[0], b[1]]);
            differing += usize::from(a != b);
            let error = (half(a) - half(b)).abs();
            max_error = max_error.max(error);
            sum_error += f64::from(error);
        }
        rows.push(json!({"axis":axis_name,"samples":compiled.len()/2,"differing_half_samples":differing,
            "max_absolute_error":max_error,"mean_absolute_error":sum_error/(compiled.len()/2) as f64,
            "attribution":attribution_rows}));
    }
    Ok(
        json!({"effect":"gfx/FX/bloom.shader","technique":"bloom","gpu":format!("{info:?}"),
        "size":SIZE,"format":"RGBA16F little-endian","native_layout_and_registers":true,
        "input_values":"synthetic asymmetric texture and normalized weights; no game-frame equivalence claim", "results":rows}),
    )
}

fn extent() -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: SIZE[0],
        height: SIZE[1],
        depth_or_array_layers: 1,
    }
}

fn old_pipeline(device: &wgpu::Device, source: &str) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("existing bloom port oracle"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("existing bloom port oracle"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("blur_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn old_bindings(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    constants: &[f32],
) -> wgpu::BindGroup {
    let buffer = |bytes: &[u8]| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let uv = buffer(bytemuck::cast_slice(&[1.0_f32, 1.0, 0.0, 0.0]));
    let blur = buffer(bytemuck::cast_slice(constants));
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("existing bloom port inputs"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: uv.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: blur.as_entire_binding(),
            },
        ],
    })
}

/// Submits `draw`, whose uploads are in `arena`, which is not reset.
fn render(
    arena: &shader_pipeline::gpu::Arena,
    queue: &wgpu::Queue,
    draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
) -> io::Result<Vec<u8>> {
    let device = arena.device();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bloom fixture output"),
        size: extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(SIZE[0] * SIZE[1] * 8),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bloom fixture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        draw(&mut pass);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE[0] * 8),
                rows_per_image: None,
            },
        },
        extent(),
    );
    arena.flush(queue);
    queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    let (send, receive) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| fail(e.to_string()))?;
    receive
        .recv()
        .map_err(|e| fail(e.to_string()))?
        .map_err(|e| fail(e.to_string()))?;
    let bytes = slice
        .get_mapped_range()
        .map_err(|e| fail(e.to_string()))?
        .to_vec();
    read.unmap();
    Ok(bytes)
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (bits >> 10) & 31;
    let mantissa = bits & 1023;
    match exponent {
        0 => sign * f32::from(mantissa) * 2.0_f32.powi(-24),
        31 => {
            if mantissa == 0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + f32::from(mantissa) / 1024.0) * 2.0_f32.powi(i32::from(exponent) - 15),
    }
}
