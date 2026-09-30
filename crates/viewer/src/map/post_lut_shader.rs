//! Native LUT blend registration14126F590 and submission141271BD0.
use std::{io, sync::Arc};

use eframe::wgpu;
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};

use super::fullscreen::{self, Fullscreen};

// 14126F930 allocates three 1024x32 targets in native format0xD.
pub(super) const SIZE: [u32; 2] = [1024, 32];
pub(super) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader LUT blend: 1 refused pass: {}",
        reason.into()
    ))
}

pub(super) fn load(layers: &ResolvedLayers, map: [u32; 2]) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = fullscreen::library(layers, map);
    library
        .load("gfx/FX/lut_blender.shader", &defines)
        .map_err(refuse)
}

pub(super) struct Blends {
    draws: [Fullscreen; 2],
    null: wgpu::TextureView,
}

impl Blends {
    pub(super) fn new(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        let build = |name| -> Result<Fullscreen, String> {
            let selection = fullscreen::select(effect, name)?;
            let source = &effect.source;
            let technique = &source.techniques[selection.index];
            let compiled = effect.compiled(selection.index)?;
            let mut state = gpu::State::authored(source, technique)?;
            // 141270CA0/141270D90 bind four physical PS texture/sampler slots.
            state
                .samplers
                .retain(|slot, _| (0..4).any(|i| *slot == binding(i, Stage::Pixel)));
            let mut pass = gpu::Pass::packed(&compiled, &state, FORMAT, 1);
            let (primitive, _) = gpu::primitive(
                &source.rasterizer(technique),
                wgpu::PrimitiveTopology::TriangleList,
            );
            // 142400380 sets the scissor to the entire target when binding it,
            // matching wgpu's full-attachment default even when authored enabled.
            pass.primitive = primitive;
            // 14126F930 creates colour-only targets; 141271BD0 attaches no DSV.
            // Authored depth/stencil is bound natively but has no attachment to test.
            Fullscreen::from_pass(device, &selection, &state, pass)
        };
        let null = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("native null LUT texture"),
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
            })
            .create_view(&Default::default());
        Ok(Self {
            draws: [
                build("LutBlend1").map_err(refuse)?,
                build("LutBlend2").map_err(refuse)?,
            ],
            null,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        sources: [&wgpu::TextureView; 4],
        factors: [f32; 2],
        double: bool,
    ) -> io::Result<()> {
        // 141271BD0 writes BOTH inverse dimensions as1/1024, then factors.
        let constants = [
            1.0 / 1024.0,
            1.0 / 1024.0,
            if double { factors[0] } else { 0.0 },
            factors[1],
        ];
        let sources = if double {
            sources
        } else {
            [sources[0], &self.null, sources[2], &self.null]
        };
        let draw = &self.draws[usize::from(double)];
        let bindings = draw
            .bind_textures(arena, 0, bytemuck::cast_slice(&constants), &sources)
            .map_err(refuse)?;
        // 141271BD0 does not clear: authored RGB-only writes retain alpha.
        // Native initial target bytes are unspecified (1423FB4F0 null data).
        draw.draw(encoder, target, &bindings, Some(SIZE));
        Ok(())
    }
}
