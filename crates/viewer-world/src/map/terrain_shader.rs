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
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::map::gpu_contract::DEPTH_FORMAT;
use viewer_foundation::map::pass_key::{Keyed, PassKey};
use viewer_foundation::map::textures::Content;
use viewer_foundation::map::gpu_contract::REFRACTION_FORMAT;
use viewer_foundation::map::gpu_contract::SCENE_FORMAT;
use viewer_foundation::parallel::Workers;

/// What one frame's terrain draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Cover {
    /// The blocks the camera's distance chose, each on its own map copy, with the edges that meet
    /// a coarser neighbour snapped to it ([`super::terrain::blocks`]).
    Blocks(Vec<Block>),
    /// A map the shader's blocks cannot tile, refused by name.
    Refused(&'static str),
}

impl Cover {
    /// Blocks the cover draws.
    pub fn blocks(&self) -> usize {
        match self {
            Self::Blocks(blocks) => blocks.len(),
            Self::Refused(_) => 0,
        }
    }

    /// Quads the cover draws, before clipping to the map.
    pub fn quads(&self) -> u64 {
        match self {
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
    let mut out = Vec::new();
    heights_into(&positions(), bytes, map, patch, &mut out);
    out
}
fn heights_into(
    positions: &[[i16; 4]],
    bytes: &[u8],
    map: [u32; 2],
    patch: Patch,
    out: &mut Vec<[i16; 2]>,
) {
    let sample = |x: u32, z: u32| {
        let x = x % map[0];
        let z = z.min(map[1] - 1);
        // Multiply 0.1 by 100 before truncating to signed 16-bit. Decoded bitmap rows are top-down;
        // height samples for the terrain vertex stream are bottom-up.
        i16::from(bytes[((map[1] - 1 - z) * map[0] + x) as usize]) * 10
    };
    out.clear();
    out.extend(positions.iter().map(|&[x, z, edge, direction]| {
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
    }));
}

/// A block must start on its own step's block grid inside the map, with a step that divides the
/// map.
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

/// The blocks the terrain draws for one wrap view of a `map`-sized map: those standing on that
/// copy.
pub fn drawn_blocks(cover: &Cover, map: [u32; 2], offset: f32) -> io::Result<Vec<Block>> {
    match cover {
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

/// A block's constants ([`BLOCK_BYTES`]) written in place.
fn constants_into(data: &mut [u8], patch: Patch, matrix: [[f32; 4]; 4], snap: [bool; 4]) {
    // All 112 bytes are initialized to zero. Uniform LOD has no snaps; a free eye's blocks set
    // the snap switch of each edge that meets a coarser block.
    data[..BLOCK_BYTES].fill(0);
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
}

fn constants(patch: Patch, matrix: [[f32; 4]; 4], snap: [bool; 4]) -> Vec<u8> {
    let mut data = vec![0; BLOCK_BYTES];
    constants_into(&mut data, patch, matrix, snap);
    data
}

/// A block's own constant buffer: the shadow texture matrix every block of a view carries, then
/// the block's origin, step and edge snaps.
const BLOCK_BYTES: usize = 112;

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
/// One patch's height stream: a sample pair for each of the patch's vertex positions.
const PATCH_BYTES: usize = ((SIDE + 1) * (SIDE + 1)) as usize * size_of::<[i16; 2]>();
struct Geometry {
    /// The slab holding this patch's heights, and where in it.
    slab: usize,
    offset: u64,
    indices: wgpu::Buffer,
    count: u32,
}

/// Every patch a uniform grid or a free eye's blocks can draw: each power-of-two step up to the
/// coarsest either reaches, at every origin on that step's lattice within the map.
fn every_patch(map: [u32; 2]) -> impl Iterator<Item = Patch> {
    // The uniform grid stops at 256; a free eye's root block spans the map.
    let widest = map[0]
        .max(map[1])
        .div_ceil(SIDE)
        .next_power_of_two()
        .max(256);
    std::iter::successors(Some(1u32), move |step| (*step < widest).then_some(step * 2)).flat_map(
        move |step| {
            let pitch = (SIDE * step) as usize;
            (0..map[1]).step_by(pitch).flat_map(move |z| {
                (0..map[0]).step_by(pitch).map(move |x| Patch {
                    origin: [x, z],
                    step,
                })
            })
        },
    )
}
/// Patches per fill task: enough to amortise a turn, few enough to share among the builders.
const FILL_GROUP: usize = 64;
/// Writes every patch's heights into its slab's mapped memory, on the calling thread and the
/// builders `workers` lends while they are free.
fn fill_slabs(
    slabs: &[wgpu::Buffer],
    patches: &[Patch],
    bytes: &[u8],
    map: [u32; 2],
    workers: &Workers,
) -> io::Result<()> {
    let mut views = slabs
        .iter()
        .map(|slab| slab.slice(..).get_mapped_range_mut())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| refuse(format!("terrain heights could not be written: {error}")))?;
    // Slabs hold the patches in order, so the slabs' chunks line up with `patches`.
    let mut pairs = patches.iter().copied().zip(
        views
            .iter_mut()
            .flat_map(|view| view.slice(..).into_chunks::<PATCH_BYTES>().0),
    );
    let mut groups = Vec::new();
    loop {
        let group: Vec<_> = pairs.by_ref().take(FILL_GROUP).collect();
        if group.is_empty() {
            break;
        }
        groups.push(Mutex::new(group));
    }
    let positions = positions();
    workers.run(groups.len(), &|index| {
        let group = std::mem::take(&mut *groups[index].lock().unwrap_or_else(|e| e.into_inner()));
        let mut patch_heights = Vec::with_capacity(positions.len());
        for (patch, out) in group {
            heights_into(&positions, bytes, map, patch, &mut patch_heights);
            wgpu::WriteOnly::<[u8]>::from(out)
                .copy_from_slice(bytemuck::cast_slice(&patch_heights));
        }
    });
    Ok(())
}
pub struct Terrain {
    pub effect: Arc<Effect>,
    pipelines: BTreeMap<Mode, Result<Arc<gpu::Pipeline>, String>>,
    programs: BTreeMap<Mode, super::shader_scene::Program>,
    positions: wgpu::Buffer,
    bytes: Vec<u8>,
    map: [u32; 2],
    /// Every patch's heights, packed into as few buffers as the device allows.
    slabs: Vec<wgpu::Buffer>,
    /// Made at load for [`every_patch`], so no frame builds one.
    geometry: BTreeMap<Patch, Geometry>,
    /// Each wrap view's last patch and its height stream, which that view's next draws inherit,
    /// by the view's offset bits.
    retained_heights: Vec<(u32, Patch, Arc<[u8]>)>,
    /// Each wrap view's blocks, by the view's offset bits, for the cover they were cut from.
    /// The cover is the camera's, so a frame that neither moves the camera nor resizes the view
    /// cuts no blocks at all, and the three stages of one frame cut each view's once.
    blocks: Vec<(u32, Arc<[(Patch, [bool; 4])]>)>,
    cut_from: Option<Cover>,
    samples: u32,
    /// Each mode's retained draws, one entry per set of views a frame prepares it for.
    retained: Vec<Retained>,
    /// Each mode whose pass is drawn only while something it reads changes: the key its target
    /// holds ([`Terrain::prepare_kept`]).
    kept: BTreeMap<Mode, Keyed>,
}

/// Each view's blocks and the matrix every block of that view carries.
type Views = Vec<(Arc<[(Patch, [bool; 4])]>, [[f32; 4]; 4])>;

/// Frames a mode's draw list must stand unchanged before it is recorded into a render bundle: a
/// camera that moves cuts new blocks nearly every frame, and recording again would cost more
/// than the commands it saves.
const STILL: u32 = 8;

/// Everything a mode's retained draws were bound from. Any of it differing binds them again;
/// the constant blocks' bytes are written every frame instead.
#[derive(PartialEq)]
struct Built {
    mode: Mode,
    samples: u32,
    /// Each view's scene block in bytes, whose place is reserved for it.
    scene: usize,
    offsets: Vec<u32>,
    cover: Cover,
    textures: Vec<wgpu::TextureView>,
}

/// One mode's retained draws over the views it was prepared for: a persistent constants buffer
/// with a place for every block, the bindings each draw was bound through once, and the render
/// bundle the draws were recorded into once they stood still.
struct Retained {
    built: Built,
    pipeline: Arc<gpu::Pipeline>,
    buffer: wgpu::Buffer,
    /// The buffer's bytes as the GPU holds them.
    bytes: Vec<u8>,
    /// Each view's scene block, and the light grid once when the mode reads one.
    frames: Vec<usize>,
    grid: Option<usize>,
    draws: Vec<Keep>,
    /// Every patch's own block, once for each binding read per instance: its view, what it
    /// carries and where it stands.
    places: Vec<Placed>,
    /// Each view's blocks as one byte range, written together when that view's matrix moves.
    view_blocks: Vec<std::ops::Range<usize>>,
    /// The matrix every block of each view carries.
    matrices: Vec<[[f32; 4]; 4]>,
    /// The draws, pipeline, bind group and state sets the bundle holds, counted where it runs.
    bundle: Option<(wgpu::RenderBundle, [u64; 4])>,
    still: u32,
    used: u64,
}

/// One instanced draw: the patches of one view that share an index buffer and a slab of heights,
/// drawn as one, with the bindings they were bound through once.
struct Keep {
    indices: wgpu::Buffer,
    count: u32,
    instances: u32,
    bindings: gpu::Bindings,
}

/// One patch's own block in one of the arrays a draw reads per instance.
struct Placed {
    view: usize,
    patch: Patch,
    snap: [bool; 4],
    at: usize,
    /// What the program reads of the block; the rest of the row is never read.
    length: usize,
}

/// Which place in a retained buffer a constant read binds: the view's scene block, the draw's
/// own block, or the light grid.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Frame,
    Own,
    Grid,
}

impl Place {
    fn of(read: u32) -> Option<Self> {
        match read {
            read if read == binding(0, Stage::Vertex) || read == binding(0, Stage::Pixel) => {
                Some(Self::Frame)
            }
            read if read == binding(1, Stage::Vertex) || read == binding(1, Stage::Pixel) => {
                Some(Self::Own)
            }
            read if read == binding(2, Stage::Pixel) => Some(Self::Grid),
            _ => None,
        }
    }
}

/// The colour and depth formats and sample count a mode's pipelines are built for, which its
/// render bundle is recorded for.
fn attachments(mode: Mode, samples: u32) -> (wgpu::TextureFormat, wgpu::TextureFormat, u32) {
    match mode {
        Mode::Receiver => (
            super::shadow::BUFFER_FORMAT,
            super::shadow_blur::DEPTH_FORMAT,
            1,
        ),
        Mode::Underwater => (REFRACTION_FORMAT, DEPTH_FORMAT, 1),
        Mode::Lit(_) => (SCENE_FORMAT, DEPTH_FORMAT, samples),
    }
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
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
/// The retained entry this frame's prepared draws stand in.
pub struct Prepared {
    at: usize,
}
impl Terrain {
    pub fn new(
        device: &wgpu::Device,
        effect: &Arc<Effect>,
        bytes: &[u8],
        map: [u32; 2],
        samples: u32,
        workers: &Workers,
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
        let patches: Vec<Patch> = every_patch(map).collect();
        // A draw reads its instances' heights from its whole slab, so a slab is bound as one
        // storage buffer as well as one vertex buffer.
        let largest = device
            .limits()
            .max_buffer_size
            .min(u64::from(device.limits().max_storage_buffer_binding_size));
        let per_slab = (largest / PATCH_BYTES as u64) as usize;
        if per_slab == 0 {
            return Err(refuse(format!(
                "the device's largest buffer cannot hold one {PATCH_BYTES}-byte terrain patch"
            )));
        }
        let slabs: Vec<wgpu::Buffer> = patches
            .chunks(per_slab)
            .map(|chunk| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("native terrain heights"),
                    size: (chunk.len() * PATCH_BYTES) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: true,
                })
            })
            .collect();
        fill_slabs(&slabs, &patches, bytes, map, workers)?;
        for slab in &slabs {
            slab.unmap();
        }
        // One index buffer per shape: interior patches share one, the map's edges a few more.
        let mut shapes = BTreeMap::<[u32; 2], (wgpu::Buffer, u32)>::new();
        let mut geometry = BTreeMap::new();
        for (index, &patch) in patches.iter().enumerate() {
            let cells: [u32; 2] = std::array::from_fn(|axis| {
                ((map[axis] - patch.origin[axis]) / patch.step).min(SIDE)
            });
            let (indices, count) = shapes
                .entry(cells)
                .or_insert_with(|| {
                    let indices = indices(cells);
                    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("native terrain indices"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    (buffer, indices.len() as u32)
                })
                .clone();
            geometry.insert(
                patch,
                Geometry {
                    slab: index / per_slab,
                    offset: ((index % per_slab) * PATCH_BYTES) as u64,
                    indices,
                    count,
                },
            );
        }
        Ok(Self {
            effect: effect.clone(),
            pipelines: BTreeMap::new(),
            programs: BTreeMap::new(),
            positions,
            bytes: bytes.to_vec(),
            map,
            slabs,
            geometry,
            retained_heights: Vec::new(),
            blocks: Vec::new(),
            cut_from: None,
            samples,
            retained: Vec::new(),
            kept: BTreeMap::new(),
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
                // Another program draws another picture from the same inputs.
                self.kept.clear();
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
        // A view's patches differ only in their own block and in their slab of heights, so one
        // draw stands for all of them that share an index buffer and a slab.
        pass.instanced = (1 << binding(1, Stage::Vertex)) | (1 << binding(1, Stage::Pixel));
        pass.stream = Some(1);
        pass.primitive = gpu::primitive(&raster, wgpu::PrimitiveTopology::TriangleList).0;
        pass.depth_stencil =
            Some(gpu::depth_stencil(&DepthStencilState::default(), &raster, depth).0);
        let pipeline = gpu::Cache::default()
            .get_pass(device, &selection, &state, &pass)
            .map_err(refuse)?;
        if !pipeline.reads_per_instance() {
            return Err(refuse(
                "this terrain program cannot read its patches per instance",
            ));
        }
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
    /// The map's width and height in pixels, which the terrain's blocks tile.
    pub fn map(&self) -> [u32; 2] {
        self.map
    }

    /// The cover's blocks for one wrap view: the uniform grid's for that offset, or the free
    /// eye's blocks standing on that copy.
    fn view_blocks(
        &mut self,
        cover: &Cover,
        offset: f32,
    ) -> io::Result<Arc<[(Patch, [bool; 4])]>> {
        if self.cut_from.as_ref() != Some(cover) {
            self.cut_from = Some(cover.clone());
            self.blocks.clear();
        }
        let key = offset.to_bits();
        if let Some((_, kept)) = self.blocks.iter().find(|(view, _)| *view == key) {
            return Ok(Arc::clone(kept));
        }
        let made: Arc<[_]> = drawn_blocks(cover, self.map, offset)?
            .into_iter()
            .map(|block| {
                let patch = Patch {
                    origin: block.origin,
                    step: block.step,
                };
                (patch, block.snap)
            })
            .collect();
        self.blocks.push((key, Arc::clone(&made)));
        Ok(made)
    }
    fn views(&mut self, inputs: &Inputs<'_>) -> io::Result<Views> {
        let mut views = Vec::with_capacity(inputs.offsets.len());
        for offset in inputs.offsets {
            let blocks = self.view_blocks(inputs.cover, *offset)?;
            for &(patch, _) in blocks.iter() {
                if !self.geometry.contains_key(&patch) {
                    return Err(refuse(format!(
                        "terrain patch at {:?}, step {}, lies outside the patches made at load",
                        patch.origin, patch.step
                    )));
                }
            }
            let mut matrix = inputs.matrix;
            for (value, axis) in matrix[3].iter_mut().zip(inputs.matrix[0]) {
                *value += axis * offset;
            }
            views.push((blocks, matrix));
        }
        Ok(views)
    }

    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        mode: Mode,
        inputs: Inputs<'_>,
    ) -> io::Result<Prepared> {
        let pipeline = self.pipeline(arena.device(), mode)?;
        // An entry no frame of the last two prepared lets go of its buffer, bindings and bundle.
        self.retained.retain(|kept| kept.used + 2 > arena.frame());
        let views = self.views(&inputs)?;
        self.prepare_views(arena, mode, inputs, &pipeline, views)
    }

    /// Prepares `mode`'s draws only when something its compiled programs read has changed since
    /// the pass last drew: `Ok(None)` means the target still holds that very picture, so the
    /// pass is not drawn at all. `contents` gives what each texture of [`Inputs::textures`]
    /// holds, slot by slot, and `into` what the pass draws into; an input no writer describes
    /// leaves the key unformed and the pass is drawn every frame ([`Self::not_kept`]).
    ///
    /// The caller draws what this returns and then tells [`Self::submitted`] that the frame
    /// reached the queue, or [`Self::lost`] that nothing was drawn.
    pub fn prepare_kept(
        &mut self,
        arena: &gpu::Arena,
        mode: Mode,
        inputs: Inputs<'_>,
        contents: &[Option<Content>; 16],
        into: Option<Content>,
    ) -> io::Result<Option<Prepared>> {
        let frame = arena.frame();
        let pipeline = self.pipeline(arena.device(), mode)?;
        // A kept pass still uses its entry: without this its buffer, bindings and bundle would
        // be let go of after two kept frames and built again the moment an input changed.
        for kept in &mut self.retained {
            if kept.built.mode == mode {
                kept.used = frame;
            }
        }
        self.retained.retain(|kept| kept.used + 2 > frame);
        let views = self.views(&inputs)?;
        let samples = attachments(mode, self.samples).2;
        let reads = pipeline.constant_reads();
        let sampled = pipeline.sampled_slots().map(|physical| {
            // The terrain binds pixel textures only: a vertex one is bound by nothing here.
            let content = (!physical.is_multiple_of(2))
                .then(|| contents.get((physical / 2) as usize).copied().flatten())
                .flatten();
            (physical, content)
        });
        let draws = {
            let keyed = self.kept.entry(mode).or_default();
            let mut key = keyed.start();
            read_key(
                &mut key,
                mode,
                samples,
                reads,
                sampled,
                into,
                inputs.frames,
                &inputs.light_grid,
                &views,
            );
            let draws = keyed.draws(key);
            let record = |name| viewer_foundation::view_record::add("map scene", name, 1);
            record(if draws { "sea_bed_drawn" } else { "sea_bed_kept" });
            if let Some(part) = keyed.differed() {
                record(part);
            }
            draws
        };
        if !draws {
            return Ok(None);
        }
        self.prepare_views(arena, mode, inputs, &pipeline, views)
            .map(Some)
    }

    /// The frame reached the queue, so each target a kept pass drew into holds the picture its
    /// key describes.
    pub fn submitted(&mut self) {
        for keyed in self.kept.values_mut() {
            keyed.submitted();
        }
    }

    /// `mode`'s pass drew nothing this frame, so its target still holds whatever it held and
    /// the next frame draws it again.
    pub fn lost(&mut self, mode: Mode) {
        if let Some(keyed) = self.kept.get_mut(&mode) {
            keyed.lost();
        }
    }

    /// Every kept mode whose key cannot be formed, so its pass is drawn every frame: the
    /// technique and the reason.
    pub fn not_kept(&self) -> impl Iterator<Item = (&'static str, &str)> {
        self.kept
            .iter()
            .filter_map(|(mode, keyed)| Some((mode.technique(), keyed.unkept()?)))
    }

    fn prepare_views(
        &mut self,
        arena: &gpu::Arena,
        mode: Mode,
        inputs: Inputs<'_>,
        pipeline: &Arc<gpu::Pipeline>,
        views: Views,
    ) -> io::Result<Prepared> {
        let device = arena.device();
        let frame = arena.frame();
        let textures: BTreeMap<u32, wgpu::TextureView> = inputs
            .textures
            .iter()
            .enumerate()
            .map(|(i, v)| (binding(i as u32, Stage::Pixel), (*v).clone()))
            .collect();
        let built = Built {
            mode,
            samples: self.samples,
            scene: inputs.frames.iter().map(|frame| frame.len()).max().unwrap_or(0),
            offsets: inputs.offsets.iter().map(|offset| offset.to_bits()).collect(),
            cover: inputs.cover.clone(),
            textures: inputs.textures.iter().map(|view| (*view).clone()).collect(),
        };
        let at = match self
            .retained
            .iter()
            .position(|kept| kept.built == built && Arc::ptr_eq(&kept.pipeline, pipeline))
        {
            Some(at) => at,
            None => {
                let made = bind_retained(
                    arena,
                    pipeline,
                    built,
                    &views,
                    &inputs,
                    &textures,
                    &self.slabs,
                    &self.geometry,
                )?;
                self.retained.push(made);
                self.retained.len() - 1
            }
        };
        // The state the last block of each view leaves behind, which the next layers of that
        // wrap inherit; the frame draws the same blocks, so the state they leave is the same.
        if let Some(retained) = inputs.retained {
            let mut buffers = BTreeMap::new();
            if mode != Mode::Receiver {
                buffers.insert(
                    binding(2, Stage::Pixel),
                    bytemuck::bytes_of(&inputs.light_grid).to_vec(),
                );
            }
            for ((scene, offset), (blocks, matrix)) in
                inputs.frames.iter().zip(inputs.offsets).zip(&views)
            {
                let Some(&(patch, snap)) = blocks.last() else {
                    continue;
                };
                for stage in [Stage::Vertex, Stage::Pixel] {
                    buffers.insert(binding(0, stage), scene.to_vec());
                    buffers.insert(binding(1, stage), constants(patch, *matrix, snap));
                }
                let mut tracked = retained.borrow_mut();
                self.programs[&mode].bind(&mut tracked, &buffers, &textures);
                let view = offset.to_bits();
                let kept = self.retained_heights.iter().find_map(|(v, kept, heights)| {
                    (*v == view && *kept == patch).then(|| heights.clone())
                });
                let heights = match kept {
                    Some(heights) => heights,
                    None => {
                        let made: Arc<[u8]> =
                            bytemuck::cast_slice(&heights(&self.bytes, self.map, patch)).into();
                        self.retained_heights.retain(|(v, ..)| *v != view);
                        self.retained_heights.push((view, patch, made.clone()));
                        made
                    }
                };
                tracked.bind_vertex(1, 4, heights);
            }
        }
        // This frame's bytes: every view's scene block, the light grid when it changed, and the
        // blocks of each view whose matrix moved.
        let kept = &mut self.retained[at];
        kept.used = frame;
        let mut dirty: Vec<std::ops::Range<usize>> = Vec::new();
        for (view, scene) in inputs.frames.iter().enumerate() {
            let Some(&place) = kept.frames.get(view) else {
                continue;
            };
            kept.bytes[place..place + scene.len()].copy_from_slice(scene);
            dirty.push(place..place + scene.len());
        }
        if let Some(place) = kept.grid {
            let grid = bytemuck::bytes_of(&inputs.light_grid);
            if kept.bytes[place..place + grid.len()] != *grid {
                kept.bytes[place..place + grid.len()].copy_from_slice(grid);
                dirty.push(place..place + grid.len());
            }
        }
        for (view, (_, matrix)) in views.iter().enumerate() {
            if kept.matrices.get(view) == Some(matrix) {
                continue;
            }
            for place in kept.places.iter().filter(|place| place.view == view) {
                write_block(
                    &mut kept.bytes[place.at..place.at + place.length],
                    place,
                    *matrix,
                );
            }
            if let Some(range) = kept.view_blocks.get(view) {
                dirty.push(range.clone());
            }
            kept.matrices[view] = *matrix;
        }
        dirty.sort_unstable_by_key(|range| range.start);
        let mut writes: Vec<std::ops::Range<usize>> = Vec::new();
        for range in dirty {
            let (start, end) = (range.start / 4 * 4, range.end.next_multiple_of(4));
            match writes.last_mut() {
                Some(last) if start <= last.end + 1024 => last.end = last.end.max(end),
                _ => writes.push(start..end),
            }
        }
        for range in writes {
            arena.write(&kept.buffer, range.start as u64, &kept.bytes[range]);
        }
        if kept.bundle.is_none() {
            // Drawn directly until its draws have stood still.
            kept.still += 1;
            if kept.still >= STILL {
                kept.bundle = Some(record_bundle(
                    device,
                    attachments(mode, kept.built.samples),
                    &kept.draws,
                    &self.positions,
                ));
            }
        }
        Ok(Prepared { at })
    }
    /// Forgets the retained heights and cut blocks of wrap views not among `offsets`, this
    /// frame's.
    pub fn keep_views(&mut self, offsets: &[f32]) {
        self.retained_heights
            .retain(|(view, ..)| offsets.iter().any(|offset| offset.to_bits() == *view));
        self.blocks
            .retain(|(view, _)| offsets.iter().any(|offset| offset.to_bits() == *view));
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, prepared: &Prepared) {
        let kept = &self.retained[prepared.at];
        viewer_foundation::view_record::add(
            "map scene",
            if kept.bundle.is_some() {
                "terrain_bundles"
            } else {
                "terrain_direct"
            },
            1,
        );
        if let Some((bundle, counts)) = &kept.bundle {
            use shader_pipeline::gpu::stats::{Count, add};
            shader_pipeline::gpu::stats::state();
            pass.set_blend_constant(wgpu::Color::WHITE);
            pass.execute_bundles(std::iter::once(bundle));
            for (count, n) in [
                Count::Draws,
                Count::Pipelines,
                Count::BindGroups,
                Count::States,
            ]
            .into_iter()
            .zip(counts)
            {
                add(count, *n);
            }
            return;
        }
        shader_pipeline::gpu::stats::pipeline();
        pass.set_pipeline(&kept.pipeline.pipeline);
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        shader_pipeline::gpu::stats::state();
        pass.set_vertex_buffer(0, self.positions.slice(..));
        let mut previous = None;
        let mut indices = None;
        for keep in &kept.draws {
            keep.bindings.bind_after(pass, previous);
            previous = Some(&keep.bindings);
            if indices != Some(&keep.indices) {
                shader_pipeline::gpu::stats::state();
                pass.set_index_buffer(keep.indices.slice(..), wgpu::IndexFormat::Uint16);
                indices = Some(&keep.indices);
            }
            shader_pipeline::gpu::stats::draw();
            pass.draw_indexed(0..keep.count, 0, 0..keep.instances);
        }
    }
}

/// Everything one mode's pass reads, as its key: what its compiled programs read of each
/// constant block bound to them, what each texture they sample holds, what the pass draws into
/// and with, and the blocks it draws. Nothing else, and no byte of a block the programs do not
/// read, so a frame that changed only unread bytes repeats the key and is not drawn again.
///
/// `sampled` gives each texture binding the programs sample and what the texture bound there
/// holds; `reads` each constant binding they read. The sea-bed scene is the only pass kept, so
/// the names the key reports its parts by are its.
#[allow(
    clippy::too_many_arguments,
    reason = "every input of the pass, which is what the key is"
)]
fn read_key(
    key: &mut PassKey,
    mode: Mode,
    samples: u32,
    reads: &[shader_pipeline::Buffer],
    sampled: impl Iterator<Item = (u32, Option<Content>)>,
    into: Option<Content>,
    frames: &[&[u8]],
    light_grid: &[f32; 4],
    views: &Views,
) {
    key.part("sea_bed_target")
        .text(mode.technique())
        .number(u64::from(samples))
        .content(|| "the pass's own target".to_owned(), into);
    key.part("sea_bed_textures");
    for (physical, content) in sampled {
        key.number(u64::from(physical)).content(
            || format!("native texture slot {}", physical / 2),
            content,
        );
    }
    key.part("sea_bed_scene");
    for spec in reads {
        match Place::of(spec.binding) {
            // Every view's camera and lighting block, which is where the camera moves.
            Some(Place::Frame) => {
                for frame in frames {
                    key.read(spec, frame);
                }
            }
            Some(Place::Grid) => {
                key.read(spec, bytemuck::bytes_of(light_grid));
            }
            // A block's own bytes go in with that block, below.
            Some(Place::Own) => {}
            None => {
                key.unknown(format!(
                    "native constant buffer {} stands in no place the terrain binds",
                    spec.binding / 2
                ));
            }
        }
    }
    key.part("sea_bed_blocks");
    key.number(views.len() as u64);
    let own: Vec<_> = reads
        .iter()
        .filter(|spec| Place::of(spec.binding) == Some(Place::Own))
        .collect();
    let mut block = [0u8; BLOCK_BYTES];
    for (blocks, matrix) in views {
        key.number(blocks.len() as u64);
        for &(patch, snap) in blocks.iter() {
            // The patch alone chooses the block's vertices and its index buffer, which no
            // constant carries.
            key.number(u64::from(patch.origin[0]))
                .number(u64::from(patch.origin[1]))
                .number(u64::from(patch.step));
            constants_into(&mut block, patch, *matrix, snap);
            for spec in &own {
                key.read(spec, &block);
            }
        }
    }
}

/// Lays out a persistent buffer with a place for every block a mode's views draw, binds each
/// draw there once and writes every byte. A frame then writes only the blocks that changed.
#[allow(clippy::too_many_arguments, reason = "the draws, their blocks and the buffers they sit in")]
fn bind_retained(
    arena: &gpu::Arena,
    pipeline: &Arc<gpu::Pipeline>,
    built: Built,
    views: &[(Arc<[(Patch, [bool; 4])]>, [[f32; 4]; 4])],
    inputs: &Inputs<'_>,
    textures: &BTreeMap<u32, wgpu::TextureView>,
    slabs: &[wgpu::Buffer],
    geometry: &BTreeMap<Patch, Geometry>,
) -> io::Result<Retained> {
    let device = arena.device();
    let mode = built.mode;
    let empty = |built: Built, buffer: wgpu::Buffer| Retained {
        pipeline: Arc::clone(pipeline),
        buffer,
        bytes: Vec::new(),
        frames: Vec::new(),
        grid: None,
        draws: Vec::new(),
        places: Vec::new(),
        view_blocks: views.iter().map(|_| 0..0).collect(),
        matrices: views.iter().map(|(_, matrix)| *matrix).collect(),
        bundle: None,
        still: 0,
        used: 0,
        built,
    };
    let spare = || {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native terrain constants"),
            size: wgpu::COPY_BUFFER_ALIGNMENT,
            usage: wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    let scene_bytes = built.scene;
    let grid_bytes = size_of_val(&inputs.light_grid);
    if views.iter().all(|(blocks, _)| blocks.is_empty()) {
        return Ok(empty(built, spare()));
    }
    // The patches of one view that share an index buffer and a slab of heights draw as one.
    let mut groups: Vec<(usize, usize, wgpu::Buffer, u32, Vec<(Patch, [bool; 4])>)> = Vec::new();
    let mut bases: Vec<u32> = Vec::new();
    for (view, (blocks, _)) in views.iter().enumerate() {
        for &(patch, snap) in blocks.iter() {
            let made = &geometry[&patch];
            match groups.iter_mut().find(|(at, slab, indices, ..)| {
                *at == view && *slab == made.slab && *indices == made.indices
            }) {
                Some((.., patches)) => patches.push((patch, snap)),
                None => groups.push((
                    view,
                    made.slab,
                    made.indices.clone(),
                    made.count,
                    vec![(patch, snap)],
                )),
            }
        }
    }
    let mut group_bases = Vec::with_capacity(groups.len());
    for (_, _, _, _, patches) in &groups {
        group_bases.push(bases.len() as u32);
        for (patch, _) in patches {
            let offset = geometry[patch].offset;
            bases.push(
                u32::try_from(offset / size_of::<[i16; 2]>() as u64)
                    .map_err(|_| refuse("a terrain slab stands past a stream of u32 vertices"))?,
            );
        }
    }
    let bases_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("native terrain instance slabs"),
        size: (bases.len() * 4).max(4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    arena.write(&bases_buffer, 0, bytemuck::cast_slice(&bases));
    // One program a slab: the slab is bound with the draw's other resources.
    let mut programs: Vec<(usize, gpu::BindingProgram)> = Vec::new();
    for &(_, slab, ..) in &groups {
        if programs.iter().any(|(held, _)| *held == slab) {
            continue;
        }
        let made = pipeline
            .rows_program(
                arena,
                |read| textures.get(&read),
                Some([slabs[slab].clone(), bases_buffer.clone()]),
            )
            .map_err(refuse)?;
        programs.push((slab, made));
    }
    let program = &programs.first().expect("a group was made above").1;
    // Every shared read's place is reserved for the largest read of it, as the arena reserves
    // one; a read taken per instance keeps a row of its own stride instead.
    let mut reserved = [0usize; 3];
    let mut rows: Vec<(u32, usize)> = Vec::new();
    for spec in program.constant_reads() {
        let kind = Place::of(spec.binding).ok_or_else(|| {
            refuse(format!(
                "native constant buffer {} has not been bound",
                spec.binding / 2
            ))
        })?;
        if kind == Place::Grid && mode == Mode::Receiver {
            return Err(refuse("native constant buffer 2 has not been bound"));
        }
        match program
            .instanced_rows()
            .iter()
            .find(|(binding, _)| *binding == spec.binding)
        {
            Some((_, stride)) => rows.push((spec.binding, *stride as usize)),
            None => {
                let bytes = match kind {
                    Place::Frame => scene_bytes,
                    Place::Grid => grid_bytes,
                    Place::Own => {
                        return Err(refuse(format!(
                            "native constant buffer {} is not read per instance",
                            spec.binding / 2
                        )));
                    }
                };
                let slot = &mut reserved[kind as usize];
                *slot = (*slot).max(bytes.max(spec.size as usize));
            }
        }
    }
    if rows.len() != program.instanced_rows().len() {
        return Err(refuse("a terrain row is read by no constant buffer"));
    }
    let alignment = device.limits().min_uniform_buffer_offset_alignment as usize;
    let mut top = 0usize;
    let mut frames = Vec::with_capacity(views.len());
    for _ in views {
        let at = align(top, alignment);
        top = at + reserved[Place::Frame as usize];
        frames.push(at);
    }
    let grid = (mode != Mode::Receiver).then(|| {
        let at = align(top, alignment);
        top = at + reserved[Place::Grid as usize];
        at
    });
    // Each draw's rows block, then each of its per-instance arrays, every row of one stride so
    // the instance index reaches it from the base the rows block gives.
    let mut laid = Vec::with_capacity(groups.len());
    let mut places = Vec::new();
    let mut view_blocks: Vec<std::ops::Range<usize>> = views.iter().map(|_| 0..0).collect();
    for ((view, _, _, _, patches), base) in groups.iter().zip(&group_bases) {
        let block = align(top, alignment);
        top = block + gpu::ROWS_BYTES;
        let mut arrays = Vec::with_capacity(rows.len());
        for (_, stride) in &rows {
            let start = align(top, *stride);
            top = start + patches.len() * stride;
            arrays.push(start);
            for (index, &(patch, snap)) in patches.iter().enumerate() {
                places.push(Placed {
                    view: *view,
                    patch,
                    snap,
                    at: start + index * stride,
                    length: BLOCK_BYTES.min(*stride),
                });
            }
        }
        let range = &mut view_blocks[*view];
        if range.start == range.end {
            *range = block..top;
        } else {
            range.end = range.end.max(top);
        }
        laid.push((block, arrays, *base));
    }
    let mut bytes = vec![0u8; align(top.max(1), alignment)];
    for (view, scene) in inputs.frames.iter().enumerate() {
        if let Some(&at) = frames.get(view) {
            bytes[at..at + scene.len()].copy_from_slice(scene);
        }
    }
    if let Some(at) = grid {
        let value = bytemuck::bytes_of(&inputs.light_grid);
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    for place in &places {
        let matrix = views[place.view].1;
        write_block(&mut bytes[place.at..place.at + place.length], place, matrix);
    }
    if bytes.len() as u64 > device.limits().max_buffer_size {
        return Err(refuse(format!(
            "the terrain's {} constant bytes exceed the device's {}-byte buffer limit",
            bytes.len(),
            device.limits().max_buffer_size
        )));
    }
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("native terrain constants"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut draws = Vec::with_capacity(groups.len());
    for ((view, slab, indices, count, patches), (block, arrays, base)) in
        groups.iter().zip(&laid)
    {
        let program = &programs
            .iter()
            .find(|(held, _)| held == slab)
            .expect("a program was made for every slab")
            .1;
        let mut indexes: Vec<u32> = arrays
            .iter()
            .zip(&rows)
            .map(|(at, (_, stride))| (at / stride) as u32)
            .collect();
        indexes.push(*base);
        bytes[*block..*block + gpu::ROWS_BYTES]
            .copy_from_slice(&program.rows_block(&indexes).map_err(refuse)?);
        let blocks = program
            .constant_reads()
            .iter()
            .map(|spec| {
                let place = match rows.iter().position(|(binding, _)| *binding == spec.binding) {
                    Some(row) => (arrays[row], rows[row].1),
                    None => match Place::of(spec.binding).expect("every read was placed above") {
                        Place::Frame => (frames[*view], reserved[Place::Frame as usize]),
                        Place::Grid => (
                            grid.expect("reserved for a mode that reads it"),
                            reserved[Place::Grid as usize],
                        ),
                        Place::Own => unreachable!("the own block is read per instance"),
                    },
                };
                (&bytes[place.0..place.0 + place.1], place.0 as u32)
            })
            .collect::<Vec<_>>();
        draws.push(Keep {
            indices: indices.clone(),
            count: *count,
            instances: patches.len() as u32,
            bindings: program
                .bind_rows(arena, &buffer, &blocks, *block as u32)
                .map_err(refuse)?,
        });
    }
    arena.write(&buffer, 0, &bytes);
    Ok(Retained {
        frames,
        grid,
        draws,
        places,
        view_blocks,
        bytes,
        ..empty(built, buffer)
    })
}

/// A patch's own block written where a draw reads it, cut to what its program reads.
fn write_block(data: &mut [u8], place: &Placed, matrix: [[f32; 4]; 4]) {
    let mut block = [0u8; BLOCK_BYTES];
    constants_into(&mut block, place.patch, matrix, place.snap);
    data.copy_from_slice(&block[..data.len()]);
}

/// A mode's retained draws recorded into a render bundle for a pass of `color` and `depth` at
/// `samples`, as [`Terrain::draw`] records them into a pass, with the draws, pipeline, bind
/// group and state sets it holds, counted where it is executed. A bundle sets no blend
/// constant: the pass sets it before executing the bundle.
fn record_bundle(
    device: &wgpu::Device,
    (color, depth, samples): (wgpu::TextureFormat, wgpu::TextureFormat, u32),
    draws: &[Keep],
    positions: &wgpu::Buffer,
) -> (wgpu::RenderBundle, [u64; 4]) {
    let read_only = draws.iter().fold(gpu::ReadOnly::ALL, |all, keep| {
        all.and(keep.bindings.read_only())
    });
    let mut bundle = device.create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
        label: Some("native terrain draws"),
        color_formats: &[Some(color)],
        depth_stencil: Some(read_only.bundle(depth)),
        sample_count: samples,
        multiview: None,
    });
    let mut counts = [0u64; 4];
    counts[3] += 1;
    bundle.set_vertex_buffer(0, positions.slice(..));
    let mut previous = None;
    let mut indices = None;
    for keep in draws {
        let [pipelines, groups] = keep.bindings.bundle_after(&mut bundle, previous);
        counts[1] += pipelines;
        counts[2] += groups;
        previous = Some(&keep.bindings);
        if indices != Some(&keep.indices) {
            counts[3] += 1;
            bundle.set_index_buffer(keep.indices.slice(..), wgpu::IndexFormat::Uint16);
            indices = Some(&keep.indices);
        }
        counts[0] += 1;
        bundle.draw_indexed(0..keep.count, 0, 0..keep.instances);
    }
    let bundle = bundle.finish(&wgpu::RenderBundleDescriptor {
        label: Some("native terrain draws"),
    });
    (bundle, counts)
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
    /// The blocks of one wrap view are those standing on that copy, and a block's own constants
    /// carry its origin and step.
    #[test]
    fn wrap_selection_and_native_buffer_bytes() {
        let block = |origin: [u32; 2], step, offset| Block {
            origin,
            step,
            offset,
            snap: [false; 4],
        };
        let cover = Cover::Blocks(vec![
            block([0, 0], 1, 0.),
            block([0, 64], 1, 0.),
            block([64, 0], 1, -128.),
            block([64, 64], 1, -128.),
        ]);
        assert_eq!(
            drawn_blocks(&cover, [128, 128], 0.).unwrap(),
            vec![block([0, 0], 1, 0.), block([0, 64], 1, 0.)]
        );
        assert_eq!(
            drawn_blocks(&cover, [128, 128], -128.).unwrap(),
            vec![block([64, 0], 1, -128.), block([64, 64], 1, -128.)]
        );
        assert!(drawn_blocks(&cover, [128, 128], 128.).unwrap().is_empty());
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

    /// One sea-bed pass's inputs, as a key over them answers "must this be drawn again?".
    struct Pass {
        keyed: Keyed,
        /// The program reads the camera matrix of the scene block and nothing past byte 64, and
        /// the whole of each block's own constants.
        reads: Vec<shader_pipeline::Buffer>,
        sampled: Vec<(u32, Option<Content>)>,
        into: Option<Content>,
        scene: Vec<u8>,
        light_grid: [f32; 4],
        views: Views,
    }

    impl Pass {
        fn new() -> Self {
            let patch = |origin| Patch { origin, step: 1 };
            let blocks: Arc<[(Patch, [bool; 4])]> =
                vec![(patch([0, 0]), [false; 4]), (patch([64, 0]), [false; 4])].into();
            Self {
                keyed: Keyed::default(),
                reads: vec![
                    shader_pipeline::Buffer {
                        binding: binding(0, Stage::Pixel),
                        size: 352,
                        reads: vec![0..64],
                    },
                    shader_pipeline::Buffer {
                        binding: binding(1, Stage::Vertex),
                        size: BLOCK_BYTES as u32,
                        reads: vec![64..80],
                    },
                ],
                sampled: vec![
                    (binding(0, Stage::Pixel), Some(Content::next())),
                    (binding(12, Stage::Pixel), Some(Content::next())),
                ],
                into: Some(Content::next()),
                scene: vec![7; 352],
                light_grid: [0.0, 0.0, 0.125, 0.125],
                views: vec![(blocks, [[0.0; 4]; 4])],
            }
        }

        /// Whether this frame must be drawn, as the pass's own call site asks it, taking a
        /// drawn frame as submitted.
        fn draws(&mut self) -> bool {
            let mut key = self.keyed.start();
            read_key(
                &mut key,
                Mode::Underwater,
                1,
                &self.reads,
                self.sampled.iter().copied(),
                self.into,
                &[&self.scene],
                &self.light_grid,
                &self.views,
            );
            let draws = self.keyed.draws(key);
            if draws {
                self.keyed.submitted();
            }
            draws
        }
    }

    /// The sea-bed pass is drawn again for a byte its programs read and not for a byte they do
    /// not, whatever that byte is: the key is the read bytes themselves.
    #[test]
    fn a_changed_read_byte_redraws_the_sea_bed_and_an_unread_byte_does_not() {
        let mut pass = Pass::new();
        assert!(pass.draws(), "the target holds nothing yet");
        assert!(!pass.draws(), "nothing changed");
        // The animation clock and the hour's own fields sit past byte 64, which this program
        // does not read: the water moves every frame and the sea bed does not.
        pass.scene[236] = 9;
        pass.scene[351] = 9;
        assert!(!pass.draws(), "the clock moved, and no program reads it");
        // The camera matrix is read.
        pass.scene[4] = 9;
        assert!(pass.draws(), "the camera moved");
        assert!(!pass.draws());
        // A block carries its origin at byte 64 and its step at 72; the switches at 80 are past
        // what this program reads.
        let matrix = pass.views[0].1;
        let mut moved: Vec<_> = pass.views[0].0.to_vec();
        moved[1].1 = [true, false, false, false];
        pass.views = vec![(moved.into(), matrix)];
        assert!(!pass.draws(), "a snap switch no program reads");
    }

    /// Every block drawn is in the key, so a cover that cut one more or moved one is drawn
    /// again even where the blocks it kept are unchanged.
    #[test]
    fn a_changed_block_list_redraws_the_sea_bed() {
        let mut pass = Pass::new();
        assert!(pass.draws());
        let matrix = pass.views[0].1;
        let mut more: Vec<_> = pass.views[0].0.to_vec();
        more.push((
            Patch {
                origin: [128, 0],
                step: 1,
            },
            [false; 4],
        ));
        pass.views = vec![(more.clone().into(), matrix)];
        assert!(pass.draws(), "a block more");
        more.pop();
        pass.views = vec![(more.into(), matrix)];
        assert!(pass.draws(), "a block fewer");
        let mut coarser: Vec<_> = pass.views[0].0.to_vec();
        coarser[0].0.step = 2;
        pass.views = vec![(coarser.into(), matrix)];
        assert!(pass.draws(), "a block at another step draws other vertices");
    }

    /// A texture the pass samples, written by the hour or by the frame before it, redraws it;
    /// a target made afresh redraws it however little else changed.
    #[test]
    fn a_rewritten_texture_or_a_remade_target_redraws_the_sea_bed() {
        let mut pass = Pass::new();
        assert!(pass.draws());
        pass.sampled[1].1 = Some(Content::next());
        assert!(pass.draws(), "last frame's shadow buffer was written");
        assert!(!pass.draws());
        pass.into = Some(Content::next());
        assert!(pass.draws(), "the sea-bed target was made again");
        assert!(!pass.draws());
    }

    /// A texture whose writer publishes no content leaves no key to repeat, so the pass is
    /// drawn every frame and says which slot it was.
    #[test]
    fn a_texture_that_publishes_no_content_draws_the_sea_bed_every_frame() {
        let mut pass = Pass::new();
        pass.sampled[1].1 = None;
        for _ in 0..3 {
            assert!(pass.draws());
            assert_eq!(
                pass.keyed.unkept(),
                Some("native texture slot 12 publishes no content")
            );
        }
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
