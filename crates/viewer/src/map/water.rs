//! The `water` pass: a flat plane at `WATER_HEIGHT` over the visible rectangle.

use std::io;

use eframe::wgpu;

use super::{
    scene::{DEPTH_FORMAT, SAMPLE_COUNT, Targets, module, sampler_entry, texture_entry},
    textures::{Bound, Samplers},
};

/// The separate sea-bed target bound at water t5 in frame33840, event 10338.
/// It stores linear colour in eight-bit UNORM, without multisampling.
pub const REFRACTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// Water-specific textures. Tiled point-light maps are shared through group 0.
/// `GradientBorderChannel3` belongs to the unrecorded dominance effect.
pub struct WaterInputs<'a> {
    pub heightmap: &'a Bound,
    pub lean1: &'a Bound,
    pub lean2: &'a Bound,
    pub secondary: &'a Bound,
    pub specular: &'a Bound,
    pub refraction: &'a wgpu::TextureView,
    pub ice_diffuse: &'a Bound,
    pub ice_noise: &'a Bound,
    pub reflection: &'a Bound,
    pub snow_mud: &'a Bound,
    pub gradient1: &'a Bound,
    pub gradient2: &'a Bound,
}

/// The water pipeline and the bind group it draws with.
pub struct WaterPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
}

impl WaterPass {
    pub fn new(
        device: &wgpu::Device,
        globals: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let mut entries = (0..8)
            .map(|binding| {
                texture_entry(
                    binding,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::TextureViewDimension::D2,
                )
            })
            .collect::<Vec<_>>();
        entries.push(texture_entry(
            8,
            wgpu::ShaderStages::FRAGMENT,
            wgpu::TextureViewDimension::Cube,
        ));
        for binding in 9..12 {
            entries.push(texture_entry(
                binding,
                wgpu::ShaderStages::FRAGMENT,
                wgpu::TextureViewDimension::D2,
            ));
        }
        entries.push(sampler_entry(12));
        entries.push(sampler_entry(13));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water inputs"),
            entries: &entries,
        });
        let module = module(device, "water", include_str!("water.wgsl"));
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("water"),
            bind_group_layouts: &[Some(globals), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                // Native water follows trees and writes depth (frame32511 event10001).
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLE_COUNT,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("water_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // `pdxwater.shader:420-427`: src_alpha / inv_src_alpha with
                    // the alpha channel left to the passes that read it.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            layout,
            bind_group: None,
        })
    }

    /// Rebuilds the bind group; needed whenever a bound texture is replaced.
    pub fn rebind(&mut self, device: &wgpu::Device, samplers: &Samplers, inputs: WaterInputs<'_>) {
        let views = [
            &inputs.heightmap.view,
            &inputs.lean1.view,
            &inputs.lean2.view,
            &inputs.secondary.view,
            &inputs.specular.view,
            inputs.refraction,
            &inputs.ice_diffuse.view,
            &inputs.ice_noise.view,
            &inputs.reflection.view,
            &inputs.snow_mud.view,
            &inputs.gradient1.view,
            &inputs.gradient2.view,
        ];
        let mut bindings = views
            .iter()
            .enumerate()
            .map(|(binding, view)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect::<Vec<_>>();
        bindings.push(wgpu::BindGroupEntry {
            binding: 12,
            resource: wgpu::BindingResource::Sampler(&samplers.linear_wrap),
        });
        bindings.push(wgpu::BindGroupEntry {
            binding: 13,
            resource: wgpu::BindingResource::Sampler(&samplers.linear_clamp),
        });
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water inputs"),
            layout: &self.layout,
            entries: &bindings,
        }));
    }

    /// Blends the water plane over the terrain already in the target.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        globals: &wgpu::BindGroup,
    ) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("water"),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}
