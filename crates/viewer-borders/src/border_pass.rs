use viewer_gpu::gpu_contract::Targets;
use std::{
    collections::{BTreeMap, HashSet},
    io,
    sync::Arc,
};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_map::{adjacencies::{Adjacency, IMPASSABLE}, map::{MapData, ProvinceKind}};
use content_reader_image::native::Decode;
use content_reader_regions::regions::StrategicRegion;
use content_reader_states::states::StateDefinition;

use super::{borders::{self, BorderClass, BorderDraw, BorderMesh, BorderMeshes}, demilitarized::Zone};
use map_display_data::landscape::Regions;
use viewer_gpu::{
    gpu_contract::{DEPTH_FORMAT, buffer, invalid},
    province_map::WATER_HEIGHT,
    textures::{ColorSpace, DeferredTexture},
};

pub const STRAIT_WIDTH: f32 = 4.0;

const BORDER_VERTEX_STRIDE: u64 = 16;
use super::strait_shader::StraitVertex;

pub struct BorderAssets {
    pub border_shader: Arc<shader_pipeline::library::Effect>,
    pub strait_shader: Arc<shader_pipeline::library::Effect>,
    pub classes: Vec<DeferredTexture>,
    pub strait: DeferredTexture,
    pub demilitarized_color: [f32; 4],
    pub faction_highlight_color: Result<[f32; 4], String>,
    pub adjacency_highlight_color: Result<[f32; 4], String>,
    pub selection_colors: Result<[[f32; 4]; 2], String>,
}

impl BorderAssets {
    pub fn load(
        layers: &ResolvedLayers,
        defines: &content_reader_defines::defines::Defines,
        map: [u32; 2],
        texture_quality: u8,
        decode: Decode,
    ) -> io::Result<Self> {
        let channel = |name: &str| -> io::Result<f32> {
            Ok(defines.number("NGraphics", &format!("BORDER_COLOR_DEMILITARIZED_{name}"))? as f32)
        };
        let texture = |path: String, space: ColorSpace| -> io::Result<DeferredTexture> {
            DeferredTexture::load_artwork(layers, path, decode, space)
        };
        Ok(Self {
            border_shader: super::border_shader::load(layers, map, decode)?,
            strait_shader: super::strait_shader::load(layers, map, decode)?,
            classes: BorderClass::ALL
                .iter()
                .map(|class| {
                    texture(
                        format!(
                            "map/terrain/border_{}_{texture_quality}.dds",
                            class.texture_name()
                        ),
                        ColorSpace::Srgb,
                    )
                })
                .collect::<io::Result<Vec<_>>>()?,
            strait: texture("map/terrain/strait.dds".into(), ColorSpace::Linear)?,
            demilitarized_color: [channel("R")?, channel("G")?, channel("B")?, channel("A")?],
            faction_highlight_color: faction_highlight_color(defines)
                .map_err(|why| why.to_string()),
            adjacency_highlight_color: (|| -> io::Result<_> {
                let channel = |name| defines.number("NGraphics", &format!("BORDER_COLOR_SELECTION_ADJACENCY_RULE_AREA_{name}")).map(|value| value as f32);
                Ok([channel("R")?, channel("G")?, channel("B")?, channel("A")?])
            })().map_err(|why| why.to_string()),
            selection_colors: (|| -> io::Result<_> {
                let color = |what: &str| -> io::Result<[f32; 4]> {
                    let channel = |name| defines.number("NGraphics", &format!("BORDER_COLOR_SELECTION_{what}_{name}")).map(|value| value as f32);
                    Ok([channel("R")?, channel("G")?, channel("B")?, channel("A")?])
                };
                Ok([color("PROVINCE")?, color("STATE")?])
            })().map_err(|why| why.to_string()),
        })
    }
}

