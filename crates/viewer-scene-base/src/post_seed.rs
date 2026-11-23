use std::{io, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
use shader_pipeline::{binding, gpu, library::Effect};

use super::{
    BLOOM_FORMAT,
    fullscreen::{self, Fullscreen},
};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader bloom seed: 1 refused pass: {}",
        reason.into()
    ))
}

pub(super) fn load(
    layers: &ResolvedLayers,
    map: [u32; 2],
    decode: Decode,
) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = fullscreen::library(layers, map, decode);
    library
        .load(b"gfx/FX/downsample.shader", &defines)
        .map_err(refuse)
}

pub(super) struct Seed(Fullscreen);

impl Seed {
    pub(super) fn new(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| Self::build(device, effect))
    }

    fn build(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        let selection = fullscreen::select(effect, "downsample").map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| *key == binding(0, Stage::Pixel));
        Fullscreen::new(device, &selection, &state, BLOOM_FORMAT)
            .map(Self)
            .map_err(refuse)
    }

    pub(super) fn kept(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        offset: u32,
        bytes: &[u8],
        source: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        self.0
            .kept(device, buffer, 1, offset, bytes, &[source])
            .map_err(refuse)
    }

    pub(super) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        bindings: &gpu::Bindings,
    ) {
        self.0.draw(encoder, target, bindings, None);
    }
}
