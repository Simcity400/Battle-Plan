//! Railway strips from the recorded graph.
//!
//! Join neighbour pairs within provinces, excluding triangles, at the lower edge level. Degree-one
//! provinces and supply sources also receive spurs. Cubics use three-unit handles and straight bridge
//! ends. One-unit strips sample centre terrain height; zoom changes opacity alone.
//!
//! Static crossings use province boundary pixels and smoothed river points rather than authored
//! starting railway paths.

use super::{
    borders::HeightField,
    camera::Camera,
    models::{ModelField, ModelInstance, ModelLibrary, WATER_HEIGHT},
    scene::{DEPTH_FORMAT, buffer},
    textures::{Bound, ColorSpace, upload},
};
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use installed_content::image::native::Decode;
use installed_content::{
    ResolvedLayers, buildings::Buildings, image::dds, map::MapData, positions::BuildingPosition,
};
use replay::{
    frame::DecodedFrame,
    rows::{NavalBaseRow, ProvinceSupplyRow, RailEdgeRow, tables},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    ops::Range,
    sync::Arc,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Offsets -0.5 and +0.5, independent of camera.
pub const RAIL_WIDTH: f32 = 1.0;
pub const RAILROAD_TEXTURE: &str = "gfx/models/supply/railroad.dds";

/// Railway opacity advances each frame: at or above `RAILWAY_CAMERA_CUTOFF`, it falls toward zero
/// at `RAILWAY_CAMERA_CUTOFF_SPEED`; below, it rises toward one at the same rate. The strip uses
/// this opacity. Trains draw at or above [`RailFade::TRAINS_VISIBLE`] with it as TrainColor alpha.
/// The first frame, a still dump or a backwards clock uses the settled value, as a replay seek does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RailFade {
    alpha: f32,
    last: Option<f32>,
}

impl RailFade {
    /// The minimum opacity that draws trains.
    pub const TRAINS_VISIBLE: f32 = 0.001;

    /// Steps the fade to `time` seconds and returns the opacity.
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

/// The content's railway geometry operands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailDefines {
    /// `NGraphics.RAILWAY_Y_OFFSET`.
    pub y_offset: f32,
    /// `RAILWAY_BRIDGE_Y_OFFSET`.
    pub bridge_y_offset: f32,
    /// Half of `RAILWAY_BRIDGE_WIDTH` and `RAILWAY_BRIDGE_LARGE_WIDTH`: the
    /// straight run each bridge end keeps.
    pub half_widths: [f32; 2],
    /// Half of `RAILWAY_BRIDGE_GAP_WIDTH` and `RAILWAY_BRIDGE_GAP_LARGE_WIDTH`.
    pub half_gaps: [f32; 2],
    /// `NSupply.MAX_RAILWAY_LEVEL`: the texture's level bands.
    pub levels: u32,
}

