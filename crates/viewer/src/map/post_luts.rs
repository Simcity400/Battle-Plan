//! The native `lut_blender.shader` draws. `14126f930` allocates three 1024x32
//! BGRA8 UNORM targets; `141270d90`/`141271150` decide which blends run. Each
//! blend stores UNORM8 before the next reads it, and Restore samples the result,
//! so the rounding matches the game's rather than a blend of sampled colours.

use std::{collections::HashMap, io};

use eframe::wgpu;

use super::{LutProgram, LutSource, PostAssets, invalid, texture_entry};

/// The engine's colour-cube size: 32 blue slices of 32x32 side by side.
const LUT_SIZE: [u32; 2] = [1024, 32];

pub(super) struct LutRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sources: Vec<wgpu::TextureView>,
    intermediate: [wgpu::TextureView; 3],
    /// What Restore binds: a copy of the frame's result, so its bind group
    /// stays valid whichever source or intermediate the grade ends on.
    output: wgpu::TextureView,
    /// One factor buffer per draw: three blends and the output copy.
    factors: [wgpu::Buffer; 4],
    /// Bind groups by draw slot and sources, so a changing factor (a height
    /// fade while zooming) reuses them instead of allocating per frame.
    binds: HashMap<(usize, [LutSource; 4]), wgpu::BindGroup>,
    last: Option<(LutProgram, LutSource)>,
}

impl LutRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &PostAssets,
    ) -> io::Result<Self> {
        if let Some(lut) = assets
            .luts
            .iter()
            .find(|lut| [lut.width, lut.height] != LUT_SIZE)
        {
            return Err(invalid(format!(
                "a post-effect colour cube is {}x{}, not 1024x32",
                lut.width, lut.height
            )));
        }
        let extent = |layers| wgpu::Extent3d {
            width: LUT_SIZE[0],
            height: LUT_SIZE[1],
            depth_or_array_layers: layers,
        };
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("post source colour cubes"),
            size: extent(assets.luts.len() as u32),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sources = (0..assets.luts.len() as u32)
            .zip(&assets.luts)
            .map(|(layer, lut)| {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &source,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &lut.pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(LUT_SIZE[0] * 4),
                        rows_per_image: Some(LUT_SIZE[1]),
                    },
                    extent(1),
                );
                source.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("post source colour cube"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let target = || {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("post blended colour cube"),
                    size: extent(1),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let factors = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("post LUT blend factors"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let entries: Vec<_> = std::iter::once(wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
        .chain((1..=4).map(|binding| {
            texture_entry(
                binding,
                wgpu::ShaderStages::FRAGMENT,
                wgpu::TextureViewDimension::D2,
            )
        }))
        .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post LUT inputs"),
            entries: &entries,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post LUT blend"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_luts.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post LUT blend"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post LUT blend"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("blend_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            layout,
            sources,
            intermediate: std::array::from_fn(|_| target()),
            output: target(),
            factors,
            binds: HashMap::new(),
            last: None,
        })
    }

    pub(super) fn output(&self) -> &wgpu::TextureView {
        &self.output
    }

    fn view(&self, source: LutSource) -> &wgpu::TextureView {
        match source {
            LutSource::Asset(index) => &self.sources[index],
            LutSource::Intermediate(index) => &self.intermediate[index],
        }
    }

    /// Runs the frame's blends, then copies `result` into [`Self::output`].
    /// An unchanged program draws nothing.
    pub(super) fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        program: LutProgram,
        result: LutSource,
    ) {
        if self.last == Some((program, result)) {
            return;
        }
        for index in 0..=program.len {
            let (sources, factors) = match program.operations[..program.len].get(index) {
                Some(operation) => (operation.sources, operation.factors),
                None => ([result; 4], [0.0; 2]),
            };
            queue.write_buffer(
                &self.factors[index],
                0,
                bytemuck::cast_slice(&[factors[0], factors[1], 0.0, 0.0]),
            );
            let key = (index, sources);
            if !self.binds.contains_key(&key) {
                let entries: Vec<_> = std::iter::once(wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.factors[index].as_entire_binding(),
                })
                .chain(
                    (1..)
                        .zip(sources)
                        .map(|(binding, source)| wgpu::BindGroupEntry {
                            binding,
                            resource: wgpu::BindingResource::TextureView(self.view(source)),
                        }),
                )
                .collect();
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("post LUT blend inputs"),
                    layout: &self.layout,
                    entries: &entries,
                });
                self.binds.insert(key, bind);
            }
            let target = if index < program.len {
                &self.intermediate[index]
            } else {
                &self.output
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post LUT blend"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.binds[&key], &[]);
            pass.draw(0..3, 0..1);
        }
        self.last = Some((program, result));
    }
}
