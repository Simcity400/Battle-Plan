//! Compiled content pdxmap passes with native vertex streams and constant bindings. The uniform
//! terrain LOD is an approved fidelity limit.
use eframe::wgpu::{self, util::DeviceExt};
use installed_content::{
    ResolvedLayers,
    image::native::Decode,
    pdxshader::{DepthStencilState, RasterizerState, program::Stage},
};
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::{collections::BTreeMap, io, sync::Arc};

/// The uniform grid one frame draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// World `(x, z)` of the first vertex, aligned to a multiple of the step so
    /// the sampled heightmap texels do not shimmer as the camera moves.
    pub origin: [f32; 2],
    /// Spacing in map pixels; always a power of two.
    pub step: f32,
    /// Vertex columns and rows.
    pub counts: [u32; 2],
}

impl Grid {
    /// Quad count before block alignment in the native stream adapter.
    pub fn quads(&self) -> u32 {
        self.counts[0].saturating_sub(1) * self.counts[1].saturating_sub(1)
    }
}

// Allocate 65*65 vertices and 64*64*6 indices per block.
const SIDE: u32 = 64;
fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader terrain: 1 refused pass: {}",
        reason.into()
    ))
}
pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = super::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/pdxmap.shader", &defines)
        .map_err(refuse)
}