impl RailDefines {
    pub fn load(defines: &installed_content::defines::Defines) -> io::Result<Self> {
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

    /// (straight half width, half gap) of a bridge end.
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

/// Authored adjacency kind used for bridge selection.
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
    /// The bridge end's straight half width and half gap; zero off a bridge.
    half_width: f32,
    half_gap: f32,
}
#[derive(Clone, Debug)]
struct Boundary {
    points: Vec<[i32; 2]>,
    kind: BridgeKind,
}

type RiverPoint = ([f32; 2], [f32; 2]);

/// Static map input, cached once; per-hour work scales with that hour's edges.
pub struct RailGeometry {
    defines: RailDefines,
    anchors: BTreeMap<u32, [f32; 3]>,
    ports: BTreeMap<u32, [f32; 3]>,
    boundaries: BTreeMap<(u32, u32), Boundary>,
    river_points: BTreeMap<u32, Vec<RiverPoint>>,
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

/// Bilinear height at half-pixel-centred x/z coordinates, clamped to water when requested.
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

/// Bresenham sorted along the major axis, ties step upward.
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

/// Largest eight-connected boundary component.
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
    // The native mean accumulates fixed5 per-point quotients, then rounds.
    std::array::from_fn(|i| {
        let sum: i64 = best
            .iter()
            .map(|p| i64::from(p[i]) * 100000 / best.len() as i64)
            .sum();
        ((sum + 50000) / 100000) as i32
    })
}

/// Recorded rail graph and geometry inputs for one hour.
///
/// It is everything [`RailGeometry::build_network`] reads from an hour, so two
/// hours with equal networks draw the same railway.
#[derive(Clone, Debug, PartialEq)]
pub struct RailNetwork {
    /// Directed neighbours and their levels (`RailEdges`).
    graph: BTreeMap<u32, BTreeMap<u32, i32>>,
    /// Provinces that get a terminal spur per neighbour.
    sources: BTreeSet<u32>,
    /// Provinces present in the strategic-navy base map, anchored at the harbour.
    ports: BTreeSet<u32>,
}

impl RailNetwork {
    /// The hour's network from its `RailEdges`, `ProvinceSupply` and
    /// `NavalBases` rows.
    pub fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let edges = super::tables::rows::<tables::RailEdges>(frame)?;
        let supply = super::tables::rows::<tables::ProvinceSupply>(frame)?;
        let naval_bases = super::tables::rows::<tables::NavalBases>(frame)?;
        if !naval_bases.is_complete() {
            // Absent hash members could select a different graphics anchor;
            // a truncated list cannot safely fall back to province centroids.
            return Ok(Self::new(&[], &[], &[]));
        }
        Ok(Self::new(edges.rows(), supply.rows(), naval_bases.rows()))
    }

    /// RailEdges is a directed neighbour list. Never reconstruct missing rows
    /// from the 1936 file, or turn a missing table into a full network.
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
        // A present strategic-navy payload selects the harbour graphics anchor even when the naval
        // base has zero level.
        let ports = naval_bases.iter().map(|r| r.province).collect();
        Self {
            graph,
            sources,
            ports,
        }
    }

    /// Whether the province has the requested rail segment between neighbours (excluding triangles),
    /// or a terminal spur when other is None.
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

/// The rail centre line a train follows, and whether it stops short of its
/// last route province because that province's railway lacks the segment.
#[derive(Debug, PartialEq)]
pub struct TrainRoute {
    pub points: Vec<[f32; 3]>,
    pub open_end: bool,
}

impl RailGeometry {
    /// Anchors come from the slots of the hub types (`supply_node`) and ports
    /// from the `is_port` spawn buckets, as content declares them.
    pub fn new(
        map: &MapData,
        buildings: &[BuildingPosition],
        types: &Buildings,
        rivers: &installed_content::image::IndexedImage,
        heights: &HeightField<'_>,
        defines: RailDefines,
    ) -> io::Result<Self> {
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
        // Visit bottom-up rows; each boundary pixel belongs to the lower
        // province, once even when two of its neighbours have the same id.
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
            // An ambiguous slot anchors nothing; MapBuildingAssets names it.
            let Some(installed_content::buildings::Slot::Bucket(slot)) = types.slot(&p.kind) else {
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
                Some(installed_content::map::ProvinceKind::Land)
            ) || !matches!(
                map.definitions.get(&b).map(|d| d.kind),
                Some(installed_content::map::ProvinceKind::Land)
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
            // Native swaps only the first segment's endpoints, then overwrites
            // its last element with the entire major-axis-sorted second line.
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
        // Railways share the map-order graphics cache and extend the nearest end with preserved
        // swap-removal ties.
        for boundary in boundaries.values_mut() {
            let pixels: Vec<_> = boundary
                .points
                .iter()
                .map(|p| [p[0] as f32, p[1] as f32])
                .collect();
            boundary.points = super::orders::sort_edge_pixels(&pixels)
                .into_iter()
                .map(|p| [p[0] as i32, p[1] as i32])
                .collect();
        }
        let traced = super::rivers::trace(&super::rivers::RiverImage {
            width: rivers.width,
            height: rivers.height,
            indices: &rivers.indices,
        })?;
        let mut river_points: BTreeMap<u32, Vec<RiverPoint>> = BTreeMap::new();
        for river in traced.rivers {
            let mut points = river.points;
            // Apply one in-place five-point average while retaining the first and last two points. Later
            // samples read updated predecessors.
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
            defines,
            anchors,
            ports,
            boundaries,
            river_points,
        })
    }

