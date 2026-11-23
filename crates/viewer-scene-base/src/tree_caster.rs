use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader tree shadow caster: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/tree.shader", &defines)
        .map_err(refuse)
}

pub fn constants(matrix: [[f32; 4]; 4]) -> Vec<u8> {
    let mut bytes = vec![0; 416];
    bytes[..64].copy_from_slice(bytemuck::bytes_of(&matrix));
    bytes[332..336].copy_from_slice(&1_f32.to_le_bytes());
    bytes[404..408].copy_from_slice(&1_f32.to_le_bytes());
    bytes
}

pub fn vertex_layouts(
    inputs: &[shader_pipeline::VertexInput],
) -> io::Result<Vec<gpu::VertexLayout>> {
    let mut layouts = vec![
        gpu::VertexLayout {
            stride: 56,
            step: wgpu::VertexStepMode::Vertex,
            attributes: Vec::new(),
        },
        gpu::VertexLayout {
            stride: 24,
            step: wgpu::VertexStepMode::Instance,
            attributes: Vec::new(),
        },
    ];
    for input in inputs {
        let (stream, offset, format, count) = match input.semantic.as_str() {
            "POSITION" | "POSITION0" => (0, 0, wgpu::VertexFormat::Float32x3, 3),
            "TEXCOORD" | "TEXCOORD0" => (0, 12, wgpu::VertexFormat::Float32x3, 3),
            "TEXCOORD1" => (0, 24, wgpu::VertexFormat::Float32x4, 4),
            "TEXCOORD2" => (0, 40, wgpu::VertexFormat::Float32x2, 2),
            "TEXCOORD3" => (0, 48, wgpu::VertexFormat::Float32x2, 2),
            "TEXCOORD4" => (1, 0, wgpu::VertexFormat::Float32x4, 4),
            "TEXCOORD5" => (1, 16, wgpu::VertexFormat::Float32x2, 3),
            _ => {
                return Err(refuse(format!(
                    "native tree declaration cannot supply {input:?}"
                )));
            }
        };
        if input.kind != ScalarKind::Float || input.components > count {
            return Err(refuse(format!(
                "native tree declaration cannot supply {input:?}"
            )));
        }
        layouts[stream].attributes.push(wgpu::VertexAttribute {
            format,
            offset,
            shader_location: input.location,
        });
    }
    Ok(layouts)
}

pub struct Caster {
    pub pipeline: Arc<gpu::Pipeline>,
}

impl Caster {
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
            .select("treeshadow")
            .map_err(refuse)?
            .ok_or_else(|| refuse("treeshadow absent; retained program unavailable"))?;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        state.samplers.retain(|key, _| {
            [0, 3, 4, 5, 6, 14]
                .iter()
                .any(|i| *key == binding(*i, Stage::Pixel))
        });
        let mut pass = gpu::Pass::packed(&compiled, &state, format, 1);
        pass.vertices = vertex_layouts(&compiled.vertex.inputs)?;
        let raster = RasterizerState::default();
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        super::shader_constants::tree_caster(&pipeline).map_err(refuse)?;
        Ok(Self { pipeline })
    }

    pub fn bindings(
        &self,
        arena: &gpu::Arena,
        matrix: [[f32; 4]; 4],
        diffuse: &wgpu::TextureView,
        mask: &wgpu::TextureView,
        null: &wgpu::TextureView,
    ) -> io::Result<gpu::Bindings> {
        let bytes = constants(matrix);
        let buffers = BTreeMap::from([
            (binding(0, Stage::Vertex), bytes.clone()),
            (binding(0, Stage::Pixel), bytes),
        ]);
        let mut textures = BTreeMap::from([
            (binding(0, Stage::Pixel), diffuse.clone()),
            (binding(14, Stage::Pixel), mask.clone()),
        ]);
        for slot in 3..7 {
            textures.insert(binding(slot, Stage::Pixel), null.clone());
        }
        self.pipeline
            .bindings(arena, &buffers, &textures)
            .map_err(refuse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caster_uses_constructor_defaults_instead_of_lit_scene_fields() {
        let matrix = [[2.0, 3.0, 4.0, 5.0]; 4];
        let bytes = constants(matrix);
        assert_eq!(&bytes[..64], bytemuck::bytes_of(&matrix));
        assert_eq!(&bytes[64..128], &[0; 64]);
        for offset in [208, 212, 216, 240, 244, 248, 252, 304, 308, 312, 316, 328] {
            assert_eq!(&bytes[offset..offset + 4], &0_f32.to_le_bytes());
        }
        assert_eq!(&bytes[332..336], &1_f32.to_le_bytes());
        assert_eq!(&bytes[404..408], &1_f32.to_le_bytes());
    }
}