fn faction_highlight_color(defines: &content_reader_defines::defines::Defines) -> io::Result<[f32; 4]> {
    let index = defines.int32("NMapMode", "FACTION_THEATER_HIGHLIGHT_COLOR_INDEX")?;
    if !(0..=i32::from(u8::MAX) - 10).contains(&index) {
        return Err(invalid(
            "FACTION_THEATER_HIGHLIGHT_COLOR_INDEX exceeds the native reason byte",
        ));
    }
    let start = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_mul(4))
        .ok_or_else(|| {
            invalid("FACTION_THEATER_HIGHLIGHT_COLOR_INDEX is outside the custom palette")
        })?;
    let colors = defines.numbers("NGraphics", "BORDER_COLOR_CUSTOM_HIGHLIGHTS")?;
    let rgba = colors.get(start..start.saturating_add(4)).ok_or_else(|| {
        invalid("FACTION_THEATER_HIGHLIGHT_COLOR_INDEX names no complete custom RGBA quad")
    })?;
    Ok(std::array::from_fn(|channel| rgba[channel] as f32))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    water: Vec<bool>,
    hidden: Vec<bool>,
    region_of: Vec<u32>,
    impassable: Vec<bool>,
    impassable_pairs: HashSet<(u32, u32)>,
}

impl Topology {
    pub fn new(
        map: &MapData,
        states: &BTreeMap<u32, StateDefinition>,
        regions: &BTreeMap<u32, StrategicRegion>,
        adjacencies: &[Adjacency],
    ) -> io::Result<Self> {
        let largest = map
            .definitions
            .last_key_value()
            .map(|(&id, _)| id)
            .unwrap_or(0)
            .max(map.province_ids.iter().copied().max().unwrap_or(0));
        let count = usize::try_from(largest)
            .ok()
            .and_then(|id| id.checked_add(1))
            .ok_or_else(|| invalid("province ids exceed address space"))?;
        let mut topology = Self {
            water: vec![false; count],
            hidden: vec![false; count],
            region_of: vec![0; count],
            impassable: vec![false; count],
            impassable_pairs: HashSet::new(),
        };
        for (&id, definition) in &map.definitions {
            if let Some(slot) = topology.water.get_mut(id as usize) {
                *slot = matches!(definition.kind, ProvinceKind::Sea | ProvinceKind::Lake);
            }
        }
        for region in regions.values() {
            for &province in &region.provinces {
                if let Some(slot) = topology.region_of.get_mut(province as usize) {
                    *slot = region.id;
                }
            }
        }
        for state in states.values().filter(|state| state.impassable) {
            for &province in &state.provinces {
                if let Some(slot) = topology.impassable.get_mut(province as usize) {
                    *slot = true;
                }
            }
        }
        let mut kinds = BTreeMap::new();
        for adjacency in adjacencies {
            kinds.insert(pair(adjacency.from, adjacency.to), adjacency.kind);
        }
        topology.impassable_pairs = kinds
            .into_iter()
            .filter(|&(_, kind)| kind == IMPASSABLE)
            .map(|(pair, _)| pair)
            .collect();
        topology.hidden = hidden_provinces(map, count);
        Ok(topology)
    }

    pub fn is_water(&self, province: u32) -> bool {
        self.water(province)
    }

    fn water(&self, province: u32) -> bool {
        self.water.get(province as usize).copied().unwrap_or(false)
    }

    fn hidden(&self, province: u32) -> bool {
        self.hidden.get(province as usize).copied().unwrap_or(false)
    }

    fn region(&self, province: u32) -> u32 {
        self.region_of
            .get(province as usize)
            .copied()
            .unwrap_or_default()
    }

    fn impassable(&self, a: u32, b: u32) -> bool {
        let flagged = |province: u32| {
            self.impassable
                .get(province as usize)
                .copied()
                .unwrap_or(false)
        };
        flagged(a) != flagged(b) || self.impassable_pairs.contains(&pair(a, b))
    }

    pub fn static_class(&self, a: u32, b: u32) -> Option<BorderClass> {
        if a == b || self.hidden(a) || self.hidden(b) {
            return None;
        }
        if self.water(a) || self.water(b) {
            if !(self.water(a) && self.water(b)) {
                return None;
            }
            if self.region(a) != self.region(b) {
                return Some(BorderClass::SeaRegion);
            }
            return Some(BorderClass::Sea);
        }
        if self.impassable(a, b) {
            return Some(BorderClass::Impassable);
        }
        None
    }

