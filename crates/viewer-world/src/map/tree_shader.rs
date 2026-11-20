//! Lit-tree submission selected by the tree graphics setting.
use eframe::wgpu;
use installed_content_core::{
    pdxshader::DepthStencilState, pdxshader::RasterizerState, pdxshader::program::Stage,
};
use shader_pipeline::{binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader tree canopy: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn technique(high: bool) -> &'static str {
    if high { "tree" } else { "tree_low_gfx" }
}

/// Lit mode clears the matrix and writes zero seasons. Its 80-byte allocation retains an unknown
/// tail at bytes 76 through 79.
fn constants(fade: f32) -> Vec<u8> {
    let mut bytes = vec![0; 80];
    bytes[72..76].copy_from_slice(&fade.to_le_bytes());
    bytes
}

/// The pixel slot the engine binds the shadow screen buffer to for the lit canopy.
pub const SHADOW_SLOT: u32 = 8;

pub struct Lit {
    retained: super::shader_scene::Program,
    pub pipeline: Arc<gpu::Pipeline>,
    /// How the program reads the shadow screen buffer by texel ([`super::shadow::texel_reads`]).
    pub texel_reads: Option<String>,
}

impl Lit {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        samples: u32,
        high: bool,
    ) -> io::Result<Self> {
        // A visual mod's replacement that cannot be drawn here falls back to the recording.
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, format, depth, samples, high)
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        samples: u32,
        high: bool,
    ) -> io::Result<Self> {
        let name = technique(high);
        let selection = effect
            .select(name)
            .map_err(refuse)?
            .ok_or_else(|| refuse(format!("{name} absent; retained program unavailable")))?;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let texel_reads = super::shadow::texel_reads(&compiled, SHADOW_SLOT, name);
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| (0..16).any(|slot| *key == binding(slot, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        pass.vertices = super::tree_caster::vertex_layouts(&compiled.vertex.inputs)?;
        // Use the tree raster state and retain renderer depth restored by rivers and railways.
        let raster = RasterizerState::default();
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        super::shader_constants::scene(&pipeline).map_err(refuse)?;
        super::shader_constants::both(&pipeline, 1, 80, std::slice::from_ref(&(0..76)))
            .map_err(refuse)?;
        let retained = super::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &raster,
            &DepthStencilState::default(),
        );
        Ok(Self {
            retained,
            pipeline,
            texel_reads,
        })
    }

    pub fn retain(
        &self,
        retained: &mut super::shader_scene::State,
        scene: &[u8],
        fade: f32,
        b2: super::shader_constants::WorldPixelConstants,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) {
        let b1 = constants(fade);
        self.retained.bind(
            retained,
            &BTreeMap::from([
                (binding(0, Stage::Vertex), scene.to_vec()),
                (binding(0, Stage::Pixel), scene.to_vec()),
                (binding(1, Stage::Vertex), b1.clone()),
                (binding(1, Stage::Pixel), b1.clone()),
            ]),
            textures,
        );
        b2.retain(retained);
        for stage in [Stage::Vertex, Stage::Pixel] {
            retained.stream.bind_partial_constants(
                1,
                stage,
                Some(&b1),
                std::slice::from_ref(&(0..76)),
            );
        }
    }

    pub fn bindings(
        &self,
        arena: &gpu::Arena,
        scene: &[u8],
        fade: f32,
        retained_pixel_b2: super::shader_constants::WorldPixelConstants,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> io::Result<gpu::Bindings> {
        retained_pixel_b2.check(&self.pipeline).map_err(refuse)?;
        let b1 = constants(fade);
        self.pipeline
            .bindings(
                arena,
                &BTreeMap::from([
                    (binding(0, Stage::Vertex), scene.to_vec()),
                    (binding(0, Stage::Pixel), scene.to_vec()),
                    (binding(1, Stage::Vertex), b1.clone()),
                    (binding(1, Stage::Pixel), b1),
                    (binding(2, Stage::Pixel), retained_pixel_b2.bytes().to_vec()),
                ]),
                textures,
            )
            .map_err(refuse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lit_constants_have_zero_matrix_and_seasons() {
        let bytes = constants(0.375);
        assert_eq!(&bytes[..72], &[0; 72]);
        assert_eq!(&bytes[72..76], &0.375_f32.to_le_bytes());
        assert_eq!(technique(true), "tree");
        assert_eq!(technique(false), "tree_low_gfx");
    }
}
