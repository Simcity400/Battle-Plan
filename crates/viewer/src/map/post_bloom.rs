//! Native bloom draw (`140FF1D40`), through the shared content compiler.
use super::{
    BLOOM_FORMAT, Blur, MapSize,
    fullscreen::{self, Fullscreen},
};
use eframe::wgpu;
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};
use std::{io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader bloom: 2 refused passes (horizontal, vertical): {}",
        reason.into()
    ))
}

pub(super) fn load(layers: &ResolvedLayers, map: MapSize) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = fullscreen::library(layers, [map.width, map.height]);
    // 140FF1410 registers this effect and the `bloom` technique.
    library
        .load("gfx/FX/bloom.shader", &defines)
        .map_err(refuse)
}

pub(super) struct Bloom(Fullscreen);

impl Bloom {
    pub(super) fn new(device: &wgpu::Device, effect: &Arc<Effect>) -> io::Result<Self> {
        let selection = fullscreen::select(effect, "bloom").map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        // 142345450 builds the table by physical Index; 140FF1D40 binds only
        // table[0] in PS. Other registers retain preceding draws. Until those
        // are supplied, reading them must refuse.
        state
            .samplers
            .retain(|key, _| *key == binding(0, Stage::Pixel));
        Fullscreen::new(device, &selection, &state, BLOOM_FORMAT)
            .map(Self)
            .map_err(refuse)
    }

    pub(super) fn bindings(
        &self,
        arena: &gpu::Arena,
        blur: &Blur,
        source: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        // 140FF1D40 overwrites b2 in VS+PS and t0 in PS for each axis.
        self.0
            .bindings(arena, 2, bytemuck::bytes_of(blur), source)
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
