use eframe::wgpu;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use shader_pipeline::{binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};
fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader tree shadow receiver: 1 refused pass: {}",
        reason.into()
    ))
}

fn constants(matrix: [[f32; 4]; 4], fade: f32) -> Vec<u8> {
    let mut bytes = vec![0; 80];
    bytes[..64].copy_from_slice(bytemuck::bytes_of(&matrix));
    bytes[72..76].copy_from_slice(&fade.to_le_bytes());
    bytes
}
pub struct Receiver {
    pub pipeline: Arc<gpu::Pipeline>,
}
impl Receiver {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, format, depth)
        })
    }

    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        let selection = effect
            .select("treeunlit")
            .map_err(refuse)?
            .ok_or_else(|| refuse("treeunlit absent; retained program unavailable"))?;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| (0..16).any(|slot| *key == binding(slot, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, 1);
        pass.vertices = viewer_scene_base::tree_caster::vertex_layouts(&compiled.vertex.inputs)?;
        let raster = RasterizerState::default();
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        viewer_scene_base::shader_constants::both(&pipeline, 1, 80, std::slice::from_ref(&(0..76)))
            .map_err(refuse)?;
        Ok(Self { pipeline })
    }
    pub fn bindings(
        &self,
        arena: &gpu::Arena,
        scene: &[u8],
        matrix: [[f32; 4]; 4],
        fade: f32,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> io::Result<gpu::Bindings> {
        let b1 = constants(matrix, fade);
        self.pipeline
            .bindings(
                arena,
                &BTreeMap::from([
                    (binding(0, Stage::Vertex), scene.to_vec()),
                    (binding(0, Stage::Pixel), scene.to_vec()),
                    (binding(1, Stage::Vertex), b1.clone()),
                    (binding(1, Stage::Pixel), b1),
                ]),
                textures,
            )
            .map_err(refuse)
    }
}
