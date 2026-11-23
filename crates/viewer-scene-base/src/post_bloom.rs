use super::{
    BLOOM_FORMAT, Blur, MapSize,
    fullscreen::{self, Fullscreen},
};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
use shader_pipeline::{binding, gpu, library::Effect};
use std::{io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader bloom: 2 refused passes (horizontal, vertical): {}",
        reason.into()
    ))
}

pub(super) fn load(
    layers: &ResolvedLayers,
    map: MapSize,
    decode: Decode,
) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = fullscreen::library(layers, [map.width, map.height], decode);
    library
        .load(b"gfx/FX/bloom.shader", &defines)
        .map_err(refuse)
}

pub(super) struct Bloom(Fullscreen);

impl Bloom {
    pub(super) fn new(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| Self::build(device, effect))
    }

    fn build(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        let selection = fullscreen::select(effect, "bloom").map_err(refuse)?;
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
        blur: &Blur,
        source: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        self.0
            .kept(
                device,
                buffer,
                2,
                offset,
                bytemuck::bytes_of(blur),
                &[source],
            )
            .map_err(refuse)
    }

    pub(super) fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        bindings: &gpu::Bindings,
    ) {
        self.0.draw(encoder, target, bindings, None);
    }
}
