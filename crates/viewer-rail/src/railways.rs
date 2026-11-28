use content_reader_mesh::mesh::MeshProducts as _;

use viewer_gpu::gpu_contract::Targets;
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_key_names::buildings::Buildings;
use content_reader_map::{map::MapData, positions::BuildingPosition};
use content_reader_image::native::Decode;
use content_reader_image::{IndexedImage, dds};
use replay_data::{rows::NavalBaseRow, rows::ProvinceSupplyRow, rows::RailEdgeRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    ops::Range,
    sync::{Arc, Mutex},
};
use viewer_gpu::province_geometry::sort_edge_pixels;
use viewer_foundation::tables::rows;
use viewer_gpu::textures::load_uploadable;
use viewer_models::{ModelField, ModelInstance, ModelLibrary, WATER_HEIGHT};
use viewer_gpu::{
    camera::Camera,
    gpu_contract::{DEPTH_FORMAT, buffer},
    province_map::HeightField,
    textures::{Bound, ColorSpace, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub const RAIL_WIDTH: f32 = 1.0;
pub const RAILROAD_TEXTURE: &str = "gfx/models/supply/railroad.dds";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RailFade {
    alpha: f32,
    last: Option<f32>,
}

impl RailFade {
    pub const TRAINS_VISIBLE: f32 = 0.001;

    pub fn update(&mut self, camera: &Camera, time: f32) -> f32 {
        let above = camera.height >= camera.defines.railway_cutoff;
        let target = if above { 0.0 } else { 1.0 };
        match self.last {
            Some(last) if time >= last => {
                let step = (time - last) * camera.defines.railway_cutoff_speed;
                if above {
                    if self.alpha > 0.0 {
                        self.alpha = (self.alpha - step).max(0.0);
                    }
                } else if self.alpha < 1.0 {
                    self.alpha = (self.alpha + step).min(1.0);
                }
            }
            _ => self.alpha = target,
        }
        self.last = Some(time);
        self.alpha
    }

    pub fn trains_visible(&self) -> bool {
        self.alpha >= Self::TRAINS_VISIBLE
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailDefines {
    pub y_offset: f32,
    pub bridge_y_offset: f32,
    pub half_widths: [f32; 2],
    pub half_gaps: [f32; 2],
    pub levels: u32,
}

impl RailDefines {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> io::Result<Self> {
        let number = |name| defines.number("NGraphics", name).map(|value| value as f32);
        let levels = defines.number("NSupply", "MAX_RAILWAY_LEVEL")?;
        if !(1.0..=f64::from(u16::MAX)).contains(&levels) {
            return Err(invalid(format!(
                "NDefines.NSupply.MAX_RAILWAY_LEVEL {levels} is not a positive level count"
            )));
        }
        Ok(Self {
            y_offset: number("RAILWAY_Y_OFFSET")?,
            bridge_y_offset: number("RAILWAY_BRIDGE_Y_OFFSET")?,
            half_widths: [
                number("RAILWAY_BRIDGE_WIDTH")? / 2.0,
                number("RAILWAY_BRIDGE_LARGE_WIDTH")? / 2.0,
            ],
            half_gaps: [
                number("RAILWAY_BRIDGE_GAP_WIDTH")? / 2.0,
                number("RAILWAY_BRIDGE_GAP_LARGE_WIDTH")? / 2.0,
            ],
            levels: levels as u32,
        })
    }

    fn bridge(&self, kind: BridgeKind) -> (f32, f32) {
        match kind {
            BridgeKind::None => (0.0, 0.0),
            BridgeKind::Small => (self.half_widths[0], self.half_gaps[0]),
            BridgeKind::Large => (self.half_widths[1], self.half_gaps[1]),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RailVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RailMesh {
    #[cfg(test)]
    vertices: Vec<RailVertex>,
    #[cfg(test)]
    indices: Vec<u32>,
    pub segments: usize,
    pub unanchored: usize,
    pub samples: usize,
    pub bounds: [f32; 4],
    pub bridges: Vec<Bridge>,
    pub native_vertices: Vec<[f32; 6]>,
    pub native_runs: Vec<Range<u32>>,
}

impl RailMesh {
    fn with_capacity(vertices: usize, runs: usize, bridges: usize) -> io::Result<Self> {
        use engine_allocation as allocation;
        Ok(Self {
            native_vertices: allocation::vec(vertices)?,
            native_runs: allocation::vec(runs)?,
            bridges: allocation::vec(bridges)?,
            ..Self::default()
        })
    }

    fn push_run(&mut self, vertices: &[[f32; 6]], segments: usize, unanchored: usize, samples: usize) {
        self.segments += segments;
        self.unanchored += unanchored;
        self.samples += samples;
        if !vertices.is_empty() {
            let first = self.native_vertices.len() as u32;
            self.native_vertices.extend_from_slice(vertices);
            self.native_runs.push(first..self.native_vertices.len() as u32);
        }
    }

    fn finish(&mut self, bridges: impl IntoIterator<Item = Bridge>) {
        self.bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for vertex in &self.native_vertices {
            self.bounds[0] = self.bounds[0].min(vertex[0]);
            self.bounds[1] = self.bounds[1].min(vertex[2]);
            self.bounds[2] = self.bounds[2].max(vertex[0]);
            self.bounds[3] = self.bounds[3].max(vertex[2]);
        }
        if self.native_vertices.is_empty() {
            self.bounds = [0.0; 4];
        }
        self.bridges.extend(bridges);
    }

    pub fn assemble<'a>(runs: impl ExactSizeIterator<Item = &'a RailRun> + Clone, bridges: impl ExactSizeIterator<Item = Bridge>) -> io::Result<Self> {
        let mut mesh = Self::with_capacity(runs.clone().map(|run| run.vertices.len()).sum(), runs.clone().filter(|run| !run.vertices.is_empty()).count(), bridges.len())?;
        for run in runs {
            mesh.push_run(&run.vertices, run.segments, run.unanchored, run.samples);
        }
        mesh.finish(bridges);
        Ok(mesh)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BridgeKind {
    #[default]
    None,
    Small,
    Large,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bridge {
    pub position: [f32; 3],
    pub rotation: f32,
    pub kind: BridgeKind,
}
#[derive(Clone, Copy, Debug)]
struct Crossing {
    position: [f32; 3],
    direction: [f32; 3],
    half_width: f32,
    half_gap: f32,
}
#[derive(Clone, Debug)]
struct Boundary {
    points: Vec<[i32; 2]>,
    kind: BridgeKind,
}

type RiverPoint = ([f32; 2], [f32; 2]);

pub struct RailGeometry {
    id: u64,
    identity: Arc<()>,
    heightmap: Arc<IndexedImage>,
    defines: RailDefines,
    anchors: BTreeMap<u32, [f32; 3]>,
    ports: BTreeMap<u32, [f32; 3]>,
    boundaries: BTreeMap<(u32, u32), Boundary>,
    river_points: BTreeMap<u32, Vec<RiverPoint>>,
    last: Mutex<Option<Arc<RailBase>>>,
}

type Graph = BTreeMap<u32, BTreeMap<u32, i32>>;

fn changed_provinces(old: &Graph, new: &Graph) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    let (mut olds, mut news) = (old.iter(), new.iter());
    let (mut a, mut b) = (olds.next(), news.next());
    loop {
        match (a, b) {
            (Some((x, old)), Some((y, new))) => match x.cmp(y) {
                std::cmp::Ordering::Less => {
                    out.insert(*x);
                    a = olds.next();
                }
                std::cmp::Ordering::Greater => {
                    out.insert(*y);
                    b = news.next();
                }
                std::cmp::Ordering::Equal => {
                    if old != new {
                        out.insert(*x);
                    }
                    a = olds.next();
                    b = news.next();
                }
            },
            (Some((x, _)), None) => {
                out.insert(*x);
                a = olds.next();
            }
            (None, Some((y, _))) => {
                out.insert(*y);
                b = news.next();
            }
            (None, None) => return out,
        }
    }
}

#[derive(Clone)]
struct ProvinceRun {
    province: u32,
    vertices: Arc<Vec<[f32; 6]>>,
    segments: usize,
    unanchored: usize,
    samples: usize,
    #[cfg(test)]
    test: Arc<(Vec<RailVertex>, Vec<u32>)>,
}

pub struct RailBase {
    owner: Arc<()>,
    network: Arc<RailNetwork>,
    runs: Vec<ProvinceRun>,
    bridges: Vec<((u32, u32), Bridge)>,
}

fn pair(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}
fn add(a: [f32; 3], b: [f32; 3], scale: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i] * scale)
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn normalized(a: [f32; 3]) -> [f32; 3] {
    let n = length(a);
    if n > 1e-6 {
        a.map(|v| v / n)
    } else {
        [1.0, 0.0, 0.0]
    }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn rail_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
    let x = x + 0.5;
    let z = z + 0.5;
    let ix = x as i32;
    let iz = z as i32;
    let sample = |x: i32, z: i32| {
        let x = x.clamp(0, heights.width as i32 - 1) as usize;
        let z = z.clamp(0, heights.height as i32 - 1) as usize;
        f32::from(heights.values[(heights.height as usize - 1 - z) * heights.width as usize + x])
            * 0.1
    };
    let a = sample(ix, iz);
    let b = sample(ix, iz + 1);
    let lo = a + (sample(ix + 1, iz) - a) * (x - ix as f32);
    let hi = b + (sample(ix + 1, iz + 1) - b) * (x - ix as f32);
    (lo + (hi - lo) * (z - iz as f32)).max(WATER_HEIGHT)
}

fn pixel_line(a: [i32; 2], b: [i32; 2]) -> Vec<[i32; 2]> {
    let steep = (b[0] - a[0]).abs() < (b[1] - a[1]).abs();
    let swap = |p: [i32; 2]| if steep { [p[1], p[0]] } else { p };
    let (mut a, mut b) = (swap(a), swap(b));
    if a[0] > b[0] {
        std::mem::swap(&mut a, &mut b);
    }
    let dx = b[0] - a[0];
    let dy = (b[1] - a[1]).abs();
    let step = if a[1] < b[1] { 1 } else { -1 };
    let mut error = 0;
    let mut y = a[1];
    let mut out = Vec::new();
    for x in a[0]..=b[0] {
        out.push(swap([x, y]));
        error += dy;
        if error * 2 >= dx {
            y += step;
            error -= dx;
        }
    }
    out
}

fn boundary_middle(points: &[[i32; 2]]) -> [i32; 2] {
    let mut unseen: BTreeSet<_> = points.iter().map(|p| (p[1], p[0])).collect();
    let mut best = Vec::new();
    while let Some(&(z, x)) = unseen.first() {
        unseen.remove(&(z, x));
        let mut component = vec![[x, z]];
        let mut next = 0;
        while next < component.len() {
            let [x, z] = component[next];
            next += 1;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if unseen.remove(&(z + dz, x + dx)) {
                        component.push([x + dx, z + dz]);
                    }
                }
            }
        }
        if component.len() > best.len() {
            best = component;
        }
    }
    std::array::from_fn(|i| {
        let sum: i64 = best
            .iter()
            .map(|p| i64::from(p[i]) * 100000 / best.len() as i64)
            .sum();
        ((sum + 50000) / 100000) as i32
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RailNetwork {
    resolved: Arc<map_display_data::landscape::RailGraph>,
}
impl std::ops::Deref for RailNetwork {
    type Target = map_display_data::landscape::RailGraph;
    fn deref(&self) -> &Self::Target {
        &self.resolved
    }
}

impl RailNetwork {
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let edges = rows::<tables::RailEdges>(frame)?;
        let supply = rows::<tables::ProvinceSupply>(frame)?;
        let naval_bases = rows::<tables::NavalBases>(frame)?;
        if !naval_bases.is_complete() {
            return Ok(Self::new(&[], &[], &[]));
        }
        Ok(Self::new(edges.rows(), supply.rows(), naval_bases.rows()))
    }

    pub fn new(
        edges: &[RailEdgeRow],
        supply: &[ProvinceSupplyRow],
        naval_bases: &[NavalBaseRow],
    ) -> Self {
        let mut graph: BTreeMap<u32, BTreeMap<u32, i32>> = BTreeMap::new();
        for e in edges {
            if e.level > 0 && e.from != e.to {
                graph.entry(e.from).or_default().insert(e.to, e.level);
            }
        }
        let sources = supply
            .iter()
            .filter(|r| {
                r.supply_node_level != 0
                    || r.naval_base_level != 0
                    || r.capital_country_index != 0
                    || r.flags & 1 != 0
            })
            .map(|r| r.province)
            .collect();
        let ports = naval_bases.iter().map(|r| r.province).collect();
        Self {
            resolved: Arc::new(map_display_data::landscape::RailGraph {
                graph,
                sources,
                ports,
            }),
        }
    }

    pub fn display(&self) -> &map_display_data::landscape::RailGraph {
        &self.resolved
    }
    pub fn graph_mut(&mut self) -> &mut map_display_data::landscape::RailGraph {
        Arc::make_mut(&mut self.resolved)
    }
    pub fn shared_display(&self) -> Arc<map_display_data::landscape::RailGraph> {
        Arc::clone(&self.resolved)
    }
    pub fn from_display(resolved: map_display_data::landscape::RailGraph) -> Self {
        Self {
            resolved: Arc::new(resolved),
        }
    }

    fn has_segment(&self, province: u32, neighbour: u32, other: Option<u32>) -> bool {
        let Some(ns) = self.graph.get(&province) else {
            return false;
        };
        if !ns.contains_key(&neighbour) {
            return false;
        }
        match other {
            None => ns.len() == 1 || self.sources.contains(&province),
            Some(other) => {
                let (a, b) = pair(neighbour, other);
                a != b
                    && ns.contains_key(&other)
                    && !self.graph.get(&b).is_some_and(|n| n.contains_key(&a))
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RailRun {
    pub vertices: Vec<[f32; 6]>,
    pub segments: usize,
    pub unanchored: usize,
    pub samples: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrainRoute {
    pub points: Vec<[f32; 3]>,
    pub open_end: bool,
}

impl RailGeometry {
    pub fn projection_id(&self) -> u64 {
        self.id
    }
    pub fn new(
        map: &MapData,
        buildings: &[BuildingPosition],
        types: &Buildings,
        rivers: &content_reader_image::IndexedImage,
        heightmap: Arc<IndexedImage>,
        defines: RailDefines,
    ) -> io::Result<Self> {
        let heights = &HeightField {
            width: heightmap.width,
            height: heightmap.height,
            values: &heightmap.indices,
        };
        if rivers.width != map.width
            || rivers.height != map.height
            || heights.width != map.width
            || heights.height != map.height
        {
            return Err(invalid(
                "railway geography requires matching province, river and height maps",
            ));
        }
        let w = map.width as usize;
        let h = map.height as usize;
        let at = |x: i32, z: i32| -> u32 {
            if z < 0 || z >= h as i32 {
                0
            } else {
                map.province_ids[(h - 1 - z as usize) * w + x.rem_euclid(w as i32) as usize]
            }
        };
        let mut sums = vec![
            (0i64, 0i64, 0i64);
            map.province_ids.iter().copied().max().unwrap_or(0) as usize + 1
        ];
        let mut boundaries: BTreeMap<(u32, u32), Boundary> = BTreeMap::new();
        for z in 0..h as i32 {
            for x in 0..w as i32 {
                let id = at(x, z);
                let sum = &mut sums[id as usize];
                sum.0 += i64::from(x);
                sum.1 += i64::from(z);
                sum.2 += 1;
                let mut adjacent = [at(x - 1, z), at(x + 1, z), at(x, z - 1), at(x, z + 1)];
                adjacent.sort_unstable();
                for i in 0..4 {
                    let other = adjacent[i];
                    if other > id && id > 0 && (i == 0 || adjacent[i - 1] != other) {
                        boundaries
                            .entry((id, other))
                            .or_insert_with(|| Boundary {
                                points: Vec::new(),
                                kind: BridgeKind::None,
                            })
                            .points
                            .push([x, z]);
                    }
                }
            }
        }
        let centroids: BTreeMap<_, _> = sums
            .iter()
            .enumerate()
            .filter(|(id, s)| *id > 0 && s.2 > 0)
            .map(|(id, &(x, z, n))| (id as u32, [(x / n) as i32, (z / n) as i32]))
            .collect();
        let mut anchors: BTreeMap<_, _> = centroids
            .iter()
            .map(|(&id, &[x, z])| {
                (
                    id,
                    [
                        x as f32,
                        rail_height(heights, x as f32 - 1.0, z as f32 - 1.0),
                        z as f32,
                    ],
                )
            })
            .collect();
        let hubs: BTreeSet<&str> = types
            .types
            .values()
            .filter(|building| building.supply_node)
            .map(|building| building.slot_name())
            .collect();
        let port_slots: BTreeSet<&str> = types
            .spawn_points
            .values()
            .filter(|spawn| spawn.is_port)
            .map(|spawn| spawn.name.as_str())
            .collect();
        let mut ports = BTreeMap::new();
        for p in buildings {
            let Some(content_reader_key_names::buildings::Slot::Bucket(slot)) = types.slot(&p.kind) else {
                continue;
            };
            let id = at(p.x as i32, p.z as i32);
            if id == 0 {
                continue;
            }
            if hubs.contains(slot.name.as_str()) {
                anchors.insert(id, [p.x, p.y, p.z]);
            } else if port_slots.contains(slot.name.as_str()) {
                ports.insert(id, [p.x, p.y, p.z]);
            }
        }
        for (&(a, b), boundary) in &mut boundaries {
            if !matches!(
                map.definitions.get(&a).map(|d| d.kind),
                Some(content_reader_map::map::ProvinceKind::Land)
            ) || !matches!(
                map.definitions.get(&b).map(|d| d.kind),
                Some(content_reader_map::map::ProvinceKind::Land)
            ) {
                continue;
            }
            let middle = boundary_middle(&boundary.points);
            let mut first = pixel_line(centroids[&a], middle);
            if first.first() == Some(&middle) {
                let last = first.len() - 1;
                first.swap(0, last);
            }
            let mut far = centroids[&b];
            if (far[0] - centroids[&a][0]).abs() > w as i32 / 2 {
                far[0] += if centroids[&a][0] < w as i32 / 2 {
                    -(w as i32)
                } else {
                    w as i32
                };
            }
            first.pop();
            first.extend(pixel_line(middle, far));
            for [x, z] in first {
                if z < 0 || z >= h as i32 {
                    continue;
                }
                let index =
                    rivers.indices[(h - 1 - z as usize) * w + x.rem_euclid(w as i32) as usize];
                if index <= 11 {
                    boundary.kind = if index > 6 {
                        BridgeKind::Large
                    } else {
                        BridgeKind::Small
                    };
                    break;
                }
            }
        }
        for boundary in boundaries.values_mut() {
            let pixels: Vec<_> = boundary
                .points
                .iter()
                .map(|p| [p[0] as f32, p[1] as f32])
                .collect();
            boundary.points = sort_edge_pixels(&pixels)
                .into_iter()
                .map(|p| [p[0] as i32, p[1] as i32])
                .collect();
        }
        let traced = viewer_terrain::rivers::trace(&viewer_terrain::rivers::RiverImage {
            width: rivers.width,
            height: rivers.height,
            indices: &rivers.indices,
        })?;
        let mut river_points: BTreeMap<u32, Vec<RiverPoint>> = BTreeMap::new();
        for river in traced.rivers {
            let mut points = river.points;
            for i in 2..points.len().saturating_sub(2) {
                points[i] = std::array::from_fn(|axis| {
                    (points[i - 2][axis]
                        + points[i - 1][axis]
                        + points[i][axis]
                        + points[i + 1][axis]
                        + points[i + 2][axis])
                        / 5.0
                });
            }
            for i in 0..points.len().saturating_sub(1) {
                let p = points[i];
                let before = points[i.saturating_sub(1)];
                let after = points[i + 1];
                let d = [after[0] - before[0], after[1] - before[1]];
                let n = d[0].hypot(d[1]);
                if n > 0.0 {
                    river_points
                        .entry(at(p[0] as i32, p[1] as i32))
                        .or_default()
                        .push((p, [d[0] / n, d[1] / n]));
                }
            }
        }
        Ok(Self {
            id: viewer_scene_base::next_projection_id(),
            identity: Arc::new(()),
            heightmap,
            defines,
            anchors,
            ports,
            boundaries,
            river_points,
            last: Mutex::new(None),
        })
    }

    fn pair_crossing(
        &self,
        network: &RailNetwork,
        key: (u32, u32),
        heights: &HeightField<'_>,
    ) -> Option<Crossing> {
        let (Some(&a), Some(&b)) = (self.anchor(network, key.0), self.anchor(network, key.1))
        else {
            return None;
        };
        Some(self.crossing(key, a, b, heights))
    }

    fn bridge_at(&self, key: (u32, u32), c: &Crossing) -> Option<Bridge> {
        let kind = self
            .boundaries
            .get(&key)
            .map_or(BridgeKind::None, |boundary| boundary.kind);
        (kind != BridgeKind::None).then(|| Bridge {
            position: add(c.position, [0.0, 1.0, 0.0], self.defines.bridge_y_offset),
            rotation: c.direction[2].atan2(c.direction[0]) + std::f32::consts::FRAC_PI_2,
            kind,
        })
    }

    fn pair_bridge(
        &self,
        network: &RailNetwork,
        key: (u32, u32),
        heights: &HeightField<'_>,
    ) -> Option<((u32, u32), Bridge)> {
        let c = self.pair_crossing(network, key, heights)?;
        self.bridge_at(key, &c).map(|bridge| (key, bridge))
    }

    fn emit_province(
        &self,
        network: &RailNetwork,
        province: u32,
        neighbors: &BTreeMap<u32, i32>,
        crossing: impl Fn(u32) -> Option<Crossing>,
        mesh: &mut RailMesh,
        heights: &HeightField<'_>,
    ) -> io::Result<()> {
        let first = mesh.native_vertices.len() as u32;
        let ns: Vec<_> = engine_allocation::collect_vec(neighbors.iter())?;
        for (i, &(&a, &level)) in ns.iter().enumerate() {
            let start = crossing(a).map(|mut c| {
                if a > province {
                    c.direction = c.direction.map(|v| -v);
                }
                c
            });
            for &(&b, &other_level) in &ns[i + 1..] {
                if network.graph.get(&b).is_some_and(|n| n.contains_key(&a)) {
                    continue;
                }
                let end = crossing(b).map(|mut c| {
                    if b > province {
                        c.direction = c.direction.map(|v| -v);
                    }
                    c
                });
                mesh.segments += 1;
                if let (Some(start), Some(end)) = (start, end) {
                    Curve::new(start, end).emit(
                        mesh,
                        level.min(other_level),
                        heights,
                        &self.defines,
                    )?;
                } else {
                    mesh.unanchored += 1;
                }
            }
            if ns.len() == 1 || network.sources.contains(&province) {
                mesh.segments += 1;
                if let (Some(start), Some(&center)) = (start, self.anchor(network, province)) {
                    stub(start, center).emit(mesh, level, heights, &self.defines)?;
                } else {
                    mesh.unanchored += 1;
                }
            }
        }
        let end = mesh.native_vertices.len() as u32;
        if first != end {
            engine_allocation::reserve(&mut mesh.native_runs, 1)?;
            mesh.native_runs.push(first..end);
        }
        Ok(())
    }

    pub fn run(&self, network: &RailNetwork, province: u32) -> io::Result<Option<RailRun>> {
        let Some(neighbors) = network.graph.get(&province) else {
            return Ok(None);
        };
        let run = self.province_run(network, province, neighbors, &self.heights())?;
        Ok(Some(RailRun {
            vertices: Arc::unwrap_or_clone(run.vertices),
            segments: run.segments,
            unanchored: run.unanchored,
            samples: run.samples,
        }))
    }

    pub fn bridge(&self, network: &RailNetwork, key: (u32, u32)) -> Option<Bridge> {
        self.pair_bridge(network, key, &self.heights())
            .map(|(_, bridge)| bridge)
    }

    fn province_run(
        &self,
        network: &RailNetwork,
        province: u32,
        neighbors: &BTreeMap<u32, i32>,
        heights: &HeightField<'_>,
    ) -> io::Result<ProvinceRun> {
        let crossings: BTreeMap<u32, Crossing> = neighbors
            .keys()
            .filter_map(|&other| {
                self.pair_crossing(network, pair(other, province), heights)
                    .map(|c| (other, c))
            })
            .collect();
        let mut mesh = RailMesh::default();
        self.emit_province(
            network,
            province,
            neighbors,
            |other| crossings.get(&other).copied(),
            &mut mesh,
            heights,
        )?;
        Ok(ProvinceRun {
            province,
            vertices: Arc::new(mesh.native_vertices),
            segments: mesh.segments,
            unanchored: mesh.unanchored,
            samples: mesh.samples,
            #[cfg(test)]
            test: Arc::new((mesh.vertices, mesh.indices)),
        })
    }

    fn heights(&self) -> HeightField<'_> {
        HeightField {
            width: self.heightmap.width,
            height: self.heightmap.height,
            values: &self.heightmap.indices,
        }
    }

    pub fn build(&self, network: &Arc<RailNetwork>) -> io::Result<RailMesh> {
        let base = self.last_base();
        let (mesh, next) = self.build_base(base.as_deref(), network)?;
        self.keep_base(Arc::new(next));
        Ok(mesh)
    }

    fn last_base(&self) -> Option<Arc<RailBase>> {
        self.last.lock().ok().and_then(|last| last.clone())
    }

    fn keep_base(&self, base: Arc<RailBase>) {
        if !Arc::ptr_eq(&base.owner, &self.identity) {
            return;
        }
        if let Ok(mut last) = self.last.lock() {
            *last = Some(base);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn build_from(
        &self,
        base: Option<&RailBase>,
        network: &Arc<RailNetwork>,
    ) -> io::Result<(RailMesh, RailBase)> {
        self.build_base(base, network)
    }

    fn build_base(
        &self,
        base: Option<&RailBase>,
        network: &Arc<RailNetwork>,
    ) -> io::Result<(RailMesh, RailBase)> {
        use engine_allocation as allocation;
        let heights = &self.heights();
        let base = base.filter(|base| Arc::ptr_eq(&base.owner, &self.identity));
        let graph = &network.graph;
        let (changed, sources, ports): (BTreeSet<u32>, BTreeSet<u32>, BTreeSet<u32>) = match base {
            Some(base) => {
                let old = &base.network;
                (
                    changed_provinces(&old.graph, graph),
                    old.sources
                        .symmetric_difference(&network.sources)
                        .copied()
                        .collect(),
                    old.ports
                        .symmetric_difference(&network.ports)
                        .copied()
                        .collect(),
                )
            }
            None => Default::default(),
        };
        let reach: BTreeSet<u32> = changed.union(&ports).copied().collect();
        let mut runs: Vec<ProvinceRun> = allocation::vec(graph.len())?;
        let mut touched = BTreeSet::new();
        let mut olds = base
            .map_or(&[][..], |base| base.runs.as_slice())
            .iter()
            .peekable();
        for (&province, neighbors) in graph {
            while olds.next_if(|run| run.province < province).is_some() {}
            let old = olds.next_if(|run| run.province == province);
            let mut dirty = base.is_none()
                || changed.contains(&province)
                || sources.contains(&province)
                || ports.contains(&province);
            if !reach.is_empty() {
                for &neighbour in neighbors.keys() {
                    let near = reach.contains(&neighbour);
                    dirty |= near;
                    if near || reach.contains(&province) {
                        touched.insert(pair(province, neighbour));
                    }
                }
            }
            let run = match old {
                Some(old) if !dirty => old.clone(),
                _ => self.province_run(network, province, neighbors, heights)?,
            };
            runs.push(run);
        }
        let bridges = match base {
            Some(base) => {
                let mut bridges = allocation::collect_vec(
                    base.bridges
                        .iter()
                        .filter(|(key, _)| !reach.contains(&key.0) && !reach.contains(&key.1))
                        .copied(),
                )?;
                allocation::reserve(&mut bridges, touched.len())?;
                bridges.extend(
                    touched
                        .iter()
                        .filter_map(|&key| self.pair_bridge(network, key, heights)),
                );
                bridges.sort_unstable_by_key(|(key, _)| *key);
                bridges
            }
            None => {
                let every: BTreeSet<(u32, u32)> = graph
                    .iter()
                    .flat_map(|(&from, ns)| ns.keys().map(move |&to| pair(from, to)))
                    .collect();
                allocation::collect_vec(
                    every
                        .into_iter()
                        .filter_map(|key| self.pair_bridge(network, key, heights)),
                )?
            }
        };
        let mut mesh = RailMesh::with_capacity(
            runs.iter().map(|run| run.vertices.len()).sum(),
            runs.iter().filter(|run| !run.vertices.is_empty()).count(),
            bridges.len(),
        )?;
        for run in &runs {
            mesh.push_run(&run.vertices, run.segments, run.unanchored, run.samples);
            #[cfg(test)]
            {
                let offset = mesh.vertices.len() as u32;
                mesh.vertices.extend_from_slice(&run.test.0);
                mesh.indices
                    .extend(run.test.1.iter().map(|index| index + offset));
            }
        }
        mesh.finish(bridges.iter().map(|(_, bridge)| *bridge));
        Ok((
            mesh,
            RailBase {
                owner: Arc::clone(&self.identity),
                network: Arc::clone(network),
                runs,
                bridges,
            },
        ))
    }

    fn crossing(
        &self,
        key: (u32, u32),
        a: [f32; 3],
        b: [f32; 3],
        heights: &HeightField<'_>,
    ) -> Crossing {
        let direction = normalized(sub(b, a));
        let mut position = add(a, sub(b, a), 0.5);
        let mut kind = BridgeKind::None;
        if let Some(boundary) = self.boundaries.get(&key)
            && let Some(p) = boundary.points.iter().min_by_key(|p| {
                let dx0 = p[0] - a[0] as i32;
                let dz0 = p[1] - a[2] as i32;
                let dx1 = p[0] - b[0] as i32;
                let dz1 = p[1] - b[2] as i32;
                dx0 * dx0 + dz0 * dz0 + dx1 * dx1 + dz1 * dz1
            })
        {
            position = [p[0] as f32, 0.0, p[1] as f32];
            kind = boundary.kind;
        }
        let mut tangent = direction;
        if kind != BridgeKind::None {
            let best = self
                .river_points
                .get(&key.0)
                .into_iter()
                .flatten()
                .chain(self.river_points.get(&key.1).into_iter().flatten())
                .min_by(|(p, _), (q, _)| {
                    let dist =
                        |v: [f32; 2]| (v[0] - position[0]).powi(2) + (v[1] - position[2]).powi(2);
                    dist(*p).total_cmp(&dist(*q))
                });
            if let Some(&(p, d)) = best {
                position[0] = p[0];
                position[2] = p[1];
                tangent = [d[1], 0.0, -d[0]];
                if dot(tangent, direction) < 0.0 {
                    tangent = tangent.map(|v| -v);
                }
            }
        }
        position[1] = rail_height(heights, position[0] - 1.0, position[2] - 1.0);
        let (half_width, half_gap) = self.defines.bridge(kind);
        Crossing {
            position,
            direction: tangent,
            half_width,
            half_gap,
        }
    }

    pub fn train_route(
        &self,
        path: &[u32],
        network: &RailNetwork,
        heights: &HeightField<'_>,
    ) -> Option<TrainRoute> {
        if path.len() < 2 {
            return None;
        }
        let anchor = |id: u32| self.anchor(network, id);
        let into = |neighbour: u32, province: u32| -> Option<Crossing> {
            let key = pair(neighbour, province);
            let mut c = self.crossing(key, *anchor(key.0)?, *anchor(key.1)?, heights);
            if neighbour > province {
                c.direction = c.direction.map(|v| -v);
            }
            Some(c)
        };
        let mut out: Vec<[f32; 3]> = Vec::new();
        let mut open_end = false;
        for (index, &province) in path.iter().enumerate() {
            let previous = index.checked_sub(1).map(|i| path[i]);
            let next = path.get(index + 1).copied();
            let segment = match (previous, next) {
                (Some(previous), next) => network.has_segment(province, previous, next),
                (None, Some(next)) => network.has_segment(province, next, None),
                (None, None) => false,
            };
            if !segment {
                open_end = true;
                break;
            }
            let (curve, backwards) = match (previous, next) {
                (Some(previous), Some(next)) => (
                    Curve::new(into(previous, province)?, into(next, province)?),
                    false,
                ),
                (None, Some(next)) => (stub(into(next, province)?, *anchor(province)?), true),
                (Some(previous), None) => {
                    (stub(into(previous, province)?, *anchor(province)?), false)
                }
                (None, None) => return None,
            };
            let mut centers = curve.centers();
            if backwards {
                centers.reverse();
            }
            for mut p in centers {
                p[1] = rail_height(heights, p[0], p[2]);
                if out.last().is_some_and(|q| length(sub(p, *q)) < 1e-4) {
                    continue;
                }
                out.push(p);
            }
        }
        (out.len() >= 2).then_some(TrainRoute {
            points: out,
            open_end,
        })
    }

    fn anchor(&self, network: &RailNetwork, id: u32) -> Option<&[f32; 3]> {
        if network.ports.contains(&id) {
            self.ports.get(&id).or_else(|| self.anchors.get(&id))
        } else {
            self.anchors.get(&id)
        }
    }

    #[cfg(test)]
    fn build_rows(
        &self,
        edges: &[RailEdgeRow],
        supply: &[ProvinceSupplyRow],
        naval_bases: &[NavalBaseRow],
    ) -> RailMesh {
        self.build_network(&RailNetwork::new(edges, supply, naval_bases))
            .expect("the test railway grows")
    }

    pub fn build_network(&self, network: &RailNetwork) -> io::Result<RailMesh> {
        let heights = &self.heights();
        let graph = &network.graph;
        let anchor = |id: u32| self.anchor(network, id);
        let mut crossings = BTreeMap::new();
        for (&from, neighbors) in graph {
            for &to in neighbors.keys() {
                let key = pair(from, to);
                if crossings.contains_key(&key) {
                    continue;
                }
                let (Some(&a), Some(&b)) = (anchor(key.0), anchor(key.1)) else {
                    continue;
                };
                crossings.insert(key, self.crossing(key, a, b, heights));
            }
        }
        let mut mesh = RailMesh {
            bounds: [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            ..Default::default()
        };
        engine_allocation::reserve(&mut mesh.bridges, crossings.len())?;
        engine_allocation::reserve(&mut mesh.native_runs, graph.len())?;
        for (&key, c) in &crossings {
            if (graph.get(&key.0).is_some_and(|n| n.contains_key(&key.1))
                || graph.get(&key.1).is_some_and(|n| n.contains_key(&key.0)))
                && let Some(bridge) = self.bridge_at(key, c)
            {
                mesh.bridges.push(bridge);
            }
        }
        for (&province, neighbors) in graph {
            self.emit_province(
                network,
                province,
                neighbors,
                |other| crossings.get(&pair(other, province)).copied(),
                &mut mesh,
                heights,
            )?;
        }
        if mesh.native_vertices.is_empty() {
            mesh.bounds = [0.0; 4];
        }
        Ok(mesh)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RailMesh {
    pub fn bit_eq(&self, other: &Self) -> bool {
        let floats = |a: &[f32], b: &[f32]| {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
        };
        let bridge = |a: &Bridge, b: &Bridge| {
            floats(&a.position, &b.position)
                && a.rotation.to_bits() == b.rotation.to_bits()
                && a.kind == b.kind
        };
        #[cfg(test)]
        let test = bytemuck::cast_slice::<RailVertex, u8>(&self.vertices)
            == bytemuck::cast_slice::<RailVertex, u8>(&other.vertices)
            && self.indices == other.indices;
        #[cfg(not(test))]
        let test = true;
        test && (self.segments, self.unanchored, self.samples)
            == (other.segments, other.unanchored, other.samples)
            && floats(&self.bounds, &other.bounds)
            && self.native_runs == other.native_runs
            && floats(
                self.native_vertices.as_flattened(),
                other.native_vertices.as_flattened(),
            )
            && self.bridges.len() == other.bridges.len()
            && self
                .bridges
                .iter()
                .zip(&other.bridges)
                .all(|(a, b)| bridge(a, b))
    }
}

fn stub(start: Crossing, center: [f32; 3]) -> Curve {
    let end = Crossing {
        position: add(center, sub(start.position, center), 0.2),
        direction: normalized(sub(start.position, center)),
        half_width: 0.0,
        half_gap: 0.0,
    };
    Curve::new(start, end)
}

struct Curve {
    start: Crossing,
    end: Crossing,
    controls: [[f32; 3]; 4],
    length: f32,
    curve_length: f32,
}
impl Curve {
    fn new(start: Crossing, end: Crossing) -> Self {
        let a = add(start.position, start.direction, start.half_width);
        let b = add(end.position, end.direction, end.half_width);
        let handle = (length(sub(b, a)) * 0.5).min(3.0);
        let controls = [
            a,
            add(a, start.direction, handle),
            add(b, end.direction, handle),
            b,
        ];
        let mut out = Self {
            start,
            end,
            controls,
            length: 0.0,
            curve_length: 0.0,
        };
        let mut prev = a;
        for i in 1..=16 {
            let next = out.bezier(i as f32 / 16.0);
            out.curve_length += length(sub(next, prev));
            prev = next;
        }
        out.length = out.curve_length + start.half_width + end.half_width;
        out
    }
    fn bezier(&self, t: f32) -> [f32; 3] {
        let s = 1.0 - t;
        let k = [s * s * s, s * 3.0 * s * t, s * 3.0 * t * t, t * t * t];
        std::array::from_fn(|i| {
            self.controls[0][i] * k[0]
                + self.controls[1][i] * k[1]
                + self.controls[2][i] * k[2]
                + self.controls[3][i] * k[3]
        })
    }
    fn samples(&self) -> Vec<f32> {
        let inv = 1.0 / self.length;
        let begin = self.start.half_width * inv;
        let finish = self.end.half_width * inv;
        let curved = self.curve_length * inv;
        let bend = ((1.0 / (self.length * 0.2) + 1.0)
            * (dot(self.start.direction, self.end.direction) + 1.0)
            * 0.5)
            .min(1.0)
            * -0.35;
        let count = ((self.curve_length / (bend + 0.5)) as usize).max(2);
        let mut ts = Vec::with_capacity(count + 2);
        if begin > 0.0 {
            ts.push(self.start.half_gap * inv);
        }
        for i in 0..count {
            ts.push(i as f32 / (count - 1) as f32 * curved + begin);
        }
        if finish > 0.0 {
            ts.push(1.0 - self.end.half_gap * inv);
        }
        ts
    }

    fn center(&self, t: f32) -> [f32; 3] {
        let inv = 1.0 / self.length;
        let begin = self.start.half_width * inv;
        let finish = self.end.half_width * inv;
        let curved = self.curve_length * inv;
        if t < begin {
            add(
                self.start.position,
                sub(self.controls[0], self.start.position),
                t / begin,
            )
        } else if finish > 0.0 && 1.0 - t < finish {
            add(
                self.end.position,
                sub(self.controls[3], self.end.position),
                (1.0 - t) / finish,
            )
        } else {
            self.bezier(((t - begin) / curved).clamp(0.0, 1.0))
        }
    }

    fn centers(&self) -> Vec<[f32; 3]> {
        if self.length < 1e-5 || self.curve_length < 1e-5 {
            return Vec::new();
        }
        self.samples().into_iter().map(|t| self.center(t)).collect()
    }

    fn emit(
        &self,
        mesh: &mut RailMesh,
        level: i32,
        heights: &HeightField<'_>,
        defines: &RailDefines,
    ) -> io::Result<()> {
        if self.length < 1e-5 || self.curve_length < 1e-5 {
            return Ok(());
        }
        let inv = 1.0 / self.length;
        let begin = self.start.half_width * inv;
        let curved = self.curve_length * inv;
        let ts = self.samples();
        let mut vertices =
            engine_allocation::vec(engine_allocation::mul(ts.len(), 2)?)?;
        let repeats = self.length.round().max(1.0);
        let levels = defines.levels as i32;
        let band = (level.clamp(1, levels) - 1) as f32 / levels as f32;
        mesh.samples += ts.len();
        for t in ts {
            let u = ((t - begin) / curved).clamp(0.0, 1.0);
            let s = 1.0 - u;
            let center = self.center(t);
            let derivative: [f32; 3] = std::array::from_fn(|i| {
                self.controls[0][i] * (-3.0 * s * s)
                    + self.controls[1][i] * (3.0 * s * s - 6.0 * s * u)
                    + self.controls[2][i] * (6.0 * s * u - 3.0 * u * u)
                    + self.controls[3][i] * (3.0 * u * u)
            });
            let normal = normalized([derivative[2], 0.0, -derivative[0]]);
            let y = rail_height(heights, center[0], center[2]) + defines.y_offset;
            for (side, v) in [
                (RAIL_WIDTH * 0.5, band),
                (-RAIL_WIDTH * 0.5, band + 1.0 / levels as f32),
            ] {
                let mut position = add(center, normal, side);
                position[1] = y;
                mesh.bounds[0] = mesh.bounds[0].min(position[0]);
                mesh.bounds[1] = mesh.bounds[1].min(position[2]);
                mesh.bounds[2] = mesh.bounds[2].max(position[0]);
                mesh.bounds[3] = mesh.bounds[3].max(position[2]);
                vertices.push(RailVertex {
                    position,
                    uv: [t * repeats, v],
                });
            }
        }
        let pack = |v: &RailVertex| {
            [
                v.position[0],
                v.position[1],
                v.position[2],
                1.0,
                v.uv[0],
                v.uv[1],
            ]
        };
        engine_allocation::reserve(&mut mesh.native_vertices, vertices.len() + 4)?;
        mesh.native_vertices.extend([pack(&vertices[0]); 2]);
        mesh.native_vertices.extend(vertices.iter().map(pack));
        mesh.native_vertices
            .extend([pack(vertices.last().expect("curve samples")); 2]);
        #[cfg(test)]
        {
            let base = mesh.vertices.len() as u32;
            let count = vertices.len() as u32 / 2;
            mesh.vertices.extend(vertices);
            for i in 0..count.saturating_sub(1) {
                let q = base + i * 2;
                mesh.indices
                    .extend_from_slice(&[q, q + 1, q + 2, q + 2, q + 1, q + 3]);
            }
        }
        Ok(())
    }
}


pub struct RailAssets {
    pub shader: Arc<shader_pipeline::library::Effect>,
    pub texture: dds::Texture,
}

impl RailAssets {
    pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Self> {
        Ok(Self {
            shader: super::rail_shader::load(layers, map, decode)?,
            texture: layers.artwork(RAILROAD_TEXTURE, |stack| {
                load_uploadable(
                    RAILROAD_TEXTURE,
                    &|path| stack.read_uncovered_path(path),
                    decode,
                    ColorSpace::Srgb,
                )
            })?,
        })
    }
}

#[derive(Debug)]
pub struct Bridges {
    id: u64,
    pub library: ModelLibrary,
    entities: [String; 2],
}
impl Bridges {
    pub fn projection_id(&self) -> u64 {
        self.id
    }
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &content_reader_defines::defines::Defines,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
    ) -> io::Result<Self> {
        let entities = [
            defines.text("NGraphics", "RAILWAY_BRIDGE_ENTITY")?,
            defines.text("NGraphics", "RAILWAY_BRIDGE_LARGE_ENTITY")?,
        ];
        let roster = layers.descriptor_roster()?;
        Ok(Self {
            id: viewer_scene_base::next_projection_id(),
            library: ModelLibrary::load(layers, decode, roster.descriptor_job().view(), &entities, warnings)?,
            entities,
        })
    }

    fn entity(&self, kind: BridgeKind) -> &str {
        match kind {
            BridgeKind::Large => &self.entities[1],
            _ => &self.entities[0],
        }
    }
    pub fn field(&self, mesh: &RailMesh) -> ModelField {
        self.field_of(&mesh.bridges)
    }

    pub fn field_of<'b>(&self, bridges: impl IntoIterator<Item = &'b Bridge>) -> ModelField {
        let placed = bridges
            .into_iter()
            .filter_map(|bridge| {
                self.library.slot(self.entity(bridge.kind)).map(|slot| {
                    (
                        slot,
                        ModelInstance {
                            position: bridge.position,
                            rotation: bridge.rotation,
                            scale: 1.0,
                            pitch: 0.0,
                            train_clip: [0.0; 4],
                            ..ModelInstance::default()
                        },
                    )
                })
            })
            .collect();
        ModelField::build(&self.library, placed, 64.0)
    }
}


pub fn strip_indices(runs: &[Range<u32>]) -> Vec<u32> {
    let mut indices =
        Vec::with_capacity(runs.iter().map(|run| run.len() + 1).sum::<usize>());
    for run in runs {
        indices.extend(run.clone());
        indices.push(u32::MAX);
    }
    indices
}

fn strips(device: &wgpu::Device, runs: &[Range<u32>]) -> io::Result<Option<(wgpu::Buffer, u32)>> {
    if runs.is_empty() {
        return Ok(None);
    }
    let indices = strip_indices(runs);
    let count = u32::try_from(indices.len()).map_err(|_| invalid("rail strip indices exceed u32"))?;
    let indices = buffer(
        device,
        "native rail strip indices",
        wgpu::BufferUsages::INDEX,
        bytemuck::cast_slice(&indices),
    )?;
    Ok(Some((indices, count)))
}

#[derive(Default)]
pub struct PreparedRailGeometry {
    native_vertices: Option<wgpu::Buffer>,
    strips: Option<(wgpu::Buffer, u32)>,
}

impl PreparedRailGeometry {
    pub fn new(device: &wgpu::Device, mesh: &RailMesh) -> io::Result<Self> {
        let native_vertices = if mesh.native_vertices.is_empty() {
            None
        } else {
            let usage = wgpu::BufferUsages::VERTEX;
            #[cfg(test)]
            let usage = usage | wgpu::BufferUsages::COPY_SRC;
            Some(buffer(
                device,
                "native rail strips",
                usage,
                bytemuck::cast_slice(&mesh.native_vertices),
            )?)
        };
        let strips = match &native_vertices {
            Some(_) => strips(device, &mesh.native_runs)?,
            None => None,
        };
        Ok(Self { native_vertices, strips })
    }

    pub fn held(vertices: Option<wgpu::Buffer>, strips: Option<(wgpu::Buffer, u32)>) -> Self {
        Self {
            strips: strips.filter(|_| vertices.is_some()),
            native_vertices: vertices,
        }
    }

    pub fn for_hour(
        device: Option<&wgpu::Device>,
        mesh: &RailMesh,
    ) -> io::Result<Option<Arc<Self>>> {
        device
            .map(|device| Self::new(device, mesh).map(Arc::new))
            .transpose()
    }

    pub fn for_install(
        device: &wgpu::Device,
        mesh: &RailMesh,
        prepared: Option<&Arc<Self>>,
    ) -> io::Result<Arc<Self>> {
        match prepared {
            Some(geometry) => Ok(Arc::clone(geometry)),
            None => Ok(Self::for_hour(Some(device), mesh)?.expect("device supplied")),
        }
    }
}

pub struct RailPass {
    effect: Arc<shader_pipeline::library::Effect>,
    compiled: RefCell<Option<Result<super::rail_shader::Rail, String>>>,
    geometry: Arc<PreparedRailGeometry>,
    null: wgpu::TextureView,
    samples: u32,
    format: wgpu::TextureFormat,
    rail: Bound,
}

impl RailPass {
    pub fn texel_reads(&self, visit: &mut dyn FnMut(&str)) {
        if let Some(Ok(rail)) = &*self.compiled.borrow()
            && let Some(reads) = &rail.texel_reads
        {
            visit(reads);
        }
    }

    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: &RailAssets,
    ) -> io::Result<Self> {
        let null = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("railway null SRVs"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Ok(Self {
            effect: assets.shader.clone(),
            compiled: RefCell::new(None),
            geometry: Arc::default(),
            null,
            samples,
            format,
            rail: upload(
                device,
                queue,
                "RailTexture (railroad.dds)",
                &assets.texture,
                ColorSpace::Srgb,
            )?,
        })
    }

    pub fn adopt_geometry(
        &mut self,
        geometry: Arc<PreparedRailGeometry>,
    ) -> Arc<PreparedRailGeometry> {
        std::mem::replace(&mut self.geometry, geometry)
    }

    pub fn textures(&self) -> [&wgpu::TextureView; 2] {
        [&self.rail.view, &self.null]
    }

    pub fn warm(&self, device: &wgpu::Device) {
        let mut compiled = self.compiled.borrow_mut();
        if compiled.is_none() {
            *compiled = Some(
                super::rail_shader::Rail::new(
                    device,
                    &self.effect,
                    self.samples,
                    self.format,
                    DEPTH_FORMAT,
                )
                .map_err(|error| error.to_string()),
            );
        }
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        inputs: super::rail_shader::Inputs<'_>,
    ) -> io::Result<()> {
        let device = arena.device();
        if inputs.alpha < RailFade::TRAINS_VISIBLE {
            return Ok(());
        }
        let Some(vertices) = &self.geometry.native_vertices else {
            return Ok(());
        };
        self.warm(device);
        let compiled = self.compiled.borrow();
        viewer_gpu::gpu_contract::compiled(compiled.as_ref().expect("compiled above"))?
            .draw(
                arena,
                encoder,
                targets,
                inputs,
                super::rail_shader::Geometry {
                    vertices,
                    strips: self
                        .geometry
                        .strips
                        .as_ref()
                        .map(|(indices, count)| (indices, *count)),
                },
            )
    }
}

#[cfg(test)]
mod tests;
