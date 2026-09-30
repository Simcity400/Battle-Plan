//! Native luminance reductions (`140FF2890`), preserving the fixed backing targets.
use std::{io, sync::Arc};

use eframe::wgpu;
use installed_content::{ResolvedLayers, pdxshader::program::Stage};
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

pub(super) fn load(layers: &ResolvedLayers, map: [u32; 2]) -> io::Result<Assets> {
    let (mut library, mut defines) = fullscreen::library(layers, map);
    // 140FF1410 registers both reductions; their blend is retained from seed.
    let seed = library
        .load("gfx/FX/downsample.shader", &defines)
        .map_err(refuse)?;
    let normal = library
        .load("gfx/FX/downsample_luminance.shader", &defines)
        .map_err(refuse)?;
    defines.push("LUMINANCE_SAMPLE_ALPHA".into());
    let alpha = library
        .load("gfx/FX/downsample_luminance.shader", &defines)
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
        // 140FF2890 binds only the program: seed's blend/depth/raster retain.
        // Both variants use the NON-alpha effect's physical sampler table[0].
        // A null entry retains seed's sampler; higher registers remain
        // unsupported until the preceding frame's bind stream is reconstructed.
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        viewport: [u32; 2],
        uv_scale: [f32; 2],
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) -> io::Result<()> {
        // 140FF2890 uploads 40 initialized bytes into the shared 48-byte b2:
        // inverse viewport float2 + zero float2, UVScale float2 + zero float2,
        // zero float2. Only the final register's padding is not an operand.
        let constants = [
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
        ];
        let pass = &self.0[usize::from(index != 0)];
        let bindings = pass
            .bindings(arena, 2, bytemuck::cast_slice(&constants), source)
            .map_err(refuse)?;
        pass.draw(encoder, target, &bindings, Some(viewport));
        Ok(())
    }
}