/// Position stream has signed shorts, including the edge and snap direction.
fn positions() -> Vec<[i16; 4]> {
    (0..=SIDE)
        .flat_map(|z| {
            (0..=SIDE).map(move |x| {
                let (edge, direction) = if (x + z) % 2 == 0 {
                    (0, 0)
                } else if z == 0 {
                    (1, 0)
                } else if z == SIDE {
                    (2, 0)
                } else if x == 0 {
                    (3, 1)
                } else if x == SIDE {
                    (4, 1)
                } else {
                    (0, 0)
                };
                [x as i16, z as i16, edge, direction]
            })
        })
        .collect()
}
fn indices(cells: [u32; 2]) -> Vec<u16> {
    (0..cells[1])
        .flat_map(|z| {
            (0..cells[0]).flat_map(move |x| {
                let a = (z * (SIDE + 1) + x) as u16;
                let b = a + 1;
                let c = a + (SIDE + 1) as u16;
                let d = c + 1;
                if (x + z) % 2 == 0 {
                    [a, b, d, a, d, c]
                } else {
                    [a, b, c, b, d, c]
                }
            })
        })
        .collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Patch {
    origin: [u32; 2],
    step: u32,
}
fn heights(bytes: &[u8], map: [u32; 2], patch: Patch) -> Vec<[i16; 2]> {
    let sample = |x: u32, z: u32| {
        let x = x % map[0];
        let z = z.min(map[1] - 1);
        // Multiply 0.1 by 100 before truncating to signed 16-bit. Decoded bitmap rows are top-down;
        // height samples for the terrain vertex stream are bottom-up.
        i16::from(bytes[((map[1] - 1 - z) * map[0] + x) as usize]) * 10
    };
    positions()
        .into_iter()
        .map(|[x, z, edge, direction]| {
            let world = [
                patch.origin[0] + x as u32 * patch.step,
                patch.origin[1] + z as u32 * patch.step,
            ];
            let first = sample(world[0], world[1]);
            let alternate = if edge == 0 {
                first
            } else {
                sample(
                    world[0] + u32::from(direction == 0) * patch.step,
                    world[1] + u32::from(direction == 1) * patch.step,
                )
            };
            [first, alternate]
        })
        .collect()
}

fn validate_grid(grid: Grid, map: [u32; 2]) -> io::Result<()> {
    if grid.quads() == 0 {
        return Ok(());
    }
    let step = grid.step as u32;
    if step == 0
        || grid.step != step as f32
        || !step.is_power_of_two()
        || map.iter().any(|n| !n.is_multiple_of(step))
    {
        return Err(refuse(
            "uniform grid step does not divide the content map dimensions",
        ));
    }
    Ok(())
}
/// Select uniform blocks over the approved camera grid, separately for each wrap.
fn patches(grid: Grid, map: [u32; 2], offset: f32) -> io::Result<Vec<Patch>> {
    if grid.quads() == 0 {
        return Ok(Vec::new());
    }
    validate_grid(grid, map)?;
    let step = grid.step as u32;
    let pitch = SIDE * step;
    let start = [(grid.origin[0] - offset).max(0.), grid.origin[1].max(0.)];
    let end = [
        (grid.origin[0] + grid.counts[0].saturating_sub(1) as f32 * grid.step - offset)
            .min(map[0] as f32),
        (grid.origin[1] + grid.counts[1].saturating_sub(1) as f32 * grid.step).min(map[1] as f32),
    ];
    if end[0] <= start[0] || end[1] <= start[1] {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for z in (start[1] as u32 / pitch * pitch..end[1].ceil() as u32).step_by(pitch as usize) {
        for x in (start[0] as u32 / pitch * pitch..end[0].ceil() as u32).step_by(pitch as usize) {
            result.push(Patch {
                origin: [x, z],
                step,
            });
        }
    }
    Ok(result)
}
fn constants(patch: Patch, matrix: [[f32; 4]; 4]) -> Vec<u8> {
    // All 112 bytes are initialized to zero. Uniform LOD has no snaps.
    let mut data = vec![0; 112];
    data[..64].copy_from_slice(bytemuck::bytes_of(&matrix));
    for (offset, value) in [
        (64, patch.origin[0] as f32),
        (68, patch.origin[1] as f32),
        (72, patch.step as f32),
        (100, 0.1),
        (104, patch.step as f32),
    ] {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    data
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Receiver,
    Underwater,
    Lit(bool),
}
impl Mode {
    pub fn technique(self) -> &'static str {
        match self {
            Self::Receiver => "terrainunlit",
            Self::Underwater => "underwater",
            Self::Lit(true) => "terrain",
            Self::Lit(false) => "terrain_low_gfx",
        }
    }
}
struct Geometry {
    cpu_heights: Arc<[u8]>,
    heights: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}
pub struct Terrain {
    pub effect: Arc<Effect>,
    pipelines: BTreeMap<Mode, Result<Arc<gpu::Pipeline>, String>>,
    programs: BTreeMap<Mode, super::shader_scene::Program>,
    positions: wgpu::Buffer,
    bytes: Vec<u8>,
    map: [u32; 2],
    geometry: BTreeMap<Patch, Geometry>,
    samples: u32,
}
pub struct Inputs<'a> {
    pub retained: Option<&'a std::cell::RefCell<super::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub offsets: &'a [f32],
    pub grid: Grid,
    pub matrix: [[f32; 4]; 4],
    pub light_grid: [f32; 4],
    pub textures: [&'a wgpu::TextureView; 16],
}
pub struct Prepared {
    pub pipeline: Arc<gpu::Pipeline>,
    batches: Vec<(Patch, gpu::Bindings)>,
}
impl Terrain {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        bytes: &[u8],
        map: [u32; 2],
        samples: u32,
    ) -> io::Result<Self> {
        // Native loader explicitly errors on dimensions not divisible by64.
        if map.contains(&0)
            || map.iter().any(|n| !n.is_multiple_of(SIDE))
            || u64::from(map[0]) * u64::from(map[1]) != bytes.len() as u64
        {
            return Err(refuse(
                "height bitmap must match nonzero map dimensions divisible by64",
            ));
        }
        let positions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native terrain positions"),
            contents: bytemuck::cast_slice(&positions()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        Ok(Self {
            effect: effect.clone(),
            pipelines: BTreeMap::new(),
            programs: BTreeMap::new(),
            positions,
            bytes: bytes.to_vec(),
            map,
            geometry: BTreeMap::new(),
            samples,
        })
    }
    fn pipeline(&mut self, device: &wgpu::Device, mode: Mode) -> io::Result<Arc<gpu::Pipeline>> {
        if !self.pipelines.contains_key(&mode) {
            let compiled = self
                .build_pipeline(device, mode)
                .map_err(|error| error.to_string());
            self.pipelines.insert(mode, compiled);
        }
        self.pipelines[&mode]
            .as_ref()
            .cloned()
            .map_err(|reason| io::Error::other(reason.clone()))
    }
    fn build_pipeline(
        &mut self,
        device: &wgpu::Device,
        mode: Mode,
    ) -> io::Result<Arc<gpu::Pipeline>> {
        let technique = mode.technique();
        let selection = self
            .effect
            .select(technique)
            .map_err(refuse)?
            .ok_or_else(|| refuse(format!("{technique} absent; retained program unavailable")))?;
        let compiled = self.effect.compiled(selection.index).map_err(refuse)?;
        let mut state = gpu::State::authored(
            &self.effect.source,
            &self.effect.source.techniques[selection.index],
        )
        .map_err(refuse)?;
        state
            .samplers
            .retain(|key, _| (0..16).any(|slot| *key == binding(slot, Stage::Pixel)));
        let (format, depth, samples) = match mode {
            Mode::Receiver => (
                super::shadow::BUFFER_FORMAT,
                super::shadow_blur::DEPTH_FORMAT,
                1,
            ),
            Mode::Underwater => (
                super::water::REFRACTION_FORMAT,
                super::scene::DEPTH_FORMAT,
                1,
            ),
            Mode::Lit(_) => (
                super::scene::SCENE_FORMAT,
                super::scene::DEPTH_FORMAT,
                self.samples,
            ),
        };
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let mut attributes: [Vec<wgpu::VertexAttribute>; 2] = Default::default();
        // Declaration: signed16x4 POSITION stream0, signed16x2 TEXCOORD0 stream1.
        for input in &compiled.vertex.inputs {
            let (slot, format, width) = match input.semantic.as_str() {
                "POSITION" | "POSITION0" => (0, wgpu::VertexFormat::Sint16x4, 4),
                "TEXCOORD" | "TEXCOORD0" => (1, wgpu::VertexFormat::Sint16x2, 2),
                _ => {
                    return Err(refuse(format!(
                        "native terrain streams cannot supply {input:?}"
                    )));
                }
            };
            if input.kind != ScalarKind::Sint || input.components > width {
                return Err(refuse(format!(
                    "native terrain streams cannot supply {input:?}"
                )));
            }
            attributes[slot].push(wgpu::VertexAttribute {
                format,
                offset: 0,
                shader_location: input.location,
            });
        }
        pass.vertices = attributes
            .into_iter()
            .zip([8, 4])
            .map(|(attributes, stride)| gpu::VertexLayout {
                stride,
                step: wgpu::VertexStepMode::Vertex,
                attributes,
            })
            .collect();
        // Terrain uses counterclockwise fronts. Underwater and ordinary renderer depth states both
        // enable writes with LESS_EQUAL.
        let raster = RasterizerState {
            front_ccw: true,
            ..Default::default()
        };
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        super::shader_constants::scene(&pipeline).map_err(refuse)?;
        self.programs.insert(
            mode,
            super::shader_scene::Program::new(
                &selection,
                &pass.vertices,
                &state,
                &raster,
                &DepthStencilState::default(),
            ),
        );
        Ok(pipeline)
    }
    // Grid/content refusals remain fatal, before preflight shader isolation.
    pub fn validate_grid(&self, grid: Grid) -> io::Result<()> {
        validate_grid(grid, self.map)
    }
    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        mode: Mode,
        inputs: Inputs<'_>,
    ) -> io::Result<Prepared> {
        self.validate_grid(inputs.grid)?;
        let device = arena.device();
        let pipeline = self.pipeline(device, mode)?;
        let textures = inputs
            .textures
            .iter()
            .enumerate()
            .map(|(i, v)| (binding(i as u32, Stage::Pixel), (*v).clone()))
            .collect();
        let mut batches = Vec::new();
        for (frame, offset) in inputs.frames.iter().zip(inputs.offsets) {
            let mut matrix = inputs.matrix;
            for (value, axis) in matrix[3].iter_mut().zip(inputs.matrix[0]) {
                *value += axis * offset;
            }
            for patch in patches(inputs.grid, self.map, *offset)? {
                self.geometry.entry(patch).or_insert_with(|| {
                    let cells = std::array::from_fn(|axis| {
                        ((self.map[axis] - patch.origin[axis]) / patch.step).min(SIDE)
                    });
                    let indices = indices(cells);
                    let cpu_heights: Arc<[u8]> =
                        bytemuck::cast_slice(&heights(&self.bytes, self.map, patch)).into();
                    Geometry {
                        heights: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("native terrain heights"),
                            contents: &cpu_heights,
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("native terrain indices"),
                            contents: bytemuck::cast_slice(&indices),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                        cpu_heights,
                        count: indices.len() as u32,
                    }
                });
                let b1 = constants(patch, matrix);
                let mut buffers = BTreeMap::from([
                    (binding(0, Stage::Vertex), frame.to_vec()),
                    (binding(0, Stage::Pixel), frame.to_vec()),
                    (binding(1, Stage::Vertex), b1.clone()),
                    (binding(1, Stage::Pixel), b1),
                ]);
                if mode != Mode::Receiver {
                    buffers.insert(
                        binding(2, Stage::Pixel),
                        bytemuck::bytes_of(&inputs.light_grid).to_vec(),
                    );
                }
                if let Some(retained) = inputs.retained {
                    let mut tracked = retained.borrow_mut();
                    self.programs[&mode].bind(&mut tracked, &buffers, &textures);
                    tracked.bind_vertex(1, 4, self.geometry[&patch].cpu_heights.clone());
                }
                batches.push((
                    patch,
                    pipeline
                        .bindings(arena, &buffers, &textures)
                        .map_err(refuse)?,
                ));
            }
        }
        Ok(Prepared { pipeline, batches })
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, prepared: &Prepared) {
        pass.set_pipeline(&prepared.pipeline.pipeline);
        pass.set_blend_constant(wgpu::Color::WHITE);
        pass.set_vertex_buffer(0, self.positions.slice(..));
        for (key, bindings) in &prepared.batches {
            let geometry = &self.geometry[key];
            bindings.bind(pass);
            pass.set_vertex_buffer(1, geometry.heights.slice(..));
            pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..geometry.count, 0, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_checkerboard_and_snap_records() {
        let p = positions();
        assert_eq!(p.len(), 4225);
        assert_eq!(p[1], [1, 0, 1, 0]);
        assert_eq!(p[65], [0, 1, 3, 1]);
        assert_eq!(p[129], [64, 1, 4, 1]);
        assert_eq!(p[4161], [1, 64, 2, 0]);
        assert_eq!(p[0], [0, 0, 0, 0]);
        assert_eq!(
            &indices([2, 2]),
            &[
                0, 1, 66, 0, 66, 65, 1, 2, 66, 2, 67, 66, 65, 66, 130, 66, 131, 130, 66, 67, 132,
                66, 132, 131
            ]
        );
    }
    #[test]
    fn heights_preserve_bytes_wrap_clamp_and_alternate_sampling() {
        let bytes: Vec<_> = (0..128 * 64)
            .map(|i| ((i % 128 + i / 128 * 3) % 256) as u8)
            .collect();
        let h = heights(
            &bytes,
            [128, 64],
            Patch {
                origin: [64, 0],
                step: 1,
            },
        );
        assert_eq!(h[0], [2530, 2530]);
        assert_eq!(h[1], [2540, 2550]);
        assert_eq!(h[129], [1860, 1830]);
        assert_eq!(h[4224], [0, 0]);
    }
    #[test]
    fn wrap_selection_and_native_buffer_bytes() {
        let g = Grid {
            origin: [-10., 3.],
            step: 1.,
            counts: [31, 81],
        };
        assert_eq!(
            patches(g, [128, 128], 0.).unwrap(),
            vec![
                Patch {
                    origin: [0, 0],
                    step: 1
                },
                Patch {
                    origin: [0, 64],
                    step: 1
                }
            ]
        );
        assert_eq!(
            patches(g, [128, 128], -128.).unwrap(),
            vec![
                Patch {
                    origin: [64, 0],
                    step: 1
                },
                Patch {
                    origin: [64, 64],
                    step: 1
                }
            ]
        );
        assert!(patches(g, [128, 128], 128.).unwrap().is_empty());
        let p = Patch {
            origin: [64, 128],
            step: 4,
        };
        let data = constants(p, [[0.; 4]; 4]);
        let f: &[f32] = bytemuck::cast_slice(&data);
        assert_eq!(
            &f[16..],
            &[64., 128., 4., 0., 0., 0., 0., 0., 0., 0.1, 4., 0.]
        );
    }
}
