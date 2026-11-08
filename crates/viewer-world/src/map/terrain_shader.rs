//! Compiled content pdxmap passes with native vertex streams and constant bindings. The uniform
//! terrain LOD is an approved fidelity limit.
use eframe::wgpu::{self, util::DeviceExt};
use installed_content_core::{
    ResolvedLayers, pdxshader::DepthStencilState, pdxshader::RasterizerState,
    pdxshader::program::Stage,
};
use installed_content_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::cell::RefCell;
use std::{collections::BTreeMap, io, sync::Arc};
use viewer_foundation::map::gpu_contract::DEPTH_FORMAT;
use viewer_foundation::map::gpu_contract::REFRACTION_FORMAT;
use viewer_foundation::map::gpu_contract::SCENE_FORMAT;

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

/// What one frame's terrain draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Cover {
    /// The approved uniform grid, cut into blocks for every wrap view; the game's camera draws it.
    Uniform(Grid),
    /// Blocks chosen for a free eye, each on its own map copy, with the edges that meet a coarser
    /// neighbour snapped to it ([`super::terrain::blocks`]).
    Blocks(Vec<Block>),
    /// A free eye over a map the shader's blocks cannot tile, refused by name.
    Refused(&'static str),
}

impl Cover {
    /// The uniform grid, or no grid for a free eye's blocks.
    pub fn grid(&self) -> Grid {
        match self {
            Self::Uniform(grid) => *grid,
            Self::Blocks(_) | Self::Refused(_) => Grid {
                origin: [0.0, 0.0],
                step: 1.0,
                counts: [0, 0],
            },
        }
    }

    /// Quads the cover draws, before clipping to the map.
    pub fn quads(&self) -> u64 {
        match self {
            Self::Uniform(grid) => u64::from(grid.quads()),
            Self::Blocks(blocks) => blocks.len() as u64 * u64::from(SIDE * SIDE),
            Self::Refused(_) => 0,
        }
    }
}

/// One terrain block a free eye draws: 64 by 64 cells at `step` from `origin` in map pixels (cut
/// at the map's edges), drawn `offset` world units from the map's first copy. `snap` lists its
/// south, north, west and east edges in the order of the content shader's snap switches; a set
/// edge meets a neighbour of twice the step, and its odd vertices move onto that neighbour's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub origin: [u32; 2],
    pub step: u32,
    pub offset: f32,
    pub snap: [bool; 4],
}

