//! Luminance gather/adaptation, after the shrinking reductions.
use std::{io, sync::Arc};

use eframe::wgpu;
use installed_content::{ResolvedLayers, image::native::Decode, pdxshader::program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};

use super::{
    LUMINANCE_FORMAT,
    fullscreen::{self, Fullscreen},
};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader luminance gather: 1 refused pass: {}",
        reason.into()
    ))
}

pub(super) struct Assets {
    seed: Arc<Effect>,
    luminance: Arc<Effect>,
}

pub(super) fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Assets> {
    let (mut library, defines) = fullscreen::library(layers, map, decode);
    Ok(Assets {
        seed: library
            .load(b"gfx/FX/downsample.shader", &defines)
            .map_err(refuse)?,
        luminance: library
            .load(b"gfx/FX/downsample_luminance.shader", &defines)
            .map_err(refuse)?,
    })
}

pub(super) struct Gather(Fullscreen);

impl Gather {
    pub(super) fn new(device: &wgpu::Device, assets: &Assets) -> io::Result<Self> {
        let seed = fullscreen::select(&assets.seed, "downsample").map_err(refuse)?;
        let mut state = gpu::State::authored(
            &assets.seed.source,
            &assets.seed.source.techniques[seed.index],
        )
        .map_err(refuse)?;
        let selection = fullscreen::select(&assets.luminance, "LuminanceGather").map_err(refuse)?;
        let gather = gpu::State::authored(
            &assets.luminance.source,
            &assets.luminance.source.techniques[selection.index],
        )
        .map_err(refuse)?;
        // Bind the program and physical PS t0/t1+s0/s1 slots. Retain the seed blend, forced depth and
        // renderer raster state.
        state
            .samplers
            .retain(|key, _| *key == binding(0, Stage::Pixel));
        for slot in 0..2 {
            let key = binding(slot, Stage::Pixel);
            if let Some(sampler) = gather.samplers.get(&key) {
                state.samplers.insert(key, sampler.clone());
            }
        }
        let pass = Fullscreen::new(device, &selection, &state, LUMINANCE_FORMAT).map_err(refuse)?;
        pass.check_initialized(2, 48, &[0..12, 16..48])
            .map_err(refuse)?;
        Ok(Self(pass))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        gather: [u32; 2],
        backing: [u32; 2],
        tau_delta: f32,
        hdr: [f32; 2],
        source: &wgpu::TextureView,
        previous: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) -> io::Result<()> {
        // The 48-byte b2 allocation is bound to both stages. Offset 12 is unknown padding checked against
        // compiled reads at construction; bytes 40 through 47 are initialized zeros.
        let constants = [
            1.0,
            1.0,
            tau_delta,
            0.0,
            gather[0] as f32,
            gather[1] as f32,
            1.0 / backing[0] as f32,
            1.0 / backing[1] as f32,
            hdr[0],
            hdr[1],
            0.0,
            0.0,
        ];
        let bindings = self
            .0
            .bind_textures(
                arena,
                2,
                bytemuck::cast_slice(&constants),
                &[source, previous],
            )
            .map_err(refuse)?;
        self.0.draw(encoder, target, &bindings, None);
        Ok(())
    }
}
