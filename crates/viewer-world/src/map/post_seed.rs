//! Bright/log seed; the same native b1 remains bound through restore.
use std::{io, sync::Arc};

use eframe::wgpu;
use installed_content::{ResolvedLayers, image::native::Decode, pdxshader::program::Stage};
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
        let selection = fullscreen::select(effect, "downsample").map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        // Bind the program, blend and physical PS table[0]. Depth is forced off and renderer raster state
        // is retained. Other reads still refuse.
        state
            .samplers
            .retain(|key, _| *key == binding(0, Stage::Pixel));
        Fullscreen::new(device, &selection, &state, BLOOM_FORMAT)
            .map(Self)
            .map_err(refuse)
    }

    pub(super) fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        constants: &[f32; 17],
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) -> io::Result<()> {
        // Bind b1 to both stages with a 68-byte initialized payload in an 80-byte allocation. The final
        // register padding remains unknown.
        let bindings = self
            .0
            .bindings(arena, 1, bytemuck::cast_slice(constants), source)
            .map_err(refuse)?;
        self.0.draw(encoder, target, &bindings, None);
        Ok(())
    }
}