    pub fn trace_class(&self, a: u32, b: u32) -> Option<BorderClass> {
        if a == b || self.hidden(a) || self.hidden(b) || self.water(a) != self.water(b) {
            return None;
        }
        Some(self.static_class(a, b).unwrap_or(BorderClass::Province))
    }
}

fn hidden_provinces(map: &MapData, count: usize) -> Vec<bool> {
    let mut hidden = vec![false; count];
    for (&id, definition) in &map.definitions {
        hidden[id as usize] = id != 0 && definition.kind == ProvinceKind::Land;
    }
    let land = |id| {
        id != 0
            && map
                .definitions
                .get(&id)
                .is_some_and(|definition| definition.kind == ProvinceKind::Land)
    };
    let mut connect = |a: u32, b: u32| {
        if a != b && land(a) && land(b) {
            hidden[a as usize] = false;
            hidden[b as usize] = false;
        }
    };
    let width = map.width as usize;
    if width != 0 {
        for (y, row) in map.province_ids.chunks_exact(width).enumerate() {
            for (x, &a) in row.iter().enumerate() {
                let right = row[(x + 1) % width];
                connect(a, right);
                if y + 1 < map.height as usize {
                    let below = map.province_ids[(y + 1) * width + x];
                    let diagonal = map.province_ids[(y + 1) * width + (x + 1) % width];
                    connect(a, below);
                    if [a, right, below, diagonal].iter().all(|&id| id != 0)
                        && a != right
                        && a != below
                        && a != diagonal
                        && right != below
                        && right != diagonal
                        && below != diagonal
                    {
                        connect(right, below);
                    }
                }
            }
        }
    }
    hidden
}

pub fn land_class(regions: &Regions, a: u32, b: u32) -> BorderClass {
    let key = |province: u32| {
        regions
            .owner_of
            .get(province as usize)
            .copied()
            .unwrap_or_default()
    };
    let state = |province: u32| {
        regions
            .state_of
            .get(province as usize)
            .copied()
            .unwrap_or_default()
    };
    if key(a) != key(b) {
        BorderClass::Country
    } else if state(a) != state(b) {
        BorderClass::State
    } else {
        BorderClass::Province
    }
}

fn pair(a: u32, b: u32) -> (u32, u32) {
    if a <= b { (a, b) } else { (b, a) }
}

fn authored_end(end: (i32, i32)) -> Option<(i32, i32)> {
    (end.0 >= 0 && end != (0, 0)).then_some(end)
}

#[derive(Clone, Copy)]
struct Record {
    neighbour: u32,
    through: i32,
    end: (i32, i32),
}

fn neighbour_records(adjacencies: &[Adjacency], raster: &[Vec<u32>]) -> BTreeMap<u32, Vec<Record>> {
    let mut lists: BTreeMap<u32, Vec<Record>> = BTreeMap::new();
    for row in adjacencies {
        for province in [row.from, row.to] {
            lists.entry(province).or_insert_with(|| {
                raster
                    .get(province as usize)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .map(|&neighbour| Record {
                        neighbour,
                        through: 0,
                        end: (0, 0),
                    })
                    .collect()
            });
        }
        let (from, to) = (row.from, row.to);
        let listed = lists[&from].iter().any(|record| record.neighbour == to);
        let mut write = |owner: u32, neighbour: u32, end: (i32, i32)| {
            let list = lists.get_mut(&owner).expect("seeded above");
            let record = Record {
                neighbour,
                through: row.through,
                end,
            };
            if listed {
                for existing in list.iter_mut().filter(|r| r.neighbour == neighbour) {
                    *existing = record;
                }
            } else if !list.iter().any(|r| r.neighbour == neighbour) {
                list.push(record);
            }
        };
        if listed {
            write(from, to, row.stop);
            write(to, from, row.start);
        } else {
            write(from, to, row.start);
            write(to, from, row.stop);
        }
    }
    lists
}

