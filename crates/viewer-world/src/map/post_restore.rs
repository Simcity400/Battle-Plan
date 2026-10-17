//! Final scene restore, retaining the seed's b1.
use std::{io, sync::Arc};

use eframe::wgpu;
use installed_content::{ResolvedLayers, image::native::Decode, pdxshader::program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};

use super::fullscreen::{self, Fullscreen};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader scene restore: 1 refused pass: {}",
        reason.into()
    ))
}

pub(super) fn load(
    layers: &ResolvedLayers,
    map: [u32; 2],
    decode: Decode,
) -> io::Result<Arc<Effect>> {
    let (mut library, mut defines) = fullscreen::library(layers, map, decode);
    // Restore-pass variants; the viewer's HDR scene has a resolved LUT.
    defines.extend(["HDR".into(), "COLOR_LUT".into()]);
    library
        .load(b"gfx/FX/restorescene.shader", &defines)
        .map_err(refuse)
}

pub(super) struct Restore(Fullscreen);

impl Restore {
    pub(super) fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> io::Result<Self> {
        // A visual mod's replacement that cannot be drawn here falls back to the recording.
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, format, samples)
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> io::Result<Self> {
        let selection = fullscreen::select(effect, "RestoreBloom").map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        // Bind program/blend and four physical PS textures/samplers.
        // b1 and raster retain; depth remains disabled. Extra reads refuse.
        state
            .samplers
            .retain(|key, _| (0..4).any(|slot| *key == binding(slot, Stage::Pixel)));
        Fullscreen::with_samples(device, &selection, &state, format, samples)
            .map(Self)
            .map_err(refuse)
    }

    pub(super) fn bindings(
        &self,
        arena: &gpu::Arena,
        constants: &[f32; 17],
        sources: [&wgpu::TextureView; 4],
    ) -> io::Result<gpu::Bindings> {
        // Native order: scene, bloom, colour cube, current adapted luminance.
        self.0
            .bind_textures(arena, 1, bytemuck::cast_slice(constants), &sources)
            .map_err(refuse)
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, bindings: &gpu::Bindings) {
        self.0.render(pass, bindings);
    }
}
