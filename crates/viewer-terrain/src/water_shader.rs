use viewer_gpu::gpu_contract::{SeaBed, Targets};
use eframe::wgpu::{self, util::DeviceExt};
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};

fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader water: 1 refused pass: {}",
        reason.into()
    ))
}

pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/pdxwater.shader", &defines)
        .map_err(refuse)
}

pub fn reads_snow_mud(compiled: &shader_pipeline::Compiled) -> bool {
    compiled
        .pixel
        .textures
        .iter()
        .any(|&(slot, _)| slot == binding(SNOW_MUD_SLOT, Stage::Pixel))
}

pub fn sea_bed(compiled: &shader_pipeline::Compiled) -> (SeaBed, Option<String>) {
    let Some(why) = shader_pipeline::texel::by_texel(
        &compiled.pixel.wgsl,
        binding(REFRACTION_SLOT, Stage::Pixel),
        "refraction texture",
    ) else {
        return (SeaBed::Half, None);
    };
    (
        SeaBed::Full,
        Some(format!("half-size sea bed not used: the water shader {why}")),
    )
}

pub fn technique(high: bool, refractions: bool) -> &'static str {
    if !high {
        "water_low_gfx"
    } else if refractions {
        "water"
    } else {
        "water_no_refractions"
    }
}

fn half_pixel(output: [u32; 2]) -> [f32; 2] {
    output.map(|side| 0.5 / side as f32)
}

fn quad(map: [u32; 2]) -> [[i16; 2]; 4] {
    let [w, h] = map.map(|v| v as i16);
    [[w, 0], [0, 0], [w, h], [0, h]]
}

pub struct Inputs<'a> {
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub time: f32,
    pub size: [u32; 2],
    pub retained_pixel_b2: viewer_scene_base::shader_constants::WorldPixelConstants,
    pub textures: [&'a wgpu::TextureView; 16],
}

pub const SNOW_MUD_SLOT: u32 = 9;
pub const REFRACTION_SLOT: u32 = 5;
pub const SHADOW_SLOT: u32 = 15;

pub struct Water {
    retained: viewer_scene_base::shader_scene::Program,
    pipeline: Arc<gpu::Pipeline>,
    vertices: wgpu::Buffer,
    pub reads_snow_mud: bool,
    pub sea_bed: (SeaBed, Option<String>),
    pub texel_reads: Option<String>,
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
        shader_pipeline::library::with_recorded(effect, |effect| {
            Self::build(device, effect, map, samples, format, depth, technique)
        })
    }

    fn build(
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
        let reads_snow_mud = reads_snow_mud(&compiled);
        let sea_bed = sea_bed(&compiled);
        let texel_reads = viewer_scene_base::shadow::texel_reads(&compiled, SHADOW_SLOT, technique);
        let mut state =
            gpu::State::authored(&effect.source, &effect.source.techniques[selection.index])
                .map_err(refuse)?;
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
        let raster = RasterizerState::default();
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleStrip).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        viewer_scene_base::shader_constants::both(&pipeline, 3, 16, std::slice::from_ref(&(0..12)))
            .map_err(refuse)?;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native water strip"),
            contents: bytemuck::cast_slice(&quad(map)),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let retained = viewer_scene_base::shader_scene::Program::new(
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
            reads_snow_mud,
            sea_bed,
            texel_reads,
        })
    }

    pub fn draw(
        &self,
        arena: &gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
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
        let [x, y] = half_pixel(inputs.size);
        let b3 = bytemuck::cast_slice(&[inputs.time, x, y, 0.0]).to_vec();
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
        let mut pass = viewer_scene_base::shader_scene::world_pass(encoder, targets, "compiled water");
        let pass = &mut pass;
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&self.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        let mut previous = None;
        for bindings in &bound {
            bindings.bind_after(pass, previous);
            previous = Some(bindings);
            shader_pipeline::gpu::stats::draw();
            pass.draw(0..4, 0..1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_strip_is_four_packed_signed_words_in_submission_order() {
        assert_eq!(quad([421, 313]), [[421, 0], [0, 0], [421, 313], [0, 313]]);
        assert_eq!(
            quad([0x8000, 0xffff]),
            [[i16::MIN, 0], [0, 0], [i16::MIN, -1], [0, -1]]
        );
    }

    #[test]
    fn water_reading_the_refraction_texture_by_texel_keeps_the_full_size_sea_bed() {
        let refraction = binding(REFRACTION_SLOT, Stage::Pixel);
        let water = |body: &str| {
            let wgsl = format!(
                "@group(1) @binding({refraction}) var refraction: texture_2d<f32>;
                 @group(2) @binding({refraction}) var linear: sampler;
                 @fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
                     {body}
                 }}"
            );
            let stage = shader_pipeline::Shader {
                wgsl,
                independent_draws: false,
                buffers: Vec::new(),
                textures: vec![(refraction, shader_pipeline::TextureKind::D2)],
                samplers: vec![refraction],
                sampling_pairs: vec![(refraction, refraction)],
                inputs: Vec::new(),
                clock: Default::default(),
            };
            sea_bed(&shader_pipeline::Compiled {
                vertex: shader_pipeline::Shader {
                    wgsl: String::new(),
                    textures: Vec::new(),
                    samplers: Vec::new(),
                    sampling_pairs: Vec::new(),
                    ..stage.clone()
                },
                pixel: stage,
            })
        };
        assert_eq!(water("return textureSample(refraction, linear, uv);"), (SeaBed::Half, None));
        let (size, why) = water("return textureSample(refraction, linear, uv, vec2<i32>(1, 0));");
        assert_eq!(size, SeaBed::Full);
        assert_eq!(
            why.as_deref(),
            Some(
                "half-size sea bed not used: the water shader reads the refraction texture by texel: it samples it at texel offsets"
            )
        );
        let (size, why) = water(
            "let texel = 1.0 / vec2<f32>(textureDimensions(refraction));
             return textureSample(refraction, linear, uv + texel);",
        );
        assert_eq!(size, SeaBed::Full);
        assert!(why.unwrap().ends_with("by texel: it reads its own size"));
    }

    #[test]
    fn only_a_technique_that_reads_the_snow_mud_slot_shows_snow_on_water() {
        let stage = |slots: &[u32]| shader_pipeline::Shader {
            wgsl: String::new(),
            independent_draws: false,
            buffers: Vec::new(),
            textures: slots
                .iter()
                .map(|&slot| (binding(slot, Stage::Pixel), shader_pipeline::TextureKind::D2))
                .collect(),
            samplers: Vec::new(),
            sampling_pairs: Vec::new(),
            inputs: Vec::new(),
            clock: Default::default(),
        };
        let compiled = |pixel: &[u32]| shader_pipeline::Compiled {
            vertex: stage(&[SNOW_MUD_SLOT]),
            pixel: stage(pixel),
        };
        assert!(reads_snow_mud(&compiled(&[0, 5, SNOW_MUD_SLOT])));
        assert!(!reads_snow_mud(&compiled(&[0, 5, 8])));
    }
}
