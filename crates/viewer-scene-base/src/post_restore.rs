use std::{io, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
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
    ) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, format)
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let selection = fullscreen::select(effect, "RestoreBloom").map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| (0..4).any(|slot| *key == binding(slot, Stage::Pixel)));
        Fullscreen::new(device, &selection, &state, format)
            .map(Self)
            .map_err(refuse)
    }

    pub(super) fn kept(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        offset: u32,
        bytes: &[u8],
        sources: [&wgpu::TextureView; 4],
    ) -> io::Result<gpu::Bindings> {
        self.0
            .kept(device, buffer, 1, offset, bytes, &sources)
            .map_err(refuse)
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, bindings: &gpu::Bindings) {
        self.0.render(pass, bindings);
    }
}
