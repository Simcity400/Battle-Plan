use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_gpu::ribbon_content::{Mode, Vertex};

pub fn shadow_constants(scene: &[u8], matrix: [[f32; 4]; 4]) -> Vec<u8> {
    let mut bytes = viewer_scene_base::tree_caster::constants(matrix);
    for range in [
        128..208,
        220..224,
        236..284,
        288..300,
        304..316,
        320..332,
        336..348,
        352..408,
    ] {
        bytes[range.clone()].copy_from_slice(&scene[range]);
    }
    bytes
}

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader map arrows: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(
    layers: &ResolvedLayers,
    map: [u32; 2],
    decode: Decode,
    name: &str,
) -> io::Result<Arc<Effect>> {
    let fail = |reason| io::Error::other(format!("Map shader {name}: 1 refused pass: {reason}"));
    let path = viewer_scene_base::shader_sources::passes(layers)?
        .into_iter()
        .find(|(pass, _)| pass == name)
        .ok_or_else(|| fail(format!("maparrows.txt has no {name} effect")))?
        .1;
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library.load(path.as_bytes(), &defines).map_err(fail)
}

fn late_depth(mut state: DepthStencilState, late: bool) -> DepthStencilState {
    if late {
        state.stencil_ref = 2;
        state.stencil_read_mask = 2;
        state.stencil_write_mask = 2;
    }
    state
}

pub const SHADOW_SLOT: u32 = 2;

pub struct Arrow {
    retained: viewer_scene_base::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
    reference: u32,
    block: u32,
    written: u32,
    pub texel_reads: Option<String>,
}