pub fn strait_quads(
    adjacencies: &[Adjacency],
    map: &MapData,
    land: &dyn Fn(u32) -> bool,
) -> io::Result<Vec<StraitVertex>> {
    let raster = content_reader_map::movement::raster_neighbours(
        map.width as usize,
        map.height as usize,
        &map.province_ids,
    )?;
    let lists = neighbour_records(adjacencies, &raster);
    let kept: Vec<(u32, Record)> = lists
        .iter()
        .filter(|&(&owner, _)| land(owner))
        .flat_map(|(&owner, list)| list.iter().map(move |&record| (owner, record)))
        .filter(|&(owner, record)| record.neighbour <= owner && record.through != 0)
        .collect();
    let ends = |owner: u32, record: Record| {
        let back = lists
            .get(&record.neighbour)
            .and_then(|list| list.iter().find(|r| r.neighbour == owner))
            .and_then(|r| authored_end(r.end));
        (authored_end(record.end), back)
    };
    let needed = kept
        .iter()
        .filter(|&&(owner, record)| {
            let (a, b) = ends(owner, record);
            a.is_none() || b.is_none()
        })
        .flat_map(|&(owner, record)| [owner, record.neighbour])
        .collect::<HashSet<_>>();
    let spans = boundary_spans(map, &needed)?;
    let mut vertices = Vec::new();
    for (high, record) in kept {
        let low = record.neighbour;
        let (a, b) = match ends(high, record) {
            (Some(a), Some(b)) => (a, b),
            (a, b) => {
                let Some((near_a, near_b)) = nearest_pixels(&spans, high, low) else {
                    continue;
                };
                (a.unwrap_or(near_a), b.unwrap_or(near_b))
            }
        };
        let (ax, az) = (a.0 as f32, a.1 as f32);
        let (bx, bz) = (b.0 as f32, b.1 as f32);
        let (dx, dz) = (bx - ax, bz - az);
        let length = (dx * dx + dz * dz).sqrt();
        if length <= 0.0 {
            continue;
        }
        let (nx, nz) = (dz / length, -dx / length);
        let half = STRAIT_WIDTH * 0.5;
        let tiles = (length / STRAIT_WIDTH).ceil();
        let corner = |x: f32, z: f32, side: f32, u: f32, v: f32| {
            let (cx, cz) = (x + nx * half * side, z + nz * half * side);
            StraitVertex {
                position: [cx, WATER_HEIGHT, cz],
                uv: [u, v],
            }
        };
        let v0 = corner(ax, az, 1.0, 0.0, 0.0);
        let v1 = corner(ax, az, -1.0, 1.0, 0.0);
        let v2 = corner(bx, bz, 1.0, 0.0, tiles);
        let v3 = corner(bx, bz, -1.0, 1.0, tiles);
        vertices.extend_from_slice(&[v0, v1, v3, v0, v3, v2]);
    }
    Ok(vertices)
}

struct Span {
    z: i32,
    xs: Vec<i32>,
}

fn boundary_spans(map: &MapData, needed: &HashSet<u32>) -> io::Result<BTreeMap<u32, Vec<Span>>> {
    let (width, height) = (map.width as usize, map.height as usize);
    if map.province_ids.len() != width * height {
        return Err(invalid("the province image does not match its dimensions"));
    }
    let mut spans: BTreeMap<u32, Vec<Span>> = BTreeMap::new();
    if needed.is_empty() {
        return Ok(spans);
    }
    let at = |x: i64, z: i64| -> Option<u32> {
        (x >= 0 && z >= 0 && (x as usize) < width && (z as usize) < height)
            .then(|| map.province_ids[(height - 1 - z as usize) * width + x as usize])
    };
    for z in 0..height as i64 {
        let mut x = 0i64;
        while (x as usize) < width {
            let id = at(x, z).unwrap_or(0);
            let mut end = x + 1;
            while (end as usize) < width && at(end, z) == Some(id) {
                end += 1;
            }
            if needed.contains(&id) {
                let xs = (x..end)
                    .filter(|&px| {
                        ![(px - 1, z), (px + 1, z), (px, z - 1), (px, z + 1)]
                            .iter()
                            .all(|&(nx, nz)| at(nx, nz) == Some(id))
                    })
                    .map(|px| px as i32)
                    .collect();
                spans.entry(id).or_default().push(Span { z: z as i32, xs });
            }
            x = end;
        }
    }
    Ok(spans)
}

