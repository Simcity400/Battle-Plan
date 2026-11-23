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
        "Map shader luminance reductions: 1 refused pass: {}",
        reason.into()
    ))
}

pub(super) struct Assets {
    seed: Arc<Effect>,
    normal: Arc<Effect>,
    alpha: Arc<Effect>,
}

impl Assets {
    fn recorded(&self, why: &str) -> Option<Self> {
        let [seed, normal, alpha] =
            [&self.seed, &self.normal, &self.alpha].map(|effect| effect.fall_back(why));
        (seed.is_some() || normal.is_some() || alpha.is_some()).then(|| Self {
            seed: seed.unwrap_or_else(|| self.seed.clone()),
            normal: normal.unwrap_or_else(|| self.normal.clone()),
            alpha: alpha.unwrap_or_else(|| self.alpha.clone()),
        })
    }
}

pub(super) fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Assets> {
    let (mut library, mut defines) = fullscreen::library(layers, map, decode);
    let seed = library
        .load(b"gfx/FX/downsample.shader", &defines)
        .map_err(refuse)?;
    let normal = library
        .load(b"gfx/FX/downsample_luminance.shader", &defines)
        .map_err(refuse)?;
    defines.push("LUMINANCE_SAMPLE_ALPHA".into());
    let alpha = library
        .load(b"gfx/FX/downsample_luminance.shader", &defines)
        .map_err(refuse)?;
    Ok(Assets {
        seed,
        normal,
        alpha,
    })
}

pub(super) struct Reductions([Fullscreen; 2]);

impl Reductions {
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
        let normal = fullscreen::select(&assets.normal, "LuminanceDownsample").map_err(refuse)?;
        let alpha = fullscreen::select(&assets.alpha, "LuminanceDownsample").map_err(refuse)?;
        let samplers = gpu::State::authored(
            &assets.normal.source,
            &assets.normal.source.techniques[normal.index],
        )
        .map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| *key == binding(0, Stage::Pixel));
        if let Some(sampler) = samplers.samplers.get(&binding(0, Stage::Pixel)) {
            state
                .samplers
                .insert(binding(0, Stage::Pixel), sampler.clone());
        }
        Ok(Self([
            Fullscreen::new(device, &alpha, &state, LUMINANCE_FORMAT).map_err(refuse)?,
            Fullscreen::new(device, &normal, &state, LUMINANCE_FORMAT).map_err(refuse)?,
        ]))
    }

    pub(super) fn constants(viewport: [u32; 2], uv_scale: [f32; 2]) -> [f32; 10] {
        [
            1.0 / viewport[0] as f32,
            1.0 / viewport[1] as f32,
            0.0,
            0.0,
            uv_scale[0],
            uv_scale[1],
            0.0,
            0.0,
            0.0,
            0.0,
        ]
    }

    pub(super) fn kept(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        index: usize,
        offset: u32,
        bytes: &[u8],
        source: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        self.0[usize::from(index != 0)]
            .kept(device, buffer, 2, offset, bytes, &[source])
            .map_err(refuse)
    }

    pub(super) fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        viewport: [u32; 2],
        target: &wgpu::TextureView,
        bindings: &gpu::Bindings,
    ) {
        self.0[usize::from(index != 0)].draw(encoder, target, bindings, Some(viewport));
    }
}