    /// Where the rail crosses the boundary between the pair `key`, and the
    /// tangent it crosses on: the river crossing itself on a bridge, otherwise
    /// the straight line between the two anchors.
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
        // Bilinearly sample terrain at half-pixel-shifted coordinates.
        position[1] = rail_height(heights, position[0] - 1.0, position[2] - 1.0);
        let (half_width, half_gap) = self.defines.bridge(kind);
        Crossing {
            position,
            direction: tangent,
            half_width,
            half_gap,
        }
    }

    /// The rail centre line a train follows over `path`.
    ///
    /// Cache the rail segment connecting each route province's previous and next province. Terminal
    /// provinces use a reversed stub toward their anchor. Trains follow drawn rails and water-floored
    /// heights without the rail strip's vertical lift.
    ///
    /// A province whose railway lacks the segment caches none:
    /// hides cars there and destroys the train once its head
    /// province has none. The route therefore ends before that province
    /// (`open_end`), and is `None` if the first province lacks its spur.
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
        // The crossing's stored direction runs from the lower province id to
        // the higher one; a segment's ends both point into their own province.
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

    /// A naval base's province is anchored at its harbour.
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
        heights: &HeightField<'_>,
    ) -> RailMesh {
        self.build_network(&RailNetwork::new(edges, supply, naval_bases), heights)
    }

    /// The railway of one hour's network.
    pub fn build_network(&self, network: &RailNetwork, heights: &HeightField<'_>) -> RailMesh {
        let RailNetwork { graph, sources, .. } = network;
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
        for (&(a, b), c) in &crossings {
            let kind = self
                .boundaries
                .get(&(a, b))
                .map_or(BridgeKind::None, |boundary| boundary.kind);
            if kind != BridgeKind::None
                && (graph.get(&a).is_some_and(|n| n.contains_key(&b))
                    || graph.get(&b).is_some_and(|n| n.contains_key(&a)))
            {
                mesh.bridges.push(Bridge {
                    position: add(c.position, [0.0, 1.0, 0.0], self.defines.bridge_y_offset),
                    rotation: c.direction[2].atan2(c.direction[0]) + std::f32::consts::FRAC_PI_2,
                    kind,
                });
            }
        }
        for (&province, neighbors) in graph {
            let first = mesh.native_vertices.len() as u32;
            let ns: Vec<_> = neighbors.iter().collect();
            for (i, &(&a, &level)) in ns.iter().enumerate() {
                let start = crossings.get(&pair(a, province)).copied().map(|mut c| {
                    if a > province {
                        c.direction = c.direction.map(|v| -v);
                    }
                    c
                });
                for &(&b, &other_level) in &ns[i + 1..] {
                    if graph.get(&b).is_some_and(|n| n.contains_key(&a)) {
                        continue;
                    }
                    let end = crossings.get(&pair(b, province)).copied().map(|mut c| {
                        if b > province {
                            c.direction = c.direction.map(|v| -v);
                        }
                        c
                    });
                    mesh.segments += 1;
                    if let (Some(start), Some(end)) = (start, end) {
                        Curve::new(start, end).emit(
                            &mut mesh,
                            level.min(other_level),
                            heights,
                            &self.defines,
                        );
                    } else {
                        mesh.unanchored += 1;
                    }
                }
                if ns.len() == 1 || sources.contains(&province) {
                    mesh.segments += 1;
                    if let (Some(start), Some(&center)) = (start, anchor(province)) {
                        stub(start, center).emit(&mut mesh, level, heights, &self.defines);
                    } else {
                        mesh.unanchored += 1;
                    }
                }
            }
            let end = mesh.native_vertices.len() as u32;
            if first != end {
                mesh.native_runs.push(first..end);
            }
        }
        if mesh.native_vertices.is_empty() {
            mesh.bounds = [0.0; 4];
        }
        mesh
    }
}

