use std::{io, sync::Arc};

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::program::Stage;
use content_reader_image::native::Decode;
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

impl Assets {
    fn recorded(&self, why: &str) -> Option<Self> {
        let (seed, luminance) = (self.seed.fall_back(why), self.luminance.fall_back(why));
        (seed.is_some() || luminance.is_some()).then(|| Self {
            seed: seed.unwrap_or_else(|| self.seed.clone()),
            luminance: luminance.unwrap_or_else(|| self.luminance.clone()),
        })
    }
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
        Self::build(device, assets).or_else(|error| match assets.recorded(&error.to_string()) {
            Some(recorded) => Self::build(device, &recorded),
            None => Err(error),
        })
    }

    fn build(device: &wgpu::Device, assets: &Assets) -> io::Result<Self> {
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

    pub(super) fn constants(
        gather: [u32; 2],
        backing: [u32; 2],
        tau_delta: f32,
        hdr: [f32; 2],
    ) -> [f32; 12] {
        [
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
        ]
    }

    pub(super) fn kept(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        offset: u32,
        bytes: &[u8],
        source: &wgpu::TextureView,
        previous: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        self.0
            .kept(device, buffer, 2, offset, bytes, &[source, previous])
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
