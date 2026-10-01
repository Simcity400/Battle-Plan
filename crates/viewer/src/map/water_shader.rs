//! Water draw 1412661E0; registration141261100 and strip allocation141267C51.
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{
    ResolvedLayers,
    image::native::Decode,
    pdxshader::{DepthStencilState, RasterizerState, program::Stage},
};
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader water: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/pdxwater.shader", &defines)
        .map_err(refuse)
}

pub fn technique(high: bool, refractions: bool) -> &'static str {
    // 141266258..2B0: graphics+18 wins over graphics+15; debug gate14306E9DB defaults1.
    if !high {
        "water_low_gfx"
    } else if refractions {
        "water"
    } else {
        "water_no_refractions"
    }
}

fn quad(map: [u32; 2]) -> [[i16; 2]; 4] {
    // 141267C5F..C84 stores the map bitmap dimensions as16-bit words.
    let [w, h] = map.map(|v| v as i16);
    [[w, 0], [0, 0], [w, h], [0, h]]
}

pub struct Inputs<'a> {
    pub retained: Option<&'a std::cell::RefCell<super::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub time: f32,
    pub size: [u32; 2],
    pub retained_pixel_b2: super::shader_constants::WorldPixelConstants,
    pub textures: [&'a wgpu::TextureView; 16],
}

pub struct Water {
    retained: super::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
    vertices: wgpu::Buffer,
}

impl Water {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        map: [u32; 2],
        samples: u32,
        format: wgpu::TextureFormat,
        depth: wgpu::TextureFormat,
        technique: &str,
    ) -> io::Result<Self> {
        let selection = effect
            .select(technique)
            .map_err(refuse)?
            .ok_or_else(|| refuse(format!("{technique} absent; retained program unavailable")))?;
        let compiled = effect.compiled(selection.index).map_err(refuse)?;
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
        // 14126639E binds at most16 physical PS texture/sampler slots.
        state
            .samplers
            .retain(|slot, _| (0..16).any(|i| *slot == binding(i, Stage::Pixel)));
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let attributes = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                if !matches!(input.semantic.as_str(), "POSITION" | "POSITION0")
                    || input.kind != ScalarKind::Sint
                    || input.components > 2
                {
                    return Err(refuse(format!(
                        "native water stream cannot supply {input:?}"
                    )));
                }
                Ok(wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Sint16x2,
                    offset: 0,
                    shader_location: input.location,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        pass.vertices = vec![gpu::VertexLayout {
            stride: 4,
            step: wgpu::VertexStepMode::Vertex,
            attributes,
        }];
        // 141268F22 restores renderer+C0 before water. Depth is retained renderer+90:
        // rivers/railways restore it, trees do not replace it. Defaults1423F7140/7180.
        let raster = RasterizerState::default();
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleStrip).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        super::shader_constants::scene(&pipeline).map_err(refuse)?;
        super::shader_constants::both(&pipeline, 3, 16, std::slice::from_ref(&(0..12)))
            .map_err(refuse)?;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native water strip"),
            contents: bytemuck::cast_slice(&quad(map)),
            usage: wgpu::BufferUsages::VERTEX,
        });
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
            vertices,
        })
    }

    pub fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: super::border_shader::Target<'_>,
        inputs: Inputs<'_>,
    ) -> io::Result<()> {
        inputs
            .retained_pixel_b2
            .check(&self.pipeline)
            .map_err(refuse)?;
        let textures = inputs
            .textures
            .iter()
            .enumerate()
            .map(|(i, v)| (binding(i as u32, Stage::Pixel), (*v).clone()))
            .collect();
        // 141266229..244 clears all112bytes of b1. b3 alloc16 at141267CCF;
        // 14126645B..4AF initialize only12: time,0.5/width,0.5/height. Last4 unknown.
        let b3 = bytemuck::cast_slice(&[
            inputs.time,
            0.5 / inputs.size[0] as f32,
            0.5 / inputs.size[1] as f32,
            0.0,
        ])
        .to_vec();
        let mut bound = Vec::new();
        for frame in inputs.frames {
            let buffers = BTreeMap::from([
                (binding(0, Stage::Vertex), frame.to_vec()),
                (binding(0, Stage::Pixel), frame.to_vec()),
                (binding(1, Stage::Vertex), vec![0; 112]),
                (binding(1, Stage::Pixel), vec![0; 112]),
                (
                    binding(2, Stage::Pixel),
                    inputs.retained_pixel_b2.bytes().to_vec(),
                ),
                (binding(3, Stage::Vertex), b3.clone()),
                (binding(3, Stage::Pixel), b3.clone()),
            ]);
            if let Some(retained) = inputs.retained {
                let mut tracked = retained.borrow_mut();
                self.retained.bind(&mut tracked, &buffers, &textures);
                inputs.retained_pixel_b2.retain(&mut tracked);
                for stage in [Stage::Vertex, Stage::Pixel] {
                    tracked.stream.bind_partial_constants(
                        3,
                        stage,
                        Some(&buffers[&binding(3, stage)]),
                        std::slice::from_ref(&(0..12)),
                    );
                }
            }
            bound.push(
                self.pipeline
                    .bindings(arena, &buffers, &textures)
                    .map_err(refuse)?,
            );
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compiled water"),
            color_attachments: &[Some(target.color)],
            depth_stencil_attachment: Some(target.depth),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for bindings in &bound {
            bindings.bind(&mut pass);
            pass.draw(0..4, 0..1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_quality_precedes_refraction_setting() {
        assert_eq!(technique(false, false), "water_low_gfx");
        assert_eq!(technique(false, true), "water_low_gfx");
        assert_eq!(technique(true, false), "water_no_refractions");
        assert_eq!(technique(true, true), "water");
    }
    #[test]
    fn native_strip_is_four_packed_signed_words_in_submission_order() {
        assert_eq!(quad([421, 313]), [[421, 0], [0, 0], [421, 313], [0, 313]]);
        assert_eq!(
            quad([0x8000, 0xffff]),
            [[i16::MIN, 0], [0, 0], [i16::MIN, -1], [0, -1]]
        );
    }
}