/// A dead end's short run from a boundary crossing towards the anchor.
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
    /// The parameters the pass samples along the curve, bridge gaps included.
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

    /// The centre line at `t`: straight over a bridge's half-width, else Bézier.
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

    /// The centre line, one point per sample; empty for a degenerate curve.
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
    ) {
        if self.length < 1e-5 || self.curve_length < 1e-5 {
            return;
        }
        let inv = 1.0 / self.length;
        let begin = self.start.half_width * inv;
        let curved = self.curve_length * inv;
        let ts = self.samples();
        let mut vertices = Vec::new();
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
        // Emit world positions with w equal to one and two texture coordinates. Repeat each route
        // endpoint twice to join nonindexed triangle strips.
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
    }
}

// --------------------------------------------------------------------- assets

/// What the railway pass reads from installed content.
pub struct RailAssets {
    pub shader: Arc<shader_pipeline::library::Effect>,
    /// `gfx/models/supply/railroad.dds`, the five level bands.
    pub texture: dds::Texture,
}

impl RailAssets {
    /// Model textures resolve through recorded content and retain existing verification limits shared
    /// by tree and building meshes.
    pub fn load(layers: &ResolvedLayers, map: [u32; 2], decode: Decode) -> io::Result<Self> {
        Ok(Self {
            shader: super::rail_shader::load(layers, map, decode)?,
            texture: super::textures::load_file(
                RAILROAD_TEXTURE,
                &|path| layers.read_uncovered_path(path),
                decode,
            )?,
        })
    }
}

/// Static model library; placement follows each recorded hour's edges.
#[derive(Debug)]
pub struct Bridges {
    pub library: ModelLibrary,
    /// `NGraphics.RAILWAY_BRIDGE_ENTITY` and `RAILWAY_BRIDGE_LARGE_ENTITY`.
    entities: [String; 2],
}
impl Bridges {
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        defines: &installed_content::defines::Defines,
        warnings: &mut Vec<installed_content::Diagnostic>,
    ) -> io::Result<Self> {
        let entities = [
            defines.text("NGraphics", "RAILWAY_BRIDGE_ENTITY")?,
            defines.text("NGraphics", "RAILWAY_BRIDGE_LARGE_ENTITY")?,
        ];
        let roster = layers.descriptor_roster()?;
        Ok(Self {
            library: ModelLibrary::load(layers, decode, &roster.descriptors, &entities, warnings)?,
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
        let placed = mesh
            .bridges
            .iter()
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

// ------------------------------------------------------------------- the pass

/// The GPU railway pass.
pub struct RailPass {
    effect: Arc<shader_pipeline::library::Effect>,
    compiled: RefCell<Option<Result<super::rail_shader::Rail, String>>>,
    native_vertices: Option<wgpu::Buffer>,
    native_runs: Vec<Range<u32>>,
    null: wgpu::TextureView,
    samples: u32,
    format: wgpu::TextureFormat,
    rail: Bound,
}

impl RailPass {
    /// Builds the pipeline and uploads the rail texture.
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
            native_vertices: None,
            native_runs: Vec::new(),
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

    /// Uploads the recorded hour's railway strips, one run per province.
    pub fn upload(&mut self, device: &wgpu::Device, mesh: &RailMesh) -> io::Result<()> {
        self.native_runs.clone_from(&mesh.native_runs);
        self.native_vertices = if mesh.native_vertices.is_empty() {
            None
        } else {
            Some(buffer(
                device,
                "native rail strips",
                wgpu::BufferUsages::VERTEX,
                bytemuck::cast_slice(&mesh.native_vertices),
            )?)
        };
        Ok(())
    }

    pub fn textures(&self) -> [&wgpu::TextureView; 2] {
        [&self.rail.view, &self.null]
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: super::border_shader::Target<'_>,
        inputs: super::rail_shader::Inputs<'_>,
    ) -> io::Result<()> {
        let device = arena.device();
        // The train visibility threshold also gates railway submission.
        if inputs.alpha < RailFade::TRAINS_VISIBLE {
            return Ok(());
        }
        let Some(vertices) = &self.native_vertices else {
            return Ok(());
        };
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
        super::scene::passes::compiled(compiled.as_ref().expect("created above"))?.draw(
            arena,
            encoder,
            target,
            inputs,
            super::rail_shader::Geometry {
                vertices,
                runs: &self.native_runs,
            },
        )
    }
}

#[cfg(test)]
mod tests;