impl Arrow {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        technique: &str,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        mode: Mode,
    ) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(
                device, effect, technique, samples, format, depth, mode, false,
            )
        })
    }

    pub fn symbol(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        technique: &str,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
    ) -> io::Result<Self> {
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(
                device,
                effect,
                technique,
                samples,
                format,
                depth,
                Mode::Main,
                true,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        technique: &str,
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        mode: Mode,
        symbol: bool,
    ) -> io::Result<Self> {
        let selection = effect.select(technique).map_err(refuse)?.ok_or_else(|| {
            refuse(format!(
                "{technique:?} absent; retained program unavailable"
            ))
        })?;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let texel_reads = viewer_scene_base::shadow::texel_reads(&compiled, SHADOW_SLOT, technique);
        let technique = &effect.source.techniques[selection.index];
        let mut state = gpu::State::authored(&effect.source, technique).map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| (0..11).any(|i| *key == binding(i, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let mut attributes = Vec::new();
        for input in &compiled.vertex.inputs {
            let (offset, format, width) = match input.semantic.as_str() {
                "POSITION" | "POSITION0" => (0, wgpu::VertexFormat::Float32x3, 3),
                "TEXCOORD" | "TEXCOORD0" if symbol => (12, wgpu::VertexFormat::Float32x3, 3),
                "TEXCOORD" | "TEXCOORD0" => (16, wgpu::VertexFormat::Float32x4, 4),
                _ => return Err(refuse(format!("native arrow cannot supply {input:?}"))),
            };
            if input.kind != ScalarKind::Float || input.components > width {
                return Err(refuse(format!("native arrow cannot supply {input:?}")));
            }
            attributes.push(wgpu::VertexAttribute {
                format,
                offset,
                shader_location: input.location,
            });
        }
        pass.vertices = vec![gpu::VertexLayout {
            stride: if symbol {
                24
            } else {
                std::mem::size_of::<Vertex>() as u64
            },
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        let raster = RasterizerState {
            cull_mode: 0,
            ..Default::default()
        };
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleStrip).0;
        let (depth, reference) = gpu::depth_stencil(
            &late_depth(effect.source.depth_stencil(technique), mode == Mode::Late),
            &raster,
            depth,
        );
        pass.depth_stencil = Some(depth);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        if mode == Mode::Shadow {
            viewer_scene_base::shader_constants::both(&pipeline, 0, 416, &[0..348, 352..408])
                .map_err(refuse)?;
        } else {
            viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        }
        let (block, size, written) = if symbol { (4, 48, 48) } else { (3, 128, 116) };
        viewer_scene_base::shader_constants::both(&pipeline, block, size, std::slice::from_ref(&(0..written)))
            .map_err(refuse)?;
        let retained = viewer_scene_base::shader_scene::Program::new(
            &selection,
            &pass.vertices,
            &state,
            &raster,
            &late_depth(effect.source.depth_stencil(technique), mode == Mode::Late),
        );
        Ok(Self {
            retained,
            pipeline,
            reference,
            block,
            written,
            texel_reads,
        })
    }

    pub fn bindings(
        &self,
        arena: &gpu::Arena,
        scene: &[u8],
        grid: &[u8],
        arrow: &[u8],
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> io::Result<gpu::Bindings> {
        self.pipeline
            .bindings(
                arena,
                &BTreeMap::from([
                    (binding(0, Stage::Vertex), scene.to_vec()),
                    (binding(0, Stage::Pixel), scene.to_vec()),
                    (binding(2, Stage::Pixel), grid.to_vec()),
                    (binding(self.block, Stage::Vertex), arrow.to_vec()),
                    (binding(self.block, Stage::Pixel), arrow.to_vec()),
                ]),
                textures,
            )
            .map_err(refuse)
    }

    pub fn retain(
        &self,
        retained: &mut viewer_scene_base::shader_scene::State,
        scene: &[u8],
        grid: &[u8],
        arrow: &[u8],
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) {
        self.retained.bind(
            retained,
            &BTreeMap::from([
                (binding(0, Stage::Vertex), scene.to_vec()),
                (binding(0, Stage::Pixel), scene.to_vec()),
                (binding(2, Stage::Pixel), grid.to_vec()),
                (binding(self.block, Stage::Vertex), arrow.to_vec()),
                (binding(self.block, Stage::Pixel), arrow.to_vec()),
            ]),
            textures,
        );
        for stage in [Stage::Vertex, Stage::Pixel] {
            retained.stream.bind_partial_constants(
                self.block,
                stage,
                Some(arrow),
                std::slice::from_ref(&(0..self.written)),
            );
        }
    }

    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_stencil_reference(self.reference);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shadow_mode_fills_lighting_without_a_camera_block() {
        let scene: Vec<f32> = (0..104).map(|i| 100. + i as f32).collect();
        let matrix = [[7.; 4]; 4];
        let bytes = shadow_constants(bytemuck::cast_slice(&scene), matrix);
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        assert_eq!(&floats[..16], &[7.; 16]);
        assert_eq!(&floats[16..32], &[0.; 16]);
        assert_eq!(&floats[52..55], &[0.; 3]);
        assert_eq!(&floats[56..59], &[0.; 3]);
        assert_eq!((floats[55], floats[59], floats[91]), (155., 159., 191.));
        assert_eq!(
            (floats[71], floats[75], floats[79], floats[83]),
            (0., 0., 0., 1.)
        );
        assert_eq!((floats[87], floats[102], floats[103]), (0., 0., 0.));
        assert_eq!(&floats[92..102], &scene[92..102]);
        assert_eq!(Mode::Shadow.lighting(), 0.);
        assert!(!Mode::Shadow.admits(false));
        assert!(Mode::Shadow.admits(true));
    }
    #[test]
    fn late_clone_changes_only_stencil_reference_and_masks() {
        let ordinary = DepthStencilState {
            depth_enable: false,
            stencil_enable: true,
            stencil_ref: 1,
            stencil_read_mask: 1,
            stencil_write_mask: 1,
            front_function: 5,
            front_pass: 2,
            ..Default::default()
        };
        let late = late_depth(ordinary.clone(), true);
        assert_eq!(late.stencil_ref, 2);
        assert_eq!(late.stencil_read_mask, 2);
        assert_eq!(late.stencil_write_mask, 2);
        assert_eq!(late_depth(ordinary.clone(), false), ordinary);
        assert_eq!(
            DepthStencilState {
                stencil_ref: 1,
                stencil_read_mask: 1,
                stencil_write_mask: 1,
                ..late
            },
            ordinary
        );
    }
}