/// Cells along a block's side: 65*65 vertices and 64*64*6 indices per block.
pub const SIDE: u32 = 64;
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
pub(crate) fn indices(cells: [u32; 2]) -> Vec<u16> {
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
/// A free eye's block must start on its own step's block grid inside the map, with a step that
/// divides the map.
fn validate_block(patch: Patch, map: [u32; 2]) -> io::Result<()> {
    let step = patch.step;
    if step == 0
        || !step.is_power_of_two()
        || map.iter().any(|n| !n.is_multiple_of(step))
        || patch
            .origin
            .iter()
            .any(|origin| !origin.is_multiple_of(SIDE * step))
        || patch.origin[0] >= map[0]
        || patch.origin[1] >= map[1]
    {
        return Err(refuse(
            "a terrain block is not on its step's block grid inside the map",
        ));
    }
    Ok(())
}

/// The blocks the terrain draws for one wrap view of a `map`-sized map: the uniform grid's for
/// that offset, unsnapped, or the free eye's blocks standing on that copy.
pub fn drawn_blocks(cover: &Cover, map: [u32; 2], offset: f32) -> io::Result<Vec<Block>> {
    match cover {
        Cover::Uniform(grid) => Ok(patches(*grid, map, offset)?
            .into_iter()
            .map(|patch| Block {
                origin: patch.origin,
                step: patch.step,
                offset,
                snap: [false; 4],
            })
            .collect()),
        Cover::Blocks(blocks) => blocks
            .iter()
            .filter(|block| block.offset == offset)
            .map(|block| {
                let patch = Patch {
                    origin: block.origin,
                    step: block.step,
                };
                validate_block(patch, map)?;
                Ok(*block)
            })
            .collect(),
        Cover::Refused(reason) => Err(refuse(*reason)),
    }
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
fn constants(patch: Patch, matrix: [[f32; 4]; 4], snap: [bool; 4]) -> Vec<u8> {
    // All 112 bytes are initialized to zero. Uniform LOD has no snaps; a free eye's blocks set
    // the snap switch of each edge that meets a coarser block.
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
    for (edge, snapped) in snap.into_iter().enumerate() {
        if snapped {
            let offset = SNAP_SWITCHES + 4 * edge;
            data[offset..offset + 4].copy_from_slice(&1.0f32.to_le_bytes());
        }
    }
    data
}

/// The byte offset of the four edge snap switches in the block constants.
const SNAP_SWITCHES: usize = 80;
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
    pub retained: Option<&'a RefCell<super::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub offsets: &'a [f32],
    pub cover: &'a Cover,
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
    /// Builds the pipelines of `modes` now, keeping each result for the frames that draw them.
    pub fn warm(&mut self, device: &wgpu::Device, modes: &[Mode]) {
        let effect = Arc::clone(&self.effect);
        for &mode in modes {
            drop(self.pipeline(device, mode));
        }
        // A mode that fell back to the recorded effect cleared the others: they warm again.
        if !Arc::ptr_eq(&effect, &self.effect) {
            for &mode in modes {
                drop(self.pipeline(device, mode));
            }
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, mode: Mode) -> io::Result<Arc<gpu::Pipeline>> {
        if !self.pipelines.contains_key(&mode) {
            let mut built = self.build_pipeline(device, mode);
            // A visual mod's replacement that compiles but cannot be drawn here falls back to
            // the recorded effect for every mode, the replacement refused by name.
            if let Err(error) = &built
                && let Some(recorded) = self.effect.fall_back(&error.to_string())
            {
                self.effect = recorded;
                self.pipelines.clear();
                self.programs.clear();
                built = self.build_pipeline(device, mode);
            }
            self.pipelines
                .insert(mode, built.map_err(|error| error.to_string()));
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
            Mode::Underwater => (REFRACTION_FORMAT, DEPTH_FORMAT, 1),
            Mode::Lit(_) => (SCENE_FORMAT, DEPTH_FORMAT, self.samples),
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

    /// The map's width and height in pixels, which the terrain's blocks tile.
    pub fn map(&self) -> [u32; 2] {
        self.map
    }

    /// The cover's blocks for one wrap view: the uniform grid's for that offset, or the free
    /// eye's blocks standing on that copy.
    fn view_blocks(&self, cover: &Cover, offset: f32) -> io::Result<Vec<(Patch, [bool; 4])>> {
        Ok(drawn_blocks(cover, self.map, offset)?
            .into_iter()
            .map(|block| {
                let patch = Patch {
                    origin: block.origin,
                    step: block.step,
                };
                (patch, block.snap)
            })
            .collect())
    }
    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        mode: Mode,
        inputs: Inputs<'_>,
    ) -> io::Result<Prepared> {
        if let Cover::Uniform(grid) = inputs.cover {
            self.validate_grid(*grid)?;
        }
        let device = arena.device();
        let pipeline = self.pipeline(device, mode)?;
        let textures = inputs
            .textures
            .iter()
            .enumerate()
            .map(|(i, v)| (binding(i as u32, Stage::Pixel), (*v).clone()))
            .collect();
        let mut batches = Vec::new();
        let mut buffers = BTreeMap::new();
        if mode != Mode::Receiver {
            buffers.insert(
                binding(2, Stage::Pixel),
                bytemuck::bytes_of(&inputs.light_grid).to_vec(),
            );
        }
        for (frame, offset) in inputs.frames.iter().zip(inputs.offsets) {
            let mut matrix = inputs.matrix;
            for (value, axis) in matrix[3].iter_mut().zip(inputs.matrix[0]) {
                *value += axis * offset;
            }
            for stage in [Stage::Vertex, Stage::Pixel] {
                buffers.insert(binding(0, stage), frame.to_vec());
            }
            let blocks = self.view_blocks(inputs.cover, *offset)?;
            let last = blocks.len().saturating_sub(1);
            for (index, (patch, snap)) in blocks.into_iter().enumerate() {
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
                let b1 = constants(patch, matrix, snap);
                for stage in [Stage::Vertex, Stage::Pixel] {
                    let block = buffers.entry(binding(1, stage)).or_insert_with(Vec::new);
                    block.clear();
                    block.extend_from_slice(&b1);
                }
                // Every block binds the same slots, so the state it leaves behind is the view's last.
                if index == last
                    && let Some(retained) = inputs.retained
                {
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
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&prepared.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, self.positions.slice(..));
        let mut previous = None;
        for (key, bindings) in &prepared.batches {
            let geometry = &self.geometry[key];
            bindings.bind_after(pass, previous);
            previous = Some(bindings);
            shader_pipeline::gpu::stats::state();
            pass.set_vertex_buffer(1, geometry.heights.slice(..));
            shader_pipeline::gpu::stats::state();
            pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint16);
            shader_pipeline::gpu::stats::draw();
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
        let data = constants(p, [[0.; 4]; 4], [false; 4]);
        let f: &[f32] = bytemuck::cast_slice(&data);
        assert_eq!(
            &f[16..],
            &[64., 128., 4., 0., 0., 0., 0., 0., 0., 0.1, 4., 0.]
        );
    }

    /// A free eye's block writes each snapped edge's switch, in the shader's edge order, and
    /// nothing else differs from the uniform grid's constants.
    #[test]
    fn snapped_edges_set_their_switches() {
        let p = Patch {
            origin: [64, 128],
            step: 4,
        };
        let data = constants(p, [[0.; 4]; 4], [true, false, false, true]);
        let f: &[f32] = bytemuck::cast_slice(&data);
        assert_eq!(
            &f[16..],
            &[64., 128., 4., 0., 1., 0., 0., 1., 0., 0.1, 4., 0.]
        );
    }

    #[test]
    fn free_blocks_must_lie_on_their_step_grid_inside_the_map() {
        let map = [256, 128];
        let block = |origin, step| Patch { origin, step };
        assert!(validate_block(block([128, 0], 2), map).is_ok());
        assert!(validate_block(block([64, 0], 2), map).is_err());
        assert!(validate_block(block([256, 0], 1), map).is_err());
        assert!(validate_block(block([0, 0], 3), map).is_err());
        assert!(validate_block(block([0, 0], 256), map).is_err());
    }
}
