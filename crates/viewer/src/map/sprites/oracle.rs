//! Offscreen access to the native data model shader, sharing the production vertex ABI.
use super::*;
use wgpu::util::DeviceExt;
mod animation;
pub use animation::animated_source;

/// Off-grid interpolation needs the adapter's texture filtering precision. The
/// handwritten bilinear oracle remains useful for the aligned sampler fixtures.
pub fn hardware_source(source: &str) -> String {
    let mut source = source.to_owned();
    for (index, slot) in [0, 1, 5].into_iter().enumerate() {
        source = source.replace(
            &format!("sample_slot(slot{slot},"),
            &format!("sample_native(slot{slot}, native_sampler{index},"),
        );
        source.push_str(&format!(
            "\n@group(0) @binding({}) var native_sampler{index}:sampler;\n",
            8 + index
        ));
    }
    source.push_str("\nfn sample_native(t:texture_2d<f32>, s:sampler, uv:vec2<f32>, state:u32, lod:f32)->vec4<f32>{ return textureSampleLevel(t,s,uv,0.0); }\n");
    source
}

/// Exercise the production upload, sorting, pipeline cache and draw loop offscreen.
pub fn render_batch(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
) -> (Vec<u8>, std::collections::BTreeMap<String, u64>, usize) {
    let page = dds::Texture::from_rgba8(installed_content::image::Rgba8Image {
        width: 1,
        height: 1,
        pixels: vec![255; 4],
    });
    render_images(device, queue, batch, &vec![page; 16])
}

pub fn render_images(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    batch: &Batch,
    pages: &[dds::Texture],
) -> (Vec<u8>, std::collections::BTreeMap<String, u64>, usize) {
    let images = IconTextures::from_images(device, queue, pages).unwrap();
    let mut pass = SpritePass::new(
        device,
        queue,
        1,
        &images,
        &LazyTextures::new(),
        wgpu::TextureFormat::Rgba8Unorm,
    )
    .unwrap();
    // The frame's one arena, as the scene owns it: reset per frame, flushed before its draws.
    let arena = shader_pipeline::gpu::Arena::new(device);
    arena.reset();
    pass.upload(device, queue, &arena, batch, [16, 16], 1.);
    let first_count = pass.native.cache.created;
    let bake_count = pass.bake_stream.cache.created;
    let bake_views: Vec<_> = pass.bakes.iter().map(|b| b.view.clone()).collect();
    arena.reset();
    pass.upload(device, queue, &arena, batch, [16, 16], 1.);
    arena.flush(queue);
    assert_eq!(
        first_count, pass.native.cache.created,
        "second frame reuses pipelines"
    );
    assert_eq!(
        bake_count, pass.bake_stream.cache.created,
        "bake pipelines reused"
    );
    assert_eq!(
        bake_views,
        pass.bakes
            .iter()
            .map(|b| b.view.clone())
            .collect::<Vec<_>>(),
        "unchanged bake targets reused"
    );
    let pixels = crate::map::native_shader::tests::render_with(device, queue, |render| {
        pass.draw_runs(render, [16, 16])
    });
    (pixels, pass.native_refusals, first_count)
}

pub struct Oracle {
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    group: wgpu::BindGroup,
    animation: Option<wgpu::BindGroup>,
}

impl Oracle {
    /// Identical authored blend/write state isolates program and sampler differences. The
    /// SpritePass's historical colour-only target is intentionally not part of this oracle.
    pub fn with_source(
        device: &wgpu::Device,
        quad: Quad,
        views: &[&wgpu::TextureView],
        blend: Option<wgpu::BlendState>,
        write_mask: wgpu::ColorWrites,
        source: &str,
    ) -> Self {
        Self::with_format(
            device,
            quad,
            views,
            blend,
            write_mask,
            source,
            wgpu::TextureFormat::Rgba8Unorm,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_format(
        device: &wgpu::Device,
        quad: Quad,
        views: &[&wgpu::TextureView],
        blend: Option<wgpu::BlendState>,
        write_mask: wgpu::ColorWrites,
        source: &str,
        format: wgpu::TextureFormat,
    ) -> Self {
        let exact = quad.exact.unwrap();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("native effect model oracle"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("native effect model oracle"), layout: None,
            vertex: wgpu::VertexState { module: &module, entry_point: Some("exact_vertex"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ExactVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x2,2=>Float32x2,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4,8=>Uint32,9=>Uint32],
            })] },
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("exact_fragment"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask })] }),
            multiview_mask: None, cache: None,
        });
        let clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            lod_max_clamp: 0.,
            ..Default::default()
        });
        let wrap = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            lod_max_clamp: 0.,
            ..Default::default()
        });
        let mut entries: Vec<_> = views
            .iter()
            .enumerate()
            .map(|(binding, view)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect();
        if source.contains("var animation_sampler") {
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&clamp),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::Sampler(&wrap),
                },
            ]);
        }
        let native_samplers: Vec<_> = (0..3)
            .map(|i| {
                let state = (exact.samplers >> (i * 10)) & 1023;
                let mode = |v| match v {
                    0 => wgpu::AddressMode::Repeat,
                    1 => wgpu::AddressMode::MirrorRepeat,
                    _ => wgpu::AddressMode::ClampToEdge,
                };
                device.create_sampler(&wgpu::SamplerDescriptor {
                    address_mode_u: mode(state & 7),
                    address_mode_v: mode((state >> 3) & 7),
                    mag_filter: if state & 64 != 0 {
                        wgpu::FilterMode::Linear
                    } else {
                        wgpu::FilterMode::Nearest
                    },
                    min_filter: if state & 128 != 0 {
                        wgpu::FilterMode::Linear
                    } else {
                        wgpu::FilterMode::Nearest
                    },
                    lod_max_clamp: 0.,
                    ..Default::default()
                })
            })
            .collect();
        if source.contains("var native_sampler0") {
            entries.extend(
                native_samplers
                    .iter()
                    .enumerate()
                    .map(|(i, s)| wgpu::BindGroupEntry {
                        binding: 8 + i as u32,
                        resource: wgpu::BindingResource::Sampler(s),
                    }),
            );
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oracle textures"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let vertices: Vec<_> = [0, 1, 2, 0, 2, 3]
            .into_iter()
            .map(|i| ExactVertex {
                position: exact.corners[i],
                uv: exact.uv[i],
                mask_uv: exact.uv[i].map(|v| (v * 1000.).clamp(0., 1.)),
                color: quad.color,
                second: exact.second_color,
                offsets: exact.offsets,
                inputs: exact.inputs,
                lod_bias: [exact.lod_bias[0], exact.lod_bias[1], exact.lod_bias[2], 0.],
                pixel: exact.pixel,
                samplers: exact.samplers,
            })
            .collect();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native effect model vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Self {
            pipeline,
            vertices,
            group,
            animation: None,
        }
    }

    pub fn animation(mut self, device: &wgpu::Device, records: &[[[f32; 4]; 5]]) -> Self {
        let mut data = records.to_vec();
        data.resize(2, [[0.; 4]; 5]);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("oracle b1"),
            contents: bytemuck::cast_slice(&data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        self.animation = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oracle b1"),
            layout: &self.pipeline.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        }));
        self
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_bind_group(0, &self.group, &[]);
        if let Some(animation) = &self.animation {
            pass.set_bind_group(1, animation, &[]);
        }
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..6, 0..1);
    }
}