fn nearest_pixels(
    spans: &BTreeMap<u32, Vec<Span>>,
    first: u32,
    second: u32,
) -> Option<((i32, i32), (i32, i32))> {
    let (a, b) = (spans.get(&first)?, spans.get(&second)?);
    let mut best = (i64::MAX, None);
    for sa in a {
        for sb in b {
            let dz = i64::from(sa.z - sb.z);
            for &xa in &sa.xs {
                for &xb in &sb.xs {
                    let dx = i64::from(xa - xb);
                    let d = dx * dx + dz * dz;
                    if d < best.0 {
                        best = (d, Some(((xa, sa.z), (xb, sb.z))));
                    }
                }
            }
        }
    }
    best.1
}

fn chunk_bytes(chunk: &[borders::Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(chunk.len() * BORDER_VERTEX_STRIDE as usize);
    for vertex in chunk {
        for value in vertex.position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in vertex.uv {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

pub(super) fn run_indices(base_vertex: u32, points: u32, out: &mut Vec<u32>) {
    for segment in 0..points.saturating_sub(1) {
        let base = base_vertex + segment * 2;
        out.extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);
    }
}

fn draw_indices(draw: &BorderDraw, fine: &mut Vec<u32>, coarse: &mut Vec<u32>) {
    run_indices(draw.base_vertex, draw.vertex_count / 2, fine);
    run_indices(draw.coarse_base_vertex, draw.coarse_vertex_count / 2, coarse);
}

fn joined(mut fine: Vec<u32>, coarse: Vec<u32>) -> (u32, Vec<u32>) {
    let count = fine.len() as u32;
    fine.extend(coarse);
    (count, fine)
}

fn per_chunk(chunks: usize, draws: impl Iterator<Item = BorderDraw>) -> BTreeMap<usize, (u32, Vec<u32>)> {
    let mut lists: BTreeMap<usize, (Vec<u32>, Vec<u32>)> = BTreeMap::new();
    for draw in draws {
        if (draw.chunk as usize) >= chunks {
            continue;
        }
        let (fine, coarse) = lists.entry(draw.chunk as usize).or_default();
        draw_indices(&draw, fine, coarse);
    }
    lists
        .into_iter()
        .filter(|(_, (fine, _))| !fine.is_empty())
        .map(|(chunk, (fine, coarse))| (chunk, joined(fine, coarse)))
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassPlan {
    runs: Vec<Vec<PlannedRun>>,
}

#[derive(Clone, Debug, PartialEq)]
struct PlannedRun {
    source: usize,
    chunk: usize,
    fine: u32,
    indices: Arc<Vec<u32>>,
}

impl ClassPlan {
    pub fn new(meshes: &BorderMeshes, regions: &Regions, zone: &Zone) -> Self {
        Self::matching(meshes, regions, |a, b| !zone.splits(a, b))
    }

    pub fn matching(
        meshes: &BorderMeshes,
        regions: &Regions,
        keep: impl Fn(u32, u32) -> bool,
    ) -> Self {
        let kept = |draw: &BorderDraw| keep(draw.province_a, draw.province_b);
        let draws = |bucket: usize| {
            meshes
                .meshes
                .get(bucket)
                .map(|mesh| mesh.draws.as_slice())
                .unwrap_or_default()
                .iter()
                .copied()
        };
        let runs = BorderClass::ALL
            .iter()
            .map(|&class| match class {
                BorderClass::Province | BorderClass::State | BorderClass::Country => Self::runs(
                    meshes,
                    BorderClass::Province,
                    draws(BorderClass::Province.bucket()).filter(|draw| {
                        kept(draw) && land_class(regions, draw.province_a, draw.province_b) == class
                    }),
                ),
                _ => Self::runs(meshes, class, draws(class.bucket()).filter(kept)),
            })
            .collect();
        Self { runs }
    }

    fn runs(
        meshes: &BorderMeshes,
        source: BorderClass,
        draws: impl Iterator<Item = BorderDraw>,
    ) -> Vec<PlannedRun> {
        let chunks = meshes
            .meshes
            .get(source.bucket())
            .map_or(0, |mesh| mesh.chunks.len());
        per_chunk(chunks, draws)
            .into_iter()
            .map(|(chunk, (fine, indices))| PlannedRun {
                source: source.bucket(),
                chunk,
                fine,
                indices: Arc::new(indices),
            })
            .collect()
    }

    pub fn chunk_draws(meshes: &BorderMeshes) -> Vec<Vec<u32>> {
        let Some(mesh) = meshes.meshes.get(BorderClass::Province.bucket()) else { return Vec::new() };
        let mut chunks = vec![Vec::new(); mesh.chunks.len()];
        for (at, draw) in mesh.draws.iter().enumerate() {
            if let Some(chunk) = chunks.get_mut(draw.chunk as usize) {
                chunk.push(at as u32);
            }
        }
        chunks
    }

    pub fn patch_chunks(&mut self, meshes: &BorderMeshes, draws: &[Vec<u32>], chunks: &[usize], class: impl Fn(usize) -> Option<BorderClass>) {
        let Some(mesh) = meshes.meshes.get(BorderClass::Province.bucket()) else { return };
        for (at, target) in BorderClass::ALL.iter().enumerate() {
            if !matches!(target, BorderClass::Province | BorderClass::State | BorderClass::Country) {
                continue;
            }
            let runs = &mut self.runs[at];
            for &chunk in chunks {
                let (mut fine, mut coarse) = (Vec::new(), Vec::new());
                for &at in draws.get(chunk).map_or(&[][..], Vec::as_slice) {
                    if class(at as usize) == Some(*target) {
                        draw_indices(&mesh.draws[at as usize], &mut fine, &mut coarse);
                    }
                }
                runs.retain(|run| run.chunk != chunk);
                if !fine.is_empty() {
                    let place = runs.partition_point(|run| run.chunk < chunk);
                    let (fine, indices) = joined(fine, coarse);
                    runs.insert(place, PlannedRun { source: BorderClass::Province.bucket(), chunk, fine, indices: Arc::new(indices) });
                }
            }
        }
    }

    pub fn share(&mut self, other: &ClassPlan) {
        for (runs, others) in self.runs.iter_mut().zip(&other.runs) {
            for run in runs {
                if let Some(same) = others.iter().find(|other| {
                    (other.source, other.chunk) == (run.source, run.chunk)
                        && other.indices == run.indices
                }) {
                    run.indices = Arc::clone(&same.indices);
                }
            }
        }
    }
}

#[derive(Default)]
struct PlannedBuffers {
    lists: std::collections::HashMap<usize, Vec<PlannedBuffer>>,
    since_sweep: usize,
}

struct PlannedBuffer {
    device: wgpu::Device,
    list: std::sync::Weak<Vec<u32>>,
    buffer: std::sync::Weak<wgpu::Buffer>,
}

impl PlannedBuffer {
    fn live(&self) -> bool {
        self.list.strong_count() != 0 && self.buffer.strong_count() != 0
    }
}

static PLANNED: std::sync::LazyLock<std::sync::Mutex<PlannedBuffers>> =
    std::sync::LazyLock::new(Default::default);

fn index_buffer(device: &wgpu::Device, indices: &[u32]) -> io::Result<wgpu::Buffer> {
    let indices = indices
        .iter()
        .copied()
        .map(u16::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid("border chunk index exceeds 16 bits"))?;
    buffer(
        device,
        "border indices",
        wgpu::BufferUsages::INDEX,
        bytemuck::cast_slice(&indices),
    )
}

fn planned_buffer(device: &wgpu::Device, indices: &Arc<Vec<u32>>) -> io::Result<Arc<wgpu::Buffer>> {
    let key = Arc::as_ptr(indices) as usize;
    let mut planned = PLANNED.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(held) = planned.lists.get(&key).and_then(|entries| {
        entries.iter().find_map(|entry| {
            (entry.device == *device && std::ptr::eq(entry.list.as_ptr(), Arc::as_ptr(indices)))
                .then(|| entry.buffer.upgrade())
                .flatten()
        })
    }) {
        return Ok(held);
    }
    let made = Arc::new(index_buffer(device, indices)?);
    let planned = &mut *planned;
    planned.since_sweep += 1;
    if planned.since_sweep >= planned.lists.len() {
        planned.since_sweep = 0;
        planned.lists.retain(|_, entries| {
            entries.retain(PlannedBuffer::live);
            !entries.is_empty()
        });
    }
    let entries = planned.lists.entry(key).or_default();
    entries.retain(PlannedBuffer::live);
    entries.push(PlannedBuffer {
        device: device.clone(),
        list: Arc::downgrade(indices),
        buffer: Arc::downgrade(&made),
    });
    Ok(made)
}

struct Run {
    source: usize,
    chunk: usize,
    indices: Arc<wgpu::Buffer>,
    fine: u32,
    coarse: u32,
    planned: Option<Arc<Vec<u32>>>,
}

#[derive(Clone, Copy)]
pub struct BorderFrame {
    pub camera_height: f32,
    pub coarse: bool,
    pub fades: [[f32; 2]; 2],
    pub province_borders: bool,
    pub stage: BorderStage,
    pub visible: bool,
}

#[derive(Clone, Copy)]
pub enum BorderStage {
    Land,
    Sea,
}

const LAND_ORDER: &[BorderClass] = &[
    BorderClass::Province,
    BorderClass::State,
    BorderClass::Country,
    BorderClass::Impassable,
];
const SEA_ORDER: &[BorderClass] = &[BorderClass::Sea, BorderClass::SeaRegion];

pub struct BorderPass {
    compiled: Result<super::border_shader::Border, String>,
    textures: Vec<wgpu::TextureView>,
    pub compiled_straits: super::strait_shader::Straits,
    vertices: Vec<Vec<wgpu::Buffer>>,
    coarse_vertices: Vec<Vec<wgpu::Buffer>>,
    runs: Vec<Vec<Run>>,
    draws: Vec<Vec<BorderDraw>>,
}

impl BorderPass {
    pub fn texel_reads(&self) -> Option<&str> {
        self.compiled.as_ref().ok()?.texel_reads.as_deref()
    }

    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: &BorderAssets,
    ) -> io::Result<Self> {
        let textures = BorderClass::ALL
            .iter()
            .zip(&assets.classes)
            .map(|(class, texture)| {
                let label = format!("border_{}", class.texture_name());
                Ok(texture.upload(device, queue, &label, ColorSpace::Srgb)?.view)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let strait_texture = assets.strait.upload(
            device,
            queue,
            "strait",
            ColorSpace::Linear,
        )?;
        let compiled_straits = super::strait_shader::Straits::new(
            device,
            &assets.strait_shader,
            samples,
            format,
            DEPTH_FORMAT,
            strait_texture.view,
        );
        Ok(Self {
            compiled: super::border_shader::Border::new(
                device,
                &assets.border_shader,
                samples,
                format,
                DEPTH_FORMAT,
            )
            .map_err(|error| error.to_string()),
            textures,
            compiled_straits,
            vertices: Vec::new(),
            coarse_vertices: Vec::new(),
            runs: BorderClass::ALL.iter().map(|_| Vec::new()).collect(),
            draws: Vec::new(),
        })
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        straits: &[StraitVertex],
    ) -> io::Result<()> {
        self.compiled_straits.upload(device, straits)?;
        let upload = |level: &str, chunks: fn(&BorderMesh) -> &Vec<Vec<borders::Vertex>>| {
            meshes
                .meshes
                .iter()
                .map(|mesh| {
                    chunks(mesh)
                        .iter()
                        .enumerate()
                        .map(|(index, chunk)| {
                            buffer(
                                device,
                                &format!("border {} {level}chunk {index}", mesh.class.texture_name()),
                                wgpu::BufferUsages::VERTEX,
                                &chunk_bytes(chunk),
                            )
                        })
                        .collect::<io::Result<Vec<_>>>()
                })
                .collect::<io::Result<Vec<_>>>()
        };
        self.vertices = upload("", |mesh| &mesh.chunks)?;
        self.coarse_vertices = upload("coarse ", |mesh| &mesh.coarse_chunks)?;
        self.draws = meshes
            .meshes
            .iter()
            .map(|mesh| mesh.draws.clone())
            .collect();
        for class in [
            BorderClass::Impassable,
            BorderClass::Sea,
            BorderClass::SeaRegion,
        ] {
            self.runs[class.bucket()] =
                self.build_runs(device, class, meshes.mesh(class).draws.iter().copied())?;
        }
        Ok(())
    }

    pub fn set_classes(
        &mut self,
        device: &wgpu::Device,
        meshes: &BorderMeshes,
        regions: &Regions,
        zone: &Zone,
    ) -> io::Result<()> {
        self.set_plan(device, &ClassPlan::new(meshes, regions, zone))
    }

    pub fn set_plan(&mut self, device: &wgpu::Device, plan: &ClassPlan) -> io::Result<()> {
        for (bucket, runs) in plan.runs.iter().enumerate() {
            let chunks = |source: usize| self.vertices.get(source).map_or(0, Vec::len);
            let mut held = self
                .runs
                .get_mut(bucket)
                .map(std::mem::take)
                .unwrap_or_default();
            let built = runs
                .iter()
                .filter(|run| run.chunk < chunks(run.source))
                .map(|run| {
                    let kept = held.iter().position(|old| {
                        (old.source, old.chunk) == (run.source, run.chunk)
                            && old
                                .planned
                                .as_ref()
                                .is_some_and(|planned| Arc::ptr_eq(planned, &run.indices))
                    });
                    if let Some(kept) = kept {
                        return Ok(held.swap_remove(kept));
                    }
                    Ok(Run {
                        source: run.source,
                        chunk: run.chunk,
                        fine: run.fine,
                        coarse: run.indices.len() as u32 - run.fine,
                        indices: planned_buffer(device, &run.indices)?,
                        planned: Some(Arc::clone(&run.indices)),
                    })
                })
                .collect::<io::Result<Vec<_>>>()?;
            if let Some(slot) = self.runs.get_mut(bucket) {
                *slot = built;
            }
        }
        Ok(())
    }

    fn build_runs(
        &self,
        device: &wgpu::Device,
        source: BorderClass,
        draws: impl Iterator<Item = BorderDraw>,
    ) -> io::Result<Vec<Run>> {
        let chunks = self.vertices.get(source.bucket()).map_or(0, Vec::len);
        per_chunk(chunks, draws)
            .into_iter()
            .map(|(chunk, (fine, indices))| {
                Ok(Run {
                    source: source.bucket(),
                    chunk,
                    fine,
                    coarse: indices.len() as u32 - fine,
                    indices: Arc::new(index_buffer(device, &indices)?),
                    planned: None,
                })
            })
            .collect()
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        inputs: super::border_shader::Inputs<'_>,
        frame: BorderFrame,
    ) -> io::Result<viewer_scene_base::shader_constants::WorldPixelConstants> {
        let classes = match frame.stage {
            BorderStage::Land => LAND_ORDER,
            BorderStage::Sea => SEA_ORDER,
        };
        let batches = classes
            .iter()
            .map(|&class| {
                let alpha = borders::class_alpha(
                    class,
                    frame.camera_height,
                    frame.fades,
                    frame.province_borders,
                );
                super::border_shader::Batch {
                    texture: &self.textures[class.bucket()],
                    selection: [0.; 4],
                    alpha,
                    runs: self.runs[class.bucket()]
                        .iter()
                        .filter(|_| alpha > 0.)
                        .filter_map(|run| {
                            let level = if frame.coarse { &self.coarse_vertices } else { &self.vertices };
                            let range = viewer_scene_base::line_lod::drawn(run.fine, run.coarse, frame.coarse);
                            level
                                .get(run.source)
                                .and_then(|chunks| chunks.get(run.chunk))
                                .map(|vertices| super::border_shader::Run {
                                    vertices,
                                    indices: &run.indices,
                                    index_format: wgpu::IndexFormat::Uint16,
                                    range,
                                })
                        })
                        .collect(),
                }
            })
            .collect::<Vec<_>>();
        viewer_gpu::gpu_contract::compiled(&self.compiled)?.draw(
            arena,
            encoder,
            targets,
            inputs,
            &batches,
            frame.visible,
        )
    }
}

#[cfg(test)]
mod tests;
