use eframe::wgpu::{self, util::DeviceExt};
use content_reader_layers::ResolvedLayers;
use content_reader_gui::pdxshader::{DepthStencilState, RasterizerState, program::Stage};
use content_reader_image::native::Decode;
use shader_pipeline::{ScalarKind, binding, gpu, library::Effect};
use std::cell::RefCell;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
};
use viewer_gpu::gpu_contract::DEPTH_FORMAT;
use viewer_gpu::pass_key::{Keyed, PassKey};
use viewer_gpu::textures::Content;
use viewer_gpu::gpu_contract::REFRACTION_FORMAT;
use viewer_gpu::gpu_contract::SCENE_FORMAT;
use viewer_runtime::parallel::Workers;

#[derive(Clone, Debug, PartialEq)]
pub enum Cover {
    Blocks(Vec<Block>),
    Refused(&'static str),
}

impl Cover {
    pub fn blocks(&self) -> usize {
        match self {
            Self::Blocks(blocks) => blocks.len(),
            Self::Refused(_) => 0,
        }
    }

    pub fn quads(&self) -> u64 {
        match self {
            Self::Blocks(blocks) => blocks.len() as u64 * u64::from(SIDE * SIDE),
            Self::Refused(_) => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub origin: [u32; 2],
    pub step: u32,
    pub offset: f32,
    pub snap: [bool; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stand {
    pub origin: [u32; 2],
    pub step: u32,
    pub snap: [bool; 4],
    pub base: u32,
}

pub const SIDE: u32 = 64;
fn refuse(reason: impl Into<String>) -> io::Error {
    io::Error::other(format!(
        "Map shader terrain: 1 refused pass: {}",
        reason.into()
    ))
}
pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Arc<Effect>> {
    let (mut library, defines) = viewer_scene_base::shader_sources::library(layers, map, decode);
    library
        .load(b"gfx/FX/pdxmap.shader", &defines)
        .map_err(refuse)
}

pub(super) fn positions() -> Vec<[i16; 4]> {
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
pub(super) struct Patch {
    pub(super) origin: [u32; 2],
    pub(super) step: u32,
}
fn heights(bytes: &[u8], map: [u32; 2], patch: Patch) -> Vec<[i16; 2]> {
    let mut out = Vec::new();
    heights_into(&positions(), bytes, map, patch, &mut out);
    out
}
pub(super) fn heights_into(
    positions: &[[i16; 4]],
    bytes: &[u8],
    map: [u32; 2],
    patch: Patch,
    out: &mut Vec<[i16; 2]>,
) {
    out.clear();
    out.extend(positions.iter().map(|&position| vertex_heights(position, bytes, map, patch)));
}

pub(super) fn vertex_heights(
    [x, z, edge, direction]: [i16; 4],
    bytes: &[u8],
    map: [u32; 2],
    patch: Patch,
) -> [i16; 2] {
    let sample = |x: u32, z: u32| {
        let x = x % map[0];
        let z = z.min(map[1] - 1);
        i16::from(bytes[((map[1] - 1 - z) * map[0] + x) as usize]) * 10
    };
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
}

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

fn drawn_blocks(cover: &Cover, map: [u32; 2], offset: f32) -> io::Result<Vec<Block>> {
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

fn constants_into(data: &mut [u8], patch: Patch, matrix: [[f32; 4]; 4], snap: [bool; 4]) {
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

const BLOCK_BYTES: usize = 112;

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
const PATCH_BYTES: usize = ((SIDE + 1) * (SIDE + 1)) as usize * size_of::<[i16; 2]>();
struct Geometry {
    slab: usize,
    offset: u64,
    coast: Option<(usize, u64)>,
    indices: wgpu::Buffer,
    count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heights {
    Sampled,
    Coast,
    Rim(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drawn {
    patch: Patch,
    snap: [bool; 4],
    heights: Heights,
}

#[derive(Clone)]
struct Rim {
    rules: Arc<[u8]>,
    heights: Arc<[u8]>,
}

type BlockRim = Option<(Arc<[u8]>, Option<Arc<[u8]>>)>;

struct Cut {
    cover: Cover,
    views: Vec<(u32, Arc<[Drawn]>)>,
    rims: Vec<Rim>,
    stream: Option<wgpu::Buffer>,
}

fn kept_view(before: &Cut, offset: u32, drawn: &[Drawn], slots: &[Rim], stream: &Option<wgpu::Buffer>) -> Option<Arc<[Drawn]>> {
    let (_, was) = before.views.iter().find(|(view, _)| *view == offset)?;
    let same_rims = drawn.iter().all(|block| match block.heights {
        Heights::Rim(slot) => {
            before.stream == *stream
                && before.rims.get(slot).zip(slots.get(slot)).is_some_and(|(was, now)| Arc::ptr_eq(&was.heights, &now.heights))
        }
        Heights::Sampled | Heights::Coast => true,
    });
    (**was == *drawn && same_rims).then(|| Arc::clone(was))
}

fn cells(map: [u32; 2], patch: Patch) -> [u32; 2] {
    std::array::from_fn(|axis| ((map[axis] - patch.origin[axis]) / patch.step).min(SIDE))
}

fn stream(geometry: &BTreeMap<Patch, Geometry>, rim_slab: usize, drawn: &Drawn) -> (usize, u64) {
    let made = &geometry[&drawn.patch];
    match drawn.heights {
        Heights::Coast => made.coast.unwrap_or((made.slab, made.offset)),
        Heights::Rim(index) => (rim_slab, (index * PATCH_BYTES) as u64),
        Heights::Sampled => (made.slab, made.offset),
    }
}

fn every_patch(map: [u32; 2]) -> impl Iterator<Item = Patch> {
    let widest = widest_step(map);
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
fn widest_step(map: [u32; 2]) -> u32 {
    map[0].max(map[1]).div_ceil(SIDE).next_power_of_two().max(256)
}
const FILL_GROUP: usize = 64;
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
struct Coasts {
    from: Arc<Effect>,
    coast: Result<super::terrain_coast::Coast, String>,
    heights: Vec<(Patch, Vec<[i16; 2]>)>,
}

impl Coasts {
    fn new(
        effect: &Arc<Effect>,
        patches: &[Patch],
        bytes: &[u8],
        map: [u32; 2],
        workers: &Workers,
    ) -> Self {
        let coast = water_height(effect).map(|water| {
            super::terrain_coast::Coast::new(
                bytes,
                map,
                water,
                viewer_gpu::province_map::HEIGHT_SCALE,
                widest_step(map),
            )
        });
        let mut heights: Vec<_> = coast.as_ref().map_or_else(
            |_| Vec::new(),
            |coast| {
                coastal_heights(coast, patches, bytes, map, workers)
                    .into_iter()
                    .map(|(index, heights)| (patches[index], heights))
                    .collect()
            },
        );
        heights.sort_unstable_by_key(|(patch, _)| *patch);
        Self {
            from: Arc::clone(effect),
            coast,
            heights,
        }
    }

    fn follows(&self, effect: &Arc<Effect>) -> bool {
        Arc::ptr_eq(&self.from, effect)
    }

    fn capped(&self, patch: Patch) -> Option<&[[i16; 2]]> {
        let at = self.heights.binary_search_by_key(&patch, |(held, _)| *held).ok()?;
        Some(&self.heights[at].1)
    }
}

fn coastal_heights(
    coast: &super::terrain_coast::Coast,
    patches: &[Patch],
    bytes: &[u8],
    map: [u32; 2],
    workers: &Workers,
) -> Vec<(usize, Vec<[i16; 2]>)> {
    let positions = positions();
    let found: Vec<Mutex<Vec<(usize, Vec<[i16; 2]>)>>> = (0..patches.len().div_ceil(FILL_GROUP))
        .map(|_| Mutex::new(Vec::new()))
        .collect();
    workers.run(found.len(), &|group| {
        let mut out = Vec::new();
        for index in group * FILL_GROUP..((group + 1) * FILL_GROUP).min(patches.len()) {
            let patch = patches[index];
            if patch.step < 2 {
                continue;
            }
            let mut heights = Vec::with_capacity(positions.len());
            heights_into(&positions, bytes, map, patch, &mut heights);
            if coast.cap(&positions, patch.origin, patch.step, &mut heights) {
                out.push((index, heights));
            }
        }
        *found[group].lock().unwrap_or_else(|e| e.into_inner()) = out;
    });
    found
        .into_iter()
        .flat_map(|group| group.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect()
}

fn coast_slabs(
    device: &wgpu::Device,
    coasts: &Coasts,
    per_slab: usize,
    slabs: &mut Vec<wgpu::Buffer>,
) -> BTreeMap<Patch, (usize, u64)> {
    let mut at = BTreeMap::new();
    for chunk in coasts.heights.chunks(per_slab) {
        let slab = slabs.len();
        let mut contents = Vec::with_capacity(chunk.len() * PATCH_BYTES);
        for (place, (patch, heights)) in chunk.iter().enumerate() {
            at.insert(*patch, (slab, (place * PATCH_BYTES) as u64));
            contents.extend_from_slice(bytemuck::cast_slice(heights));
        }
        slabs.push(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("native terrain coast heights"),
            contents: &contents,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
        }));
    }
    at
}

fn water_height(effect: &Effect) -> Result<f32, String> {
    effect
        .source
        .static_float("WATER_HEIGHT", &effect.defines)
        .and_then(|height| {
            height
                .is_finite()
                .then_some(height)
                .ok_or_else(|| format!("WATER_HEIGHT is {height}"))
        })
}

pub const SHADOW_SLOT: u32 = 12;

pub struct Terrain {
    pub effect: Arc<Effect>,
    pipelines: BTreeMap<Mode, Result<Arc<gpu::Pipeline>, String>>,
    programs: BTreeMap<Mode, viewer_scene_base::shader_scene::Program>,
    texel_reads: BTreeMap<Mode, String>,
    positions: wgpu::Buffer,
    bytes: Vec<u8>,
    map: [u32; 2],
    slabs: Vec<wgpu::Buffer>,
    sampled_slabs: usize,
    per_slab: usize,
    coasts: Coasts,
    workers: Workers,
    geometry: BTreeMap<Patch, Geometry>,
    retained_heights: Vec<(u32, Patch, bool, Arc<[u8]>)>,
    cut: Option<Cut>,
    rim_cache: BTreeMap<(Patch, Arc<[u8]>), Option<Arc<[u8]>>>,
    last_rims: Option<(super::terrain_coast::Steps, BTreeMap<(Patch, i64), BlockRim>)>,
    rim_stream: Option<wgpu::Buffer>,
    rim_slots: Vec<Rim>,
    no_blocks: Arc<[Drawn]>,
    samples: u32,
    retained: Vec<Retained>,
    kept: BTreeMap<Mode, Keyed>,
}

type Views = Vec<(Arc<[Drawn]>, [[f32; 4]; 4])>;

const STILL: u32 = 8;

#[derive(PartialEq)]
struct Built {
    mode: Mode,
    samples: u32,
    scene: usize,
    offsets: Vec<u32>,
    views: Vec<ViewKey>,
    textures: Vec<wgpu::TextureView>,
}

struct ViewKey(Arc<[Drawn]>);

impl PartialEq for ViewKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct Retained {
    built: Built,
    pipeline: Arc<gpu::Pipeline>,
    buffer: wgpu::Buffer,
    bases: wgpu::Buffer,
    programs: Vec<(wgpu::Buffer, gpu::BindingProgram, gpu::Bindings)>,
    bytes: Vec<u8>,
    frames: Vec<usize>,
    grid: Option<usize>,
    draws: Vec<Keep>,
    places: Vec<Placed>,
    view_blocks: Vec<std::ops::Range<usize>>,
    matrices: Vec<[[f32; 4]; 4]>,
    bundle: Option<(wgpu::RenderBundle, [u64; 4])>,
    still: u32,
    used: u64,
    prepared: u64,
}

struct Keep {
    indices: wgpu::Buffer,
    count: u32,
    instances: u32,
    bindings: gpu::Bindings,
}

struct Placed {
    view: usize,
    patch: Patch,
    snap: [bool; 4],
    at: usize,
    length: usize,
}

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

fn attachments(mode: Mode, samples: u32) -> (wgpu::TextureFormat, wgpu::TextureFormat, u32) {
    match mode {
        Mode::Receiver => (
            viewer_scene_base::shadow::BUFFER_FORMAT,
            viewer_scene_base::shadow_blur::DEPTH_FORMAT,
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
    pub retained: Option<&'a RefCell<viewer_scene_base::shader_scene::State>>,
    pub frames: &'a [&'a [u8]],
    pub offsets: &'a [f32],
    pub cover: &'a Cover,
    pub matrix: [[f32; 4]; 4],
    pub light_grid: [f32; 4],
    pub textures: [&'a wgpu::TextureView; 16],
}
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
        let mut slabs: Vec<wgpu::Buffer> = patches
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
        let sampled_slabs = slabs.len();
        let coasts = Coasts::new(effect, &patches, bytes, map, workers);
        let coast_at = coast_slabs(device, &coasts, per_slab, &mut slabs);
        let mut shapes = BTreeMap::<[u32; 2], (wgpu::Buffer, u32)>::new();
        let mut geometry = BTreeMap::new();
        for (index, &patch) in patches.iter().enumerate() {
            let cells = cells(map, patch);
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
                    coast: coast_at.get(&patch).copied(),
                    indices,
                    count,
                },
            );
        }
        Ok(Self {
            effect: effect.clone(),
            pipelines: BTreeMap::new(),
            programs: BTreeMap::new(),
            texel_reads: BTreeMap::new(),
            positions,
            bytes: bytes.to_vec(),
            map,
            slabs,
            sampled_slabs,
            per_slab,
            coasts,
            workers: workers.clone(),
            geometry,
            retained_heights: Vec::new(),
            cut: None,
            rim_cache: BTreeMap::new(),
            last_rims: None,
            rim_stream: None,
            rim_slots: Vec::new(),
            no_blocks: Arc::from([]),
            samples,
            retained: Vec::new(),
            kept: BTreeMap::new(),
        })
    }
    pub fn warm(&mut self, device: &wgpu::Device, modes: &[Mode]) {
        let effect = Arc::clone(&self.effect);
        for &mode in modes {
            drop(self.pipeline(device, mode));
        }
        if !Arc::ptr_eq(&effect, &self.effect) {
            for &mode in modes {
                drop(self.pipeline(device, mode));
            }
        }
    }

    fn pipeline(&mut self, device: &wgpu::Device, mode: Mode) -> io::Result<Arc<gpu::Pipeline>> {
        if !self.pipelines.contains_key(&mode) {
            let mut built = self.build_pipeline(device, mode);
            if let Err(error) = &built
                && let Some(recorded) = self.effect.fall_back(&error.to_string())
            {
                self.effect = recorded;
                self.pipelines.clear();
                self.programs.clear();
                self.texel_reads.clear();
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
        if mode != Mode::Receiver
            && let Some(reads) = viewer_scene_base::shadow::texel_reads(&compiled, SHADOW_SLOT, technique)
        {
            self.texel_reads.insert(mode, reads);
        }
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
                viewer_scene_base::shadow::BUFFER_FORMAT,
                viewer_scene_base::shadow_blur::DEPTH_FORMAT,
                1,
            ),
            Mode::Underwater => (REFRACTION_FORMAT, DEPTH_FORMAT, 1),
            Mode::Lit(_) => (SCENE_FORMAT, DEPTH_FORMAT, self.samples),
        };
        let mut pass = gpu::Pass::packed(&compiled, &state, format, samples);
        let mut attributes: [Vec<wgpu::VertexAttribute>; 2] = Default::default();
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
        let raster = RasterizerState {
            front_ccw: true,
            ..Default::default()
        };
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
        viewer_scene_base::shader_constants::scene(&pipeline).map_err(refuse)?;
        self.programs.insert(
            mode,
            viewer_scene_base::shader_scene::Program::new(
                &selection,
                &pass.vertices,
                &state,
                &raster,
                &DepthStencilState::default(),
            ),
        );
        Ok(pipeline)
    }
    pub fn texel_reads(&self, mode: Mode) -> Option<&str> {
        self.pipelines
            .get(&mode)
            .is_some_and(Result::is_ok)
            .then(|| self.texel_reads.get(&mode))
            .flatten()
            .map(String::as_str)
    }

    pub fn map(&self) -> [u32; 2] {
        self.map
    }

    pub fn coast_refusal(&self) -> Option<&str> {
        self.coasts.coast.as_ref().err().map(String::as_str)
    }

    fn follow_effect(&mut self, device: &wgpu::Device) {
        if self.coasts.follows(&self.effect) {
            return;
        }
        let patches: Vec<Patch> = self.geometry.keys().copied().collect();
        self.coasts = Coasts::new(&self.effect, &patches, &self.bytes, self.map, &self.workers);
        self.slabs.truncate(self.sampled_slabs);
        let at = coast_slabs(device, &self.coasts, self.per_slab, &mut self.slabs);
        for (patch, made) in &mut self.geometry {
            made.coast = at.get(patch).copied();
        }
        self.cut = None;
        self.rim_cache.clear();
        self.last_rims = None;
        self.rim_stream = None;
        self.rim_slots.clear();
        self.retained_heights.clear();
    }

    fn cut(&mut self, device: &wgpu::Device, arena: Option<&gpu::Arena>, cover: &Cover) -> io::Result<&Cut> {
        self.follow_effect(device);
        if self.cut.as_ref().is_none_or(|cut| cut.cover != *cover) {
            let before = self.cut.take();
            let made = self.cut_cover(device, arena, cover, before.as_ref())?;
            self.cut = Some(made);
        }
        Ok(self.cut.as_ref().expect("the cover was cut above"))
    }

    fn cut_cover(
        &mut self,
        device: &wgpu::Device,
        arena: Option<&gpu::Arena>,
        cover: &Cover,
        before: Option<&Cut>,
    ) -> io::Result<Cut> {
        let offsets = match cover {
            Cover::Blocks(blocks) => blocks.iter().fold(Vec::new(), |mut offsets, block| {
                if !offsets.contains(&block.offset) {
                    offsets.push(block.offset);
                }
                offsets
            }),
            Cover::Refused(reason) => return Err(refuse(*reason)),
        };
        let mut cut = Vec::with_capacity(offsets.len());
        for offset in offsets {
            let blocks = drawn_blocks(cover, self.map, offset)?;
            if let Some(block) = blocks.iter().find(|block| {
                !self.geometry.contains_key(&Patch {
                    origin: block.origin,
                    step: block.step,
                })
            }) {
                return Err(refuse(format!(
                    "terrain patch at {:?}, step {}, lies outside the patches made at load",
                    block.origin, block.step
                )));
            }
            cut.push((offset, blocks));
        }
        let columns = i64::from(self.map[0] / SIDE);
        let copy = |offset: f32| (offset / self.map[0] as f32).round() as i64;
        let steps = self.coasts.coast.is_ok().then(|| {
            let held: Vec<_> = cut
                .iter()
                .flat_map(|(offset, blocks)| {
                    blocks.iter().map(move |block| {
                        (
                            copy(*offset) * columns + i64::from(block.origin[0] / SIDE),
                            block.origin[1] / SIDE,
                            block.step,
                        )
                    })
                })
                .collect();
            super::terrain_coast::Steps::new(&held, self.map.map(|side| side / SIDE))
        });
        let positions = positions();
        let mut cache = BTreeMap::new();
        let last = self.last_rims.take();
        let mut block_rims = BTreeMap::new();
        let (mut rims, mut rules) = (Vec::new(), Vec::new());
        let mut views = Vec::with_capacity(cut.len());
        for (offset, blocks) in cut {
            let mut drawn = Vec::with_capacity(blocks.len());
            for block in blocks {
                let patch = Patch {
                    origin: block.origin,
                    step: block.step,
                };
                let own = if self.geometry[&patch].coast.is_some() {
                    Heights::Coast
                } else {
                    Heights::Sampled
                };
                let mut heights = own;
                if let (Some(steps), Ok(coast)) = (&steps, &self.coasts.coast) {
                    let cells = cells(self.map, patch);
                    let column = copy(offset) * columns + i64::from(patch.origin[0] / SIDE);
                    let row = patch.origin[1] / SIDE;
                    let kept = last.as_ref().and_then(|(was, blocks)| {
                        blocks
                            .get(&(patch, copy(offset)))
                            .filter(|_| steps.same_around(was, column, row, patch.step))
                    });
                    let rim = match kept {
                        Some(rim) => rim.clone(),
                        None if patch.step >= 2
                            && !steps.alone(column, row, patch.step)
                            && steps.rim(patch, cells, copy(offset), &mut rules) =>
                        {
                            let key = (patch, Arc::<[u8]>::from(rules.as_slice()));
                            let held = self.rim_cache.remove(&key);
                            let made = match held.or_else(|| cache.get(&key).cloned()) {
                                Some(made) => made,
                                None => coast
                                    .rim(
                                        steps,
                                        &positions,
                                        &self.bytes,
                                        self.map,
                                        patch,
                                        cells,
                                        copy(offset),
                                        self.coasts.capped(patch),
                                    )
                                    .map(|rimmed| Arc::<[u8]>::from(bytemuck::cast_slice(&rimmed))),
                            };
                            Some((key.1, made))
                        }
                        None => None,
                    };
                    if let Some((rules, made)) = &rim {
                        if let Some(made) = made {
                            rims.push(Rim {
                                rules: Arc::clone(rules),
                                heights: Arc::clone(made),
                            });
                            heights = Heights::Rim(rims.len() - 1);
                        }
                        cache.insert((patch, Arc::clone(rules)), made.clone());
                    }
                    block_rims.insert((patch, copy(offset)), rim);
                }
                drawn.push(Drawn {
                    patch,
                    snap: block.snap,
                    heights,
                });
            }
            views.push((offset.to_bits(), drawn));
        }
        self.last_rims = steps.map(|steps| (steps, block_rims));
        self.rim_cache = cache;
        let slots = self.place_rims(device, arena, &rims);
        let stream = self.rim_stream.clone().filter(|_| !rims.is_empty());
        let views = views
            .into_iter()
            .map(|(offset, mut drawn)| {
                for block in &mut drawn {
                    if let Heights::Rim(index) = &mut block.heights {
                        *index = slots[*index];
                    }
                }
                let kept = before.and_then(|before| kept_view(before, offset, &drawn, &self.rim_slots, &stream));
                (offset, kept.unwrap_or_else(|| Arc::from(drawn)))
            })
            .collect();
        Ok(Cut {
            cover: cover.clone(),
            views,
            rims: self.rim_slots.clone(),
            stream,
        })
    }

    fn place_rims(&mut self, device: &wgpu::Device, arena: Option<&gpu::Arena>, rims: &[Rim]) -> Vec<usize> {
        let key = |rim: &Rim| Arc::as_ptr(&rim.heights).cast::<u8>();
        let mut held: std::collections::HashMap<*const u8, usize> =
            self.rim_slots.iter().enumerate().map(|(slot, rim)| (key(rim), slot)).collect();
        let mut used = vec![false; self.rim_slots.len()];
        let mut slots = vec![usize::MAX; rims.len()];
        for (at, rim) in rims.iter().enumerate() {
            if let Some(&slot) = held.get(&key(rim)) {
                used[slot] = true;
                slots[at] = slot;
            }
        }
        let (mut free, mut written) = (0, Vec::new());
        for (at, rim) in rims.iter().enumerate() {
            if slots[at] != usize::MAX {
                continue;
            }
            if let Some(&slot) = held.get(&key(rim)).filter(|&&slot| used[slot]) {
                slots[at] = slot;
                continue;
            }
            while free < used.len() && used[free] {
                free += 1;
            }
            let placed = Rim { rules: Arc::clone(&rim.rules), heights: Arc::clone(&rim.heights) };
            let slot = if free < used.len() {
                self.rim_slots[free] = placed;
                used[free] = true;
                free
            } else {
                self.rim_slots.push(placed);
                used.push(true);
                self.rim_slots.len() - 1
            };
            held.insert(key(rim), slot);
            slots[at] = slot;
            written.push(slot);
        }
        let capacity = self.rim_stream.as_ref().map_or(0, |stream| stream.size() as usize / PATCH_BYTES);
        match (arena, &self.rim_stream) {
            (Some(arena), Some(stream)) if self.rim_slots.len() <= capacity => {
                for slot in written {
                    arena.write(stream, (slot * PATCH_BYTES) as u64, &self.rim_slots[slot].heights);
                }
            }
            _ if written.is_empty() && self.rim_slots.len() <= capacity => {}
            _ => {
                let capacity = self.rim_slots.len().max(capacity).next_power_of_two().max(16);
                let mut contents = vec![0u8; capacity * PATCH_BYTES];
                for (slot, rim) in self.rim_slots.iter().enumerate() {
                    contents[slot * PATCH_BYTES..][..rim.heights.len()].copy_from_slice(&rim.heights);
                }
                self.rim_stream = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("native terrain rim heights"),
                    contents: &contents,
                    usage: wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST,
                }));
            }
        }
        slots
    }
    fn views(&mut self, arena: &gpu::Arena, inputs: &Inputs<'_>) -> io::Result<Views> {
        let none = Arc::clone(&self.no_blocks);
        let cut = self.cut(arena.device(), Some(arena), inputs.cover)?;
        let mut views = Vec::with_capacity(inputs.offsets.len());
        for offset in inputs.offsets {
            let blocks = cut
                .views
                .iter()
                .find(|(view, _)| *view == offset.to_bits())
                .map_or_else(|| Arc::clone(&none), |(_, blocks)| Arc::clone(blocks));
            let mut matrix = inputs.matrix;
            for (value, axis) in matrix[3].iter_mut().zip(inputs.matrix[0]) {
                *value += axis * offset;
            }
            views.push((blocks, matrix));
        }
        Ok(views)
    }

    pub fn standing(
        &mut self,
        device: &wgpu::Device,
        cover: &Cover,
        offset: f32,
    ) -> io::Result<Vec<(wgpu::Buffer, Vec<Stand>)>> {
        self.cut(device, None, cover)?;
        let cut = self.cut.as_ref().expect("the cover was cut above");
        let rim_slab = self.slabs.len();
        let mut streams: Vec<(usize, Vec<Stand>)> = Vec::new();
        let blocks = cut.views.iter().find(|(view, _)| *view == offset.to_bits());
        for drawn in blocks.map_or(&[][..], |(_, blocks)| blocks) {
            let (slab, at) = stream(&self.geometry, rim_slab, drawn);
            let stand = Stand {
                origin: drawn.patch.origin,
                step: drawn.patch.step,
                snap: drawn.snap,
                base: u32::try_from(at / size_of::<[i16; 2]>() as u64)
                    .map_err(|_| refuse("a terrain slab stands past a stream of u32 vertices"))?,
            };
            match streams.iter_mut().find(|(held, _)| *held == slab) {
                Some((_, stands)) => stands.push(stand),
                None => streams.push((slab, vec![stand])),
            }
        }
        streams
            .into_iter()
            .map(|(slab, stands)| {
                let buffer = match self.slabs.get(slab) {
                    Some(buffer) => buffer,
                    None => cut
                        .stream
                        .as_ref()
                        .ok_or_else(|| refuse("a block draws rim heights the cut did not make"))?,
                };
                Ok((buffer.clone(), stands))
            })
            .collect()
    }

    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        mode: Mode,
        inputs: Inputs<'_>,
    ) -> io::Result<Prepared> {
        let pipeline = self.pipeline(arena.device(), mode)?;
        self.retained.retain(|kept| kept.used + 2 > arena.frame());
        let views = self.views(arena, &inputs)?;
        self.prepare_views(arena, mode, inputs, &pipeline, views)
    }

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
        for kept in &mut self.retained {
            if kept.built.mode == mode {
                kept.used = frame;
            }
        }
        self.retained.retain(|kept| kept.used + 2 > frame);
        let views = self.views(arena, &inputs)?;
        let samples = attachments(mode, self.samples).2;
        let reads = pipeline.constant_reads();
        let sampled = pipeline.sampled_slots().map(|physical| {
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
                self.cut.as_ref().map_or(&[][..], |cut| cut.rims.as_slice()),
            );
            let draws = keyed.draws(key);
            let record = |name| viewer_runtime::view_record::add("map scene", name, 1);
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

    pub fn submitted(&mut self) {
        for keyed in self.kept.values_mut() {
            keyed.submitted();
        }
    }

    pub fn lost(&mut self, mode: Mode) {
        if let Some(keyed) = self.kept.get_mut(&mode) {
            keyed.lost();
        }
    }

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
            views: views.iter().map(|(blocks, _)| ViewKey(Arc::clone(blocks))).collect(),
            textures: inputs.textures.iter().map(|view| (*view).clone()).collect(),
        };
        let at = match self
            .retained
            .iter()
            .position(|kept| kept.built == built && Arc::ptr_eq(&kept.pipeline, pipeline))
        {
            Some(at) => at,
            None => {
                let rim_slab = self.slabs.len();
                let mut slabs = self.slabs.clone();
                slabs.extend(self.cut.as_ref().and_then(|cut| cut.stream.clone()));
                let geometry = &self.geometry;
                let replaced = self.retained.iter().position(|kept| {
                    kept.built.mode == mode && kept.prepared < frame && Arc::ptr_eq(&kept.pipeline, pipeline)
                });
                let made = bind_retained(
                    arena,
                    pipeline,
                    built,
                    &views,
                    &inputs,
                    &textures,
                    &slabs,
                    geometry,
                    &|drawn| stream(geometry, rim_slab, drawn),
                    replaced.map(|at| &self.retained[at]),
                )?;
                match replaced {
                    Some(at) => {
                        self.retained[at] = made;
                        at
                    }
                    None => {
                        self.retained.push(made);
                        self.retained.len() - 1
                    }
                }
            }
        };
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
                let Some(&Drawn {
                    patch,
                    snap,
                    heights: drawn,
                }) = blocks.last()
                else {
                    continue;
                };
                for stage in [Stage::Vertex, Stage::Pixel] {
                    buffers.insert(binding(0, stage), scene.to_vec());
                    buffers.insert(binding(1, stage), constants(patch, *matrix, snap));
                }
                let mut tracked = retained.borrow_mut();
                self.programs[&mode].bind(&mut tracked, &buffers, &textures);
                let view = offset.to_bits();
                let coast = drawn == Heights::Coast;
                let rim = match (drawn, &self.cut) {
                    (Heights::Rim(index), Some(cut)) => Some(Arc::clone(&cut.rims[index].heights)),
                    _ => None,
                };
                let kept = rim.or_else(|| {
                    self.retained_heights.iter().find_map(|(v, kept, capped, heights)| {
                        (*v == view && *kept == patch && *capped == coast).then(|| heights.clone())
                    })
                });
                let heights = match kept {
                    Some(heights) => heights,
                    None => {
                        let capped = self.coasts.capped(patch).filter(|_| coast);
                        let made: Arc<[u8]> = match capped {
                            Some(capped) => bytemuck::cast_slice(capped).into(),
                            None => bytemuck::cast_slice(&heights(&self.bytes, self.map, patch)).into(),
                        };
                        self.retained_heights.retain(|(v, ..)| *v != view);
                        self.retained_heights.push((view, patch, coast, made.clone()));
                        made
                    }
                };
                tracked.bind_vertex(1, 4, heights);
            }
        }
        let kept = &mut self.retained[at];
        kept.used = frame;
        kept.prepared = frame;
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
    pub fn keep_views(&mut self, offsets: &[f32]) {
        self.retained_heights
            .retain(|(view, ..)| offsets.iter().any(|offset| offset.to_bits() == *view));
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, prepared: &Prepared) {
        let kept = &self.retained[prepared.at];
        viewer_runtime::view_record::add(
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
    rims: &[Rim],
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
            Some(Place::Frame) => {
                for frame in frames {
                    key.read(spec, frame);
                }
            }
            Some(Place::Grid) => {
                key.read(spec, bytemuck::bytes_of(light_grid));
            }
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
        for &Drawn {
            patch,
            snap,
            heights,
        } in blocks.iter()
        {
            key.number(u64::from(patch.origin[0]))
                .number(u64::from(patch.origin[1]))
                .number(u64::from(patch.step));
            match heights {
                Heights::Sampled => {
                    key.number(0);
                }
                Heights::Coast => {
                    key.number(1);
                }
                Heights::Rim(index) => {
                    let rules = &rims[index].rules;
                    key.number(2).number(rules.len() as u64);
                    for chunk in rules.chunks(8) {
                        let mut packed = [0u8; 8];
                        packed[..chunk.len()].copy_from_slice(chunk);
                        key.number(u64::from_le_bytes(packed));
                    }
                }
            }
            constants_into(&mut block, patch, *matrix, snap);
            for spec in &own {
                key.read(spec, &block);
            }
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "the draws, their blocks and the buffers they sit in")]
fn bind_retained(
    arena: &gpu::Arena,
    pipeline: &Arc<gpu::Pipeline>,
    built: Built,
    views: &[(Arc<[Drawn]>, [[f32; 4]; 4])],
    inputs: &Inputs<'_>,
    textures: &BTreeMap<u32, wgpu::TextureView>,
    slabs: &[wgpu::Buffer],
    geometry: &BTreeMap<Patch, Geometry>,
    stream: &dyn Fn(&Drawn) -> (usize, u64),
    replaced: Option<&Retained>,
) -> io::Result<Retained> {
    let device = arena.device();
    let mode = built.mode;
    let empty = |built: Built, buffer: wgpu::Buffer| Retained {
        pipeline: Arc::clone(pipeline),
        bases: buffer.clone(),
        buffer,
        bytes: Vec::new(),
        programs: Vec::new(),
        frames: Vec::new(),
        grid: None,
        draws: Vec::new(),
        places: Vec::new(),
        view_blocks: views.iter().map(|_| 0..0).collect(),
        matrices: views.iter().map(|(_, matrix)| *matrix).collect(),
        bundle: None,
        still: 0,
        used: 0,
        prepared: 0,
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
    let mut groups: Vec<(usize, usize, wgpu::Buffer, u32, Vec<Drawn>)> = Vec::new();
    let mut bases: Vec<u32> = Vec::new();
    for (view, (blocks, _)) in views.iter().enumerate() {
        for &drawn in blocks.iter() {
            let made = &geometry[&drawn.patch];
            let (slab, _) = stream(&drawn);
            match groups.iter_mut().find(|(at, held, indices, ..)| {
                *at == view && *held == slab && *indices == made.indices
            }) {
                Some((.., patches)) => patches.push(drawn),
                None => groups.push((view, slab, made.indices.clone(), made.count, vec![drawn])),
            }
        }
    }
    let mut group_bases = Vec::with_capacity(groups.len());
    for (_, _, _, _, patches) in &groups {
        group_bases.push(bases.len() as u32);
        for drawn in patches {
            let (_, offset) = stream(drawn);
            bases.push(
                u32::try_from(offset / size_of::<[i16; 2]>() as u64)
                    .map_err(|_| refuse("a terrain slab stands past a stream of u32 vertices"))?,
            );
        }
    }
    let base_bytes = (bases.len() * 4).max(4) as u64;
    let bases_buffer = match replaced.filter(|replaced| replaced.bases.size() >= base_bytes) {
        Some(replaced) => replaced.bases.clone(),
        None => device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native terrain instance slabs"),
            size: base_bytes.next_power_of_two(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    };
    let held = replaced.filter(|replaced| replaced.bases == bases_buffer && replaced.built.textures == built.textures);
    let mut programs: Vec<(usize, gpu::BindingProgram, Option<gpu::Bindings>)> = Vec::new();
    for &(_, slab, ..) in &groups {
        if programs.iter().any(|(held, ..)| *held == slab) {
            continue;
        }
        let kept = held.and_then(|held| held.programs.iter().find(|(bound, ..)| *bound == slabs[slab]));
        programs.push(match kept {
            Some((_, program, bound)) => (slab, program.clone(), Some(bound.clone())),
            None => {
                let program = pipeline
                    .rows_program(
                        arena,
                        |read| textures.get(&read),
                        Some([slabs[slab].clone(), bases_buffer.clone()]),
                    )
                    .map_err(refuse)?;
                (slab, program, None)
            }
        });
    }
    let program = &programs.first().expect("a group was made above").1;
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
            for (index, &Drawn { patch, snap, .. }) in patches.iter().enumerate() {
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
    let size = bytes.len() as u64;
    let buffer = match replaced.filter(|replaced| replaced.buffer.size() >= size) {
        Some(replaced) => replaced.buffer.clone(),
        None => device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native terrain constants"),
            size: size.next_power_of_two().min(device.limits().max_buffer_size).max(size),
            usage: wgpu::BufferUsages::UNIFORM
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
    };
    if !replaced.is_some_and(|replaced| replaced.buffer == buffer) {
        for (.., bound) in &mut programs {
            *bound = None;
        }
    }
    let mut draws = Vec::with_capacity(groups.len());
    for ((view, slab, indices, count, patches), (block, arrays, base)) in
        groups.iter().zip(&laid)
    {
        let (_, program, bound) = programs
            .iter_mut()
            .find(|(held, ..)| held == slab)
            .expect("a program was made for every slab");
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
        let bindings = match bound {
            Some(bound) => program.rebind_rows(bound, &blocks, *block as u32),
            None => program.bind_rows(arena, &buffer, &blocks, *block as u32),
        }
        .map_err(refuse)?;
        bound.get_or_insert_with(|| bindings.clone());
        draws.push(Keep {
            indices: indices.clone(),
            count: *count,
            instances: patches.len() as u32,
            bindings,
        });
    }
    arena.write(&bases_buffer, 0, bytemuck::cast_slice(&bases));
    arena.write(&buffer, 0, &bytes);
    Ok(Retained {
        frames,
        grid,
        draws,
        places,
        view_blocks,
        bytes,
        bases: bases_buffer,
        programs: programs
            .into_iter()
            .filter_map(|(slab, program, bound)| Some((slabs[slab].clone(), program, bound?)))
            .collect(),
        ..empty(built, buffer)
    })
}

fn write_block(data: &mut [u8], place: &Placed, matrix: [[f32; 4]; 4]) {
    let mut block = [0u8; BLOCK_BYTES];
    constants_into(&mut block, place.patch, matrix, place.snap);
    data.copy_from_slice(&block[..data.len()]);
}

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

    struct Pass {
        keyed: Keyed,
        reads: Vec<shader_pipeline::Buffer>,
        sampled: Vec<(u32, Option<Content>)>,
        into: Option<Content>,
        scene: Vec<u8>,
        light_grid: [f32; 4],
        views: Views,
        rims: Vec<Rim>,
    }

    impl Pass {
        fn new() -> Self {
            let drawn = |origin| Drawn {
                patch: Patch { origin, step: 1 },
                snap: [false; 4],
                heights: Heights::Sampled,
            };
            let blocks: Arc<[Drawn]> = vec![drawn([0, 0]), drawn([64, 0])].into();
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
                rims: Vec::new(),
            }
        }

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
                &self.rims,
            );
            let draws = self.keyed.draws(key);
            if draws {
                self.keyed.submitted();
            }
            draws
        }
    }

    #[test]
    fn a_changed_read_byte_redraws_the_sea_bed_and_an_unread_byte_does_not() {
        let mut pass = Pass::new();
        assert!(pass.draws(), "the target holds nothing yet");
        assert!(!pass.draws(), "nothing changed");
        pass.scene[236] = 9;
        pass.scene[351] = 9;
        assert!(!pass.draws(), "the clock moved, and no program reads it");
        pass.scene[4] = 9;
        assert!(pass.draws(), "the camera moved");
        assert!(!pass.draws());
        let matrix = pass.views[0].1;
        let mut moved: Vec<_> = pass.views[0].0.to_vec();
        moved[1].snap = [true, false, false, false];
        pass.views = vec![(moved.into(), matrix)];
        assert!(!pass.draws(), "a snap switch no program reads");
    }

    #[test]
    fn a_changed_block_list_redraws_the_sea_bed() {
        let mut pass = Pass::new();
        assert!(pass.draws());
        let matrix = pass.views[0].1;
        let mut more: Vec<_> = pass.views[0].0.to_vec();
        more.push(Drawn {
            patch: Patch {
                origin: [128, 0],
                step: 1,
            },
            snap: [false; 4],
            heights: Heights::Sampled,
        });
        pass.views = vec![(more.clone().into(), matrix)];
        assert!(pass.draws(), "a block more");
        more.pop();
        pass.views = vec![(more.into(), matrix)];
        assert!(pass.draws(), "a block fewer");
        let mut coarser: Vec<_> = pass.views[0].0.to_vec();
        coarser[0].patch.step = 2;
        pass.views = vec![(coarser.clone().into(), matrix)];
        assert!(pass.draws(), "a block at another step draws other vertices");
        assert!(!pass.draws());
        coarser[0].heights = Heights::Coast;
        pass.views = vec![(coarser.clone().into(), matrix)];
        assert!(pass.draws(), "a block's coast heights are other vertices");
        assert!(!pass.draws());
        let rim = |rules: &[u8]| Rim {
            rules: rules.into(),
            heights: Arc::from([0u8; 4]),
        };
        pass.rims = vec![rim(&[1, 1, 2, 1])];
        coarser[0].heights = Heights::Rim(0);
        pass.views = vec![(coarser.clone().into(), matrix)];
        assert!(pass.draws(), "a rim matched to its neighbours is other vertices");
        assert!(!pass.draws());
        pass.rims = vec![rim(&[1, 2, 2, 1])];
        assert!(pass.draws(), "a rim matched to other neighbours is other vertices again");
        assert!(!pass.draws());
        coarser[0].heights = Heights::Sampled;
        pass.views = vec![(coarser.into(), matrix)];
        assert!(pass.draws(), "and its sampled heights again");
    }

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
    fn a_view_that_draws_the_same_blocks_from_the_same_rims_keeps_its_list_across_cuts() {
        let rim = |byte: u8| Rim { rules: Arc::from([0u8]), heights: Arc::from([byte; 4]) };
        let block = |x: u32, heights| Drawn { patch: Patch { origin: [x * SIDE, 0], step: 2 }, snap: [false; 4], heights };
        let near: Arc<[Drawn]> = Arc::from([block(0, Heights::Rim(0)), block(2, Heights::Sampled)]);
        let far: Arc<[Drawn]> = Arc::from([block(4, Heights::Coast)]);
        let slots = vec![rim(7), rim(9)];
        let before = Cut {
            cover: Cover::Blocks(Vec::new()),
            views: vec![(0, Arc::clone(&near)), (1, Arc::clone(&far))],
            rims: slots.clone(),
            stream: None,
        };
        let kept = kept_view(&before, 0, &near, &slots, &None).expect("the same blocks and rims");
        assert!(Arc::ptr_eq(&kept, &near));
        assert!(Arc::ptr_eq(&kept_view(&before, 1, &far, &slots, &None).unwrap(), &far));
        let moved = [block(0, Heights::Rim(0)), block(2, Heights::Coast)];
        assert!(kept_view(&before, 0, &moved, &slots, &None).is_none(), "a changed block");
        let refilled = vec![rim(7), slots[1].clone()];
        assert!(kept_view(&before, 0, &near, &refilled, &None).is_none(), "another rim in the slot");
        assert!(kept_view(&before, 1, &far, &refilled, &None).is_some(), "a view without rims ignores them");
        assert!(kept_view(&before, 2, &far, &slots, &None).is_none(), "a view the cut before had not");
    }

    #[test]
    fn a_cover_change_after_a_still_frame_binds_into_the_groups_it_replaces() {
        use shader_pipeline::{Buffer, Compiled, Shader, VertexInput};

        content_reader_script::test_game::enter();
        let program = content_reader_gui::pdxshader::program::read_with(b"terrain.shader", &|path| {
            Some(if path == b"terrain.shader" {
                b"Effect terrainunlit { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let shader = |wgsl: &str| Shader {
            wgsl: wgsl.into(),
            independent_draws: false,
            buffers: Vec::new(),
            textures: Vec::new(),
            samplers: Vec::new(),
            sampling_pairs: Vec::new(),
            inputs: Vec::new(),
            clock: Default::default(),
        };
        let mut vertex = shader(r#"
            struct Constants { values: vec4<f32> }
            @group(0) @binding(0) var<uniform> frame: Constants;
            @group(0) @binding(2) var<uniform> tile: Constants;
            @vertex fn main(@location(0) xy: vec4<i32>, @location(1) height: vec2<i32>)
                -> @builtin(position) vec4<f32> {
                return frame.values + tile.values + vec4<f32>(vec2<f32>(xy.xy), f32(height.x), 1.0);
            }
        "#);
        vertex.buffers = [0, 1]
            .into_iter()
            .map(|slot| Buffer {
                binding: binding(slot, Stage::Vertex),
                size: 16,
                reads: std::iter::once(0..16).collect(),
            })
            .collect();
        vertex.inputs = [(0, "POSITION", 4), (1, "TEXCOORD0", 2)]
            .into_iter()
            .map(|(location, semantic, components)| VertexInput {
                kind: ScalarKind::Sint,
                location,
                semantic: semantic.into(),
                components,
            })
            .collect();
        let pixel = shader("@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }");
        let effect = Effect::from_compiled(
            Arc::new(program),
            Vec::new(),
            vec![Ok(Arc::new(Compiled { vertex, pixel }))],
        );
        let map = [512, 256];
        let (_serial, device, _queue) = viewer_gpu::test_support::gpu::device();
        let arena = gpu::Arena::new(&device);
        let bytes = vec![200u8; (map[0] * map[1]) as usize];
        let mut terrain = Terrain::new(&device, &effect, &bytes, map, 1, &Workers::default()).unwrap();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let scene = [0u8; 16];
        let row = |z: u32| {
            Cover::Blocks((0..4).map(|x| Block { origin: [x * 2 * SIDE, z], step: 2, offset: 0.0, snap: [false; 4] }).collect())
        };
        let mut prepare = |cover: &Cover| {
            arena.reset();
            let inputs = Inputs {
                retained: None,
                frames: &[&scene],
                offsets: &[0.0],
                cover,
                matrix: [[0.0; 4]; 4],
                light_grid: [0.0; 4],
                textures: [&view; 16],
            };
            let prepared = terrain.prepare(&arena, Mode::Receiver, inputs).unwrap();
            terrain.retained[prepared.at].draws.iter().map(|keep| keep.bindings.clone()).collect::<Vec<_>>()
        };
        let (south, north) = (row(0), row(2 * SIDE));
        let bound = prepare(&south);
        let kept = prepare(&south);
        assert!(!bound.is_empty());
        assert_eq!(bound.len(), kept.len());
        assert!(bound.iter().zip(&kept).all(|(a, b)| a.same_groups(b)));
        let moved = prepare(&north);
        assert_eq!(moved.len(), bound.len());
        assert!(
            bound.iter().zip(&moved).all(|(a, b)| a.same_groups(b)),
            "a cover change after a still frame binds into the groups of the entry it replaces"
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

    #[test]
    fn coast_caps_lie_under_the_water_of_the_effect_drawn() {
        content_reader_script::test_game::enter();
        let program = |water: &str| {
            let text = format!("Code [[ static const float WATER_HEIGHT = {water}; ]]");
            Arc::new(
                content_reader_gui::pdxshader::program::read_with(b"terrain.shader", &|path| {
                    Some(if path == b"terrain.shader" {
                        text.clone().into_bytes()
                    } else {
                        Vec::new()
                    })
                })
                .unwrap(),
            )
        };
        let recorded = Effect::from_compiled(program("9.5f"), Vec::new(), Vec::new());
        let modded = Effect::replacing(program("2.0f"), Vec::new(), recorded);
        let map = [128, 64];
        let mut bytes = vec![10u8; 128 * 64];
        for row in bytes.chunks_mut(128) {
            row[40..].fill(200);
        }
        let patches: Vec<Patch> = every_patch(map).collect();
        let workers = Workers::default();
        let lowest = |coasts: &Coasts| {
            assert!(coasts.coast.is_ok(), "{:?}", coasts.coast.as_ref().err());
            let capped: Vec<i16> = coasts
                .heights
                .iter()
                .flat_map(|(_, heights)| heights.iter().flatten().copied())
                .filter(|&height| height != 100 && height != 2000)
                .collect();
            assert!(!capped.is_empty());
            capped
        };
        let coasts = Coasts::new(&modded, &patches, &bytes, map, &workers);
        assert!(lowest(&coasts).iter().all(|&height| height == 190), "under the mod's 2.0");
        let drawn = modded.fall_back("its terrain cannot be drawn here").unwrap();
        assert!(coasts.follows(&modded) && !coasts.follows(&drawn));
        let coasts = Coasts::new(&drawn, &patches, &bytes, map, &workers);
        assert!(coasts.follows(&drawn));
        assert!(lowest(&coasts).iter().all(|&height| height == 940), "under the recorded 9.5");
    }
}

pub fn terrain_readers(high: bool) -> [Mode; 2] {
    [Mode::Underwater, Mode::Lit(high)]
}

#[cfg(test)]
mod shadow_tests;
