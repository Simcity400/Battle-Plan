//! Offline GPU ABI validation. Inputs are synthetic, not native engine bindings.
use std::collections::BTreeMap;

use eframe::wgpu;
use installed_content::pdxshader::program::{Program, Technique};
use serde_json::{Value, json};
use shader_pipeline::{Compiled, TextureKind, gpu};

pub(super) struct Probe {
    device: wgpu::Device,
    pub adapter: String,
}

impl Probe {
    pub(super) fn new() -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .map_err(|e| e.to_string())?;
        let info = format!("{:?}", adapter.get_info());
        let (device, _) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features() & wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self {
            device,
            adapter: info,
        })
    }

    pub(super) fn bind(
        &self,
        compiled: &Compiled,
        source: &Program,
        technique: &Technique,
    ) -> Result<Value, String> {
        let state = gpu::State::authored(source, technique)?;
        // The tightly packed declaration is an ABI fixture, not the game's vertex layout.
        let pass = gpu::Pass::packed(compiled, &state, wgpu::TextureFormat::Rgba16Float, 1);
        let pipeline =
            gpu::Pipeline::for_pass(&self.device, compiled, source, technique, &state, &pass)?;
        let constants: BTreeMap<_, _> = compiled
            .vertex
            .buffers
            .iter()
            .chain(&compiled.pixel.buffers)
            .map(|buffer| (buffer.binding, vec![0; buffer.size as usize]))
            .collect();
        let textures: BTreeMap<_, _> = compiled
            .vertex
            .textures
            .iter()
            .chain(&compiled.pixel.textures)
            .map(|&(binding, kind)| {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("synthetic shader ABI texture"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: if kind == TextureKind::Cube { 6 } else { 1 },
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: if kind == TextureKind::D3 {
                        wgpu::TextureDimension::D3
                    } else {
                        wgpu::TextureDimension::D2
                    },
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                (
                    binding,
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(kind.view()),
                        ..Default::default()
                    }),
                )
            })
            .collect();
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let arena = gpu::Arena::new(&self.device);
        let bindings = pipeline.bindings(&arena, &constants, &textures);
        let error = pollster::block_on(validation.pop());
        bindings?;
        if let Some(error) = error {
            return Err(error.to_string());
        }
        Ok(json!({
            "pipeline_and_bindings_created": true,
            "native_inputs": false,
            "scope": "synthetic zero-filled reflected constants/textures, packed typed vertices; no draw or engine-state equivalence claim",
            "constant_buffers": constants.len(), "textures": textures.len(),
            "vertex_attributes": compiled.vertex.inputs.len(),
        }))
    }
}
