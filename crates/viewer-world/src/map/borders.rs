//! Border meshes traced along province boundaries.
//!
//! The province-id image is traced once into triangle strips. Each rail vertex
//! samples terrain height; compiled content supplies the border artwork.
//! Four-connected components preserve disconnected province pieces and UV
//! direction. Contours emit only edges against earlier labels, with ordered
//! junction corners, closure, smoothing and widening.
//!
//! [`build`] returns CPU vertex/index buffers for later upload. [`class_alpha`]
//! supplies the per-frame fade used by the draw pass.
use std::{collections::HashMap, io};

use super::line_lod;

use viewer_foundation::map::province_map::{HeightField, ProvinceMap};
#[cfg(test)]
use viewer_foundation::map::province_map::{WATER_HEIGHT, world_from_pixel};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Maximum smoothing radius for a water-to-water chain.
pub const WIDE_SMOOTHING_RADIUS: usize = 6;
/// Vertices per vertex buffer chunk, the `0x10000` the engine passes its mesh builder
/// Indices stay relative to a draw's base vertex, so they fit in `u16`.
pub const MAX_CHUNK_VERTICES: usize = 65536;

/// The six border classes, in the engine's own bucket order.
///
/// Material order shared by texture lookup, renderer buckets and the custom highlight palette.
/// Highlighted edges use buckets 6 through 11. DMZ state comes from replay; faction hover and its
/// clock belong to presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BorderClass {
    Province,
    State,
    Country,
    Impassable,
    Sea,
    SeaRegion,
}

impl BorderClass {
    /// Every class, in bucket order.
    pub const ALL: [BorderClass; 6] = [
        BorderClass::Province,
        BorderClass::State,
        BorderClass::Country,
        BorderClass::Impassable,
        BorderClass::Sea,
        BorderClass::SeaRegion,
    ];

    /// The class's bucket index, 0 to 5.
    pub fn bucket(self) -> usize {
        match self {
            BorderClass::Province => 0,
            BorderClass::State => 1,
            BorderClass::Country => 2,
            BorderClass::Impassable => 3,
            BorderClass::Sea => 4,
            BorderClass::SeaRegion => 5,
        }
    }

    /// The infix of the class's texture, `map/terrain/border_<name>_<quality>.dds`.
    pub fn texture_name(self) -> &'static str {
        match self {
            BorderClass::Province => "province",
            BorderClass::State => "state",
            BorderClass::Country => "country",
            BorderClass::Impassable => "impassable",
            BorderClass::Sea => "sea",
            BorderClass::SeaRegion => "sea_region",
        }
    }
}

/// The per-frame alpha for a class at a camera height, the value that becomes
/// `vTime_Transparency.y`.
///
/// Province and state borders fade out between the `PROVINCE_BORDER_FADE_*` and
/// `STATE_BORDER_FADE_*` heights in `fades`; every other class draws at full alpha. The engine additionally drops a province edge
/// entirely once its alpha reaches zero, which is the drawing pass's decision, not this function's.
/// A camera height that is not a number is treated as fully near.
///
/// `province_borders` is the map mode's choice ([`crate::map::map_mode::visibility`]): the
/// resources, infrastructure and manpower modes give province borders no alpha at any height.
pub fn class_alpha(
    class: BorderClass,
    camera_height: f32,
    fades: [[f32; 2]; 2],
    province_borders: bool,
) -> f32 {
    match class {
        BorderClass::Province if !province_borders => 0.0,
        BorderClass::Province => fade(camera_height, fades[0][0], fades[0][1]),
        BorderClass::State => fade(camera_height, fades[1][0], fades[1][1]),
        _ => 1.0,
    }
}

fn fade(height: f32, near: f32, far: f32) -> f32 {
    if height.is_nan() || height <= near {
        return 1.0;
    }
    if height >= far {
        return 0.0;
    }
    1.0 - (height - near) / (far - near)
}

/// One traced province boundary, before smoothing and widening.
///
/// `points` are in world XZ. Most sit on pixel crack midpoints, so consecutive points are one map
/// pixel apart along an axis and `sqrt(0.5)` apart around a corner; the rest are **junction
/// corners**, the lattice corners where three or more provinces meet, which
/// [`BoundaryComponents::is_junction`] finds and [`trace`] inserts. A closed polyline repeats its first point
/// as its last, the way the engine's seam splice closes a single-pair contour.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    /// The lower of the two province ids the boundary separates.
    pub province_a: u32,
    /// The higher of the two.
    pub province_b: u32,
    pub class: BorderClass,
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
    /// Both sides are water, which selects the engine's wide two-pass smoothing; see [`smooth`].
    pub wide: bool,
}

/// What [`trace`] found: ordinary classified boundaries, dormant material-0xff
/// boundaries and the degenerate provinces it diagnoses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    pub polylines: Vec<Polyline>,
    /// Native material 0xff: ordinary buckets omit these, but a custom highlight
    /// remaps them to the Province secondary bank.
    pub hidden: Vec<Polyline>,
    /// Provinces covering exactly one pixel, sorted. The native "One-pixel province color found"
    /// message diagnoses these without excluding their geometry.
    pub one_pixel_provinces: Vec<u32>,
}

/// A border strip vertex: `float3 position` then `6G16_SINT uv`, the engine's 16 byte layout.
///
/// `uv.x` is 0 on one rail and 1 on the other and becomes the shader's V coordinate across the
/// strip; `uv.y` counts points along the polyline and becomes the U coordinate, which with
/// `BORDER_TILE` (the content `border.shader`) and an `AddressU = Wrap` sampler tiles the diffuse every 2.5 points. It is a
/// 16 bit counter in the engine too, so a polyline longer than 32 767 points wraps it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [i16; 2],
}

/// One drawable run of a border strip: `BorderDesc` in the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderDraw {
    /// The lower of the two province ids this boundary separates.
    pub province_a: u32,
    /// The higher of the two.
    pub province_b: u32,
    /// Index into the owning mesh's `chunks`.
    pub chunk: u32,
    /// First vertex of the run within that chunk; the shared index buffer is relative to it.
    pub base_vertex: u32,
    /// `2 * points`.
    pub vertex_count: u32,
    /// `(points - 1) * 6`, taken from the start of [`BorderMeshes::indices`].
    pub index_count: u32,
    /// The same run at the coarse level ([`super::line_lod`]): its first vertex in
    /// [`BorderMesh::coarse_chunks`]`[chunk]`, and `2 * points`.
    pub coarse_base_vertex: u32,
    pub coarse_vertex_count: u32,
}

/// Every strip of one class.
///
/// The engine keeps one vertex buffer list for the whole map and sorts edges into class buckets
/// afterwards; this splits the chunks per class instead, because a class is what a draw call binds.
/// The chunking rule is the engine's either way.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderMesh {
    pub class: BorderClass,
    /// Vertex buffers of at most [`MAX_CHUNK_VERTICES`] vertices each.
    pub chunks: Vec<Vec<Vertex>>,
    /// The coarse level of each chunk's runs, chunk for chunk.
    pub coarse_chunks: Vec<Vec<Vertex>>,
    pub draws: Vec<BorderDraw>,
}

impl BorderMesh {
    fn new(class: BorderClass) -> Self {
        Self {
            class,
            chunks: vec![Vec::new()],
            coarse_chunks: vec![Vec::new()],
            draws: Vec::new(),
        }
    }

    /// Total vertices over every chunk.
    pub fn vertex_count(&self) -> usize {
        self.chunks.iter().map(Vec::len).sum()
    }
}

/// The finished border geometry: one mesh per class and the single index buffer they share.
#[derive(Clone, Debug, PartialEq)]
pub struct BorderMeshes {
    /// Six meshes in [`BorderClass::ALL`] order.
    pub meshes: Vec<BorderMesh>,
    /// Unclassified strips, retained by the same trace for custom highlights only.
    pub hidden: BorderMesh,
    /// The shared quad pattern `(i, i + 2, i + 1), (i + 2, i + 3, i + 1)` repeated, long enough for
    /// the longest draw. A draw reads `index_count` of it from index zero and offsets by its own
    /// `base_vertex`.
    pub indices: Vec<u16>,
    /// Provinces of exactly one pixel, reported by the native diagnostic.
    pub one_pixel_provinces: Vec<u32>,
}

impl BorderMeshes {
    /// The mesh of one class.
    pub fn mesh(&self, class: BorderClass) -> &BorderMesh {
        &self.meshes[class.bucket()]
    }
}

/// Traces the native component contours.
///
/// A contour keeps its component on the left. At a diagonal contact, the next pixel is tried
/// diagonally to the right, then straight ahead, then the walk turns left in place. Only an edge
/// facing an earlier component is emitted, once for the pair. Runs split when the neighbouring
/// province changes or an edge is not emitted; disconnected pieces of the same neighbouring
/// province can belong to one run. A three-component corner contributes a point on both touching
/// edges. The first/last runs splice only when the start is not itself such a corner.
///
/// The scan begins at the bottom-left boundary pixel, preferring south/east/north/west edges.
/// This fixes the direction, UV origin and the unsmoothed endpoint of a closed contour. Geometry
/// is traced at load; the hour-dependent classifier and GPU pass choose its material later.
pub fn trace(
    map: &ProvinceMap,
    classify: &dyn Fn(u32, u32) -> Option<BorderClass>,
    water: &dyn Fn(u32) -> bool,
) -> io::Result<Trace> {
    map.validate()?;
    let components = BoundaryComponents::new(map);
    let mut done =
        vec![false; components.components.iter().copied().max().unwrap_or(0) as usize + 1];
    let mut polylines = Vec::new();
    let offsets = [[-1, 0], [0, -1], [1, 0], [0, 1]];
    let width = map.width as i32;
    let height = map.height as i32;
    let index = |x: i32, z: i32| ((height - 1 - z) * width + x) as usize;
    let inside = |x: i32, z: i32| x >= 0 && x < width && z >= 0 && z < height;
    for z in 0..height {
        for x in 0..width {
            let component = components.components[index(x, z)];
            if done[component as usize] {
                continue;
            }
            let Some(start_direction) = [1, 2, 3, 0].into_iter().find(|&d| {
                let [dx, dz] = offsets[d];
                inside(x + dx, z + dz) && components.components[index(x + dx, z + dz)] != component
            }) else {
                continue;
            };
            done[component as usize] = true;
            let province = map.ids[index(x, z)];
            let mut runs: Vec<(u32, Vec<[f32; 2]>)> = Vec::new();
            let mut active = false;
            let mut starts_emitting = false;
            let mut starts_at_junction = false;
            let mut first_step = true;
            let mut position = [x, z];
            let mut direction = start_direction;
            loop {
                let [px, pz] = position;
                let [dx, dz] = offsets[direction];
                let [tx, tz] = [-dz, dx];
                let nx = (px + dx).rem_euclid(width);
                let nz = pz + dz;
                if nz >= 0 && nz < height && components.components[index(nx, nz)] <= component {
                    let other = map.ids[index(nx, nz)];
                    if !active || runs.last().unwrap().0 != other {
                        runs.push((other, Vec::new()));
                    }
                    active = true;
                    if first_step {
                        starts_emitting = true;
                    }
                    let mut midpoint = [px as f32 + dx as f32 * 0.5, pz as f32 + dz as f32 * 0.5];
                    if midpoint[0] < 0.0 {
                        midpoint[0] += width as f32;
                    }
                    let points = &mut runs.last_mut().unwrap().1;
                    for (sign, is_before) in [(-1.0, true), (1.0, false)] {
                        let corner = [
                            midpoint[0] + sign * tx as f32 * 0.5,
                            midpoint[1] + sign * tz as f32 * 0.5,
                        ];
                        let cx = (corner[0] + 0.5) as u32;
                        let cy = (height as f32 - 0.5 - corner[1]) as u32;
                        if components.is_junction([cx, cy]) {
                            points.push(corner);
                            if first_step && is_before {
                                starts_at_junction = true;
                            }
                        }
                        if is_before {
                            points.push(midpoint);
                        }
                    }
                } else {
                    active = false;
                }
                first_step = false;
                let diagonal = [px + dx + tx, pz + dz + tz];
                let tangent = [px + tx, pz + tz];
                if inside(diagonal[0], diagonal[1])
                    && components.components[index(diagonal[0], diagonal[1])] == component
                {
                    position = diagonal;
                    direction = (direction + 3) % 4;
                } else if inside(tangent[0], tangent[1])
                    && components.components[index(tangent[0], tangent[1])] == component
                {
                    position = tangent;
                } else {
                    direction = (direction + 1) % 4;
                }
                if position == [x, z] && direction == start_direction {
                    break;
                }
            }
            if starts_emitting
                && !starts_at_junction
                && active
                && !runs.is_empty()
                && runs[0].0 == runs.last().unwrap().0
            {
                if runs.len() == 1 {
                    let points = &mut runs[0].1;
                    if points.first() != points.last() {
                        points.push(points[0]);
                    }
                } else {
                    // Keep the first descriptor, prepend the last run's points and remove the last
                    // descriptor from the pair's linked list.
                    let mut last = runs.pop().unwrap();
                    last.1.extend(std::mem::take(&mut runs[0].1));
                    runs[0] = last;
                }
            }
            for (other, points) in runs {
                if other == province {
                    continue;
                }
                let class = classify(province.min(other), province.max(other));
                let closed = points.first() == points.last();
                polylines.push((
                    (province, other),
                    class.is_none(),
                    Polyline {
                        province_a: province.min(other),
                        province_b: province.max(other),
                        class: class.unwrap_or(BorderClass::Province),
                        points,
                        closed,
                        wide: water(province) && water(other),
                    },
                ));
            }
        }
    }
    // Directed province pairs hold newest-first descriptor lists. Preserve this order at joins where
    // translucent strips overlap.
    polylines.reverse();
    polylines.sort_by_key(|(key, _, _)| *key);
    let mut hidden = Vec::new();
    let polylines = polylines
        .into_iter()
        .filter_map(|(_, unclassified, line)| {
            if unclassified {
                hidden.push(line);
                None
            } else {
                Some(line)
            }
        })
        .collect();
    Ok(Trace {
        polylines,
        hidden,
        one_pixel_provinces: one_pixel_provinces(map),
    })
}

/// Polyline smoothing with fixed endpoints.
///
/// Land and mixed chains receive one three-point mean pass. Water pairs receive two passes, with up
/// to six neighbours per side limited by distance to endpoints. First and last points remain fixed.
/// Each pass reads the previous pass's complete output.
///
/// Leaving the endpoints alone is what makes [`trace`]'s junction corners work: both chains that
/// meet at a corner keep that corner bit-identical, so their strips meet exactly.
///
/// Four pixel centres are averaged only for junction corners in trace; polyline smoothing is a
/// separate step.
pub fn smooth(points: &[[f32; 2]], wide: bool) -> Vec<[f32; 2]> {
    let count = points.len();
    if count < 3 {
        return points.to_vec();
    }
    let mut source = points.to_vec();
    for pass in 0..2 {
        if pass == 1 && !wide {
            break;
        }
        let mut out = source.clone();
        for (index, point) in out.iter_mut().enumerate().take(count - 1).skip(1) {
            let radius = if wide {
                index.min(count - 1 - index).min(WIDE_SMOOTHING_RADIUS)
            } else {
                1
            };
            let window = &source[index - radius..=index + radius];
            let mut sum = [0.0_f32; 2];
            for tap in window {
                sum[0] += tap[0];
                sum[1] += tap[1];
            }
            let taps = window.len() as f32;
            *point = [sum[0] / taps, sum[1] / taps];
        }
        source = out;
    }
    source
}

/// A point of the polyline, wrapped for a closed loop and mirror extrapolated past an open end.
fn extend(points: &[[f32; 2]], index: isize, closed: bool) -> [f32; 2] {
    let count = points.len() as isize;
    if closed {
        return points[index.rem_euclid(count) as usize];
    }
    if index < 0 {
        let [first, second] = [points[0], points[1]];
        let steps = (-index) as f32;
        return [
            first[0] + (first[0] - second[0]) * steps,
            first[1] + (first[1] - second[1]) * steps,
        ];
    }
    if index >= count {
        let last = points[count as usize - 1];
        let before = points[count as usize - 2];
        let steps = (index - count + 1) as f32;
        return [
            last[0] + (last[0] - before[0]) * steps,
            last[1] + (last[1] - before[1]) * steps,
        ];
    }
    points[index as usize]
}

/// Builds the border meshes for one map.
///
/// Traces with [`trace`], smooths each polyline once with [`smooth`], widens it to `width`
/// (`BORDER_WIDTH`) map units with a terrain height per rail vertex, and packs the result into one
/// mesh per class with chunks of at most [`MAX_CHUNK_VERTICES`] vertices.
///
/// Errors are named `InvalidData`: a province map whose id count does not match its dimensions, a
/// height field of a different size than the province map, or a height field whose sample count
/// does not match its own dimensions.
pub fn build(
    map: &ProvinceMap,
    classify: &dyn Fn(u32, u32) -> Option<BorderClass>,
    water: &dyn Fn(u32) -> bool,
    heights: &HeightField,
    width: f32,
) -> io::Result<BorderMeshes> {
    build_with_limit(map, classify, water, heights, width, MAX_CHUNK_VERTICES)
}

fn build_with_limit(
    map: &ProvinceMap,
    classify: &dyn Fn(u32, u32) -> Option<BorderClass>,
    water: &dyn Fn(u32) -> bool,
    heights: &HeightField,
    width: f32,
    limit: usize,
) -> io::Result<BorderMeshes> {
    heights.validate()?;
    if heights.width != map.width || heights.height != map.height {
        return Err(invalid(format!(
            "height field is {} x {} but the province map is {} x {}",
            heights.width, heights.height, map.width, map.height
        )));
    }
    if limit < 4 {
        return Err(invalid("a border chunk must hold at least two points"));
    }
    let traced = trace(map, classify, water)?;
    let mut meshes: Vec<BorderMesh> = BorderClass::ALL.iter().map(|&class| BorderMesh::new(class)).collect();
    let mut longest_run = 0_usize;
    for polyline in &traced.polylines {
        // A closed polyline already repeats its first point as its last; `trace` closes it before
        // smoothing, the way the engine's seam splice does.
        let strip = smooth(&polyline.points, polyline.wide);
        if strip.len() < 2 {
            continue;
        }
        emit(
            polyline,
            &strip,
            heights,
            width,
            limit,
            &mut meshes[polyline.class.bucket()],
            &mut longest_run,
        );
    }
    let mut hidden = BorderMesh::new(BorderClass::Province);
    for polyline in &traced.hidden {
        let strip = smooth(&polyline.points, polyline.wide);
        if strip.len() < 2 {
            continue;
        }
        emit(
            polyline,
            &strip,
            heights,
            width,
            limit,
            &mut hidden,
            &mut longest_run,
        );
    }
    let mut indices = Vec::with_capacity(longest_run.saturating_sub(1) * 6);
    for segment in 0..longest_run.saturating_sub(1) {
        let base = (segment * 2) as u16;
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 2, base + 3, base + 1]);
    }
    Ok(BorderMeshes {
        meshes,
        hidden,
        indices,
        one_pixel_provinces: traced.one_pixel_provinces,
    })
}

/// Widens one smoothed polyline into rail vertices and appends the draws it needs, at both levels.
///
/// A polyline longer than a chunk is split into runs that share their last and first point, so the
/// strip stays continuous across the seam; `uv.y` keeps counting from the polyline's own start so
/// the diffuse keeps tiling across the split.
///
/// The coarse level keeps the points [`coarse_points`] picks and is widened from its own
/// neighbours, so its strip is as wide as the fine one; each kept point keeps its own `uv.y`. A
/// run's coarse vertices go to the coarse chunk of the run's chunk.
fn emit(
    polyline: &Polyline,
    strip: &[[f32; 2]],
    heights: &HeightField,
    width: f32,
    limit: usize,
    mesh: &mut BorderMesh,
    longest_run: &mut usize,
) {
    let closed = polyline.closed;
    let total = strip.len();
    let run_points = limit / 2;
    let mut runs = Vec::new();
    let mut start = 0_usize;
    while start + 1 < total {
        let take = (total - start).min(run_points);
        runs.push((start, take));
        if start + take == total {
            break;
        }
        start += take - 1;
    }
    let kept = coarse_points(strip, closed, heights, &runs);
    let coarse: Vec<[f32; 2]> = kept.iter().map(|&index| strip[index]).collect();
    let mut first_kept = 0_usize;
    for (start, take) in runs {
        let chunk = mesh.chunks.last().map(Vec::len).unwrap_or(0);
        if (chunk + take * 2 > limit && chunk > 0) || mesh.chunks.is_empty() {
            mesh.chunks.push(Vec::new());
        }
        mesh.coarse_chunks.resize_with(mesh.chunks.len(), Vec::new);
        let at = mesh.chunks.len() - 1;
        let base_vertex = mesh.chunks[at].len() as u32;
        for index in start..start + take {
            rails(strip, index, index, closed, heights, width, &mut mesh.chunks[at]);
        }
        let last_kept = first_kept
            + kept[first_kept..]
                .iter()
                .position(|&index| index == start + take - 1)
                .expect("run ends are kept");
        let coarse_base_vertex = mesh.coarse_chunks[at].len() as u32;
        for at_coarse in first_kept..=last_kept {
            rails(&coarse, at_coarse, kept[at_coarse], closed, heights, width, &mut mesh.coarse_chunks[at]);
        }
        mesh.draws.push(BorderDraw {
            province_a: polyline.province_a,
            province_b: polyline.province_b,
            chunk: at as u32,
            base_vertex,
            vertex_count: (take * 2) as u32,
            index_count: ((take - 1) * 6) as u32,
            coarse_base_vertex,
            coarse_vertex_count: ((last_kept - first_kept + 1) * 2) as u32,
        });
        *longest_run = (*longest_run).max(take);
        first_kept = last_kept;
    }
}

/// The two rail vertices of `points[index]`, with `uv.y` `counter`.
fn rails(
    points: &[[f32; 2]],
    index: usize,
    counter: usize,
    closed: bool,
    heights: &HeightField,
    width: f32,
    out: &mut Vec<Vertex>,
) {
    let point = points[index];
    let previous = neighbour(points, index as isize - 1, closed);
    let next = neighbour(points, index as isize + 1, closed);
    let direction = [next[0] - previous[0], next[1] - previous[1]];
    let length = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
    // A zero length direction can only come from a degenerate polyline; the quad then
    // collapses rather than producing a not-a-number position.
    let rail_offset = if length > 0.0 {
        [
            (width * 0.5 * direction[1]) / length,
            (width * 0.5 * -direction[0]) / length,
        ]
    } else {
        [0.0, 0.0]
    };
    let counter = counter as i16;
    for (side, sign) in [(0_i16, 1.0_f32), (1, -1.0)] {
        let x = point[0] + rail_offset[0] * sign;
        let z = point[1] + rail_offset[1] * sign;
        out.push(Vertex {
            position: [x, heights.border_height_at_world(x, z), z],
            uv: [side, counter],
        });
    }
}

/// The points of a smoothed strip the coarse level keeps ([`line_lod::simplify`]): the ends of its
/// runs, every point off the line by more than [`line_lod::TOLERANCE`] or above or below it by more
/// than [`line_lod::HEIGHT_TOLERANCE`], and on a closed loop two more a third of the way round
/// each, so it stays a loop.
fn coarse_points(
    strip: &[[f32; 2]],
    closed: bool,
    heights: &HeightField,
    runs: &[(usize, usize)],
) -> Vec<usize> {
    let height: Vec<f32> = strip
        .iter()
        .map(|point| heights.border_height_at_world(point[0], point[1]))
        .collect();
    let mut anchors: Vec<usize> = runs.iter().map(|&(start, take)| start + take - 1).collect();
    if closed {
        let last = strip.len().saturating_sub(1);
        anchors.extend([last / 3, last * 2 / 3]);
    }
    line_lod::simplify(strip.len(), &anchors, |a, b, index| {
        let (off, t) = line_lod::offset(strip[a], strip[b], strip[index]);
        let rise = height[index] - line_lod::lerp(height[a], height[b], t);
        (off / line_lod::TOLERANCE).max(rise.abs() / line_lod::HEIGHT_TOLERANCE)
    })
}

/// The neighbouring point used for the widening normal: wrapped on a closed strip whose last point
/// repeats its first, mirror extrapolated at an open end.
fn neighbour(strip: &[[f32; 2]], index: isize, closed: bool) -> [f32; 2] {
    let count = strip.len() as isize;
    if closed && count >= 3 {
        // The strip repeats its first point at the end, so stepping off either end skips the copy.
        if index < 0 {
            return strip[(count - 2) as usize];
        }
        if index >= count {
            return strip[1];
        }
        return strip[index as usize];
    }
    extend(strip, index, false)
}

/// Provinces covering exactly one pixel, sorted, for the native diagnostic (not an exclusion).
fn one_pixel_provinces(map: &ProvinceMap) -> Vec<u32> {
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for &id in map.ids {
        *counts.entry(id).or_insert(0) += 1;
    }
    let mut degenerate: Vec<u32> = counts
        .into_iter()
        .filter(|&(_, count)| count == 1)
        .map(|(id, _)| id)
        .collect();
    degenerate.sort_unstable();
    degenerate
}

/// The pixel-edge cracks of a province map, addressed by index.
///
/// Vertical cracks come first, `width * height` of them: crack `y * width + column` separates pixel
/// `(column, y)` from pixel `(column + 1, y)`, wrapping so the crack at `column = width - 1` sits on
/// the map's x seam. Horizontal cracks follow, `(height - 1) * width` of them: crack `row * width +
/// x` separates pixel `(x, row)` from pixel `(x, row + 1)`. Pixel corners run `0..=width` by
/// `0..=height`; the seam's corners at `x = width` are deliberately not the same nodes as those at
/// `x = 0`, which is what cuts a chain at the seam instead of letting it jump the map.
/// The engine labels four-connected province components in bottom-up raster order before tracing.
/// Labels do not wrap at the map seam; corner lookup does.
struct BoundaryComponents {
    components: Vec<u32>,
    width: usize,
    height: usize,
}

impl BoundaryComponents {
    fn new(map: &ProvinceMap) -> Self {
        let width = map.width as usize;
        let height = map.height as usize;
        let mut components = vec![0; width * height];
        let mut stack = Vec::new();
        let mut label = 0;
        for y in (0..height).rev() {
            for x in 0..width {
                let start = y * width + x;
                if components[start] != 0 {
                    continue;
                }
                label += 1;
                components[start] = label;
                stack.push(start);
                while let Some(index) = stack.pop() {
                    let x = index % width;
                    let y = index / width;
                    for next in [
                        (x > 0).then(|| index - 1),
                        (x + 1 < width).then(|| index + 1),
                        (y > 0).then(|| index - width),
                        (y + 1 < height).then(|| index + width),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        if components[next] == 0 && map.ids[next] == map.ids[start] {
                            components[next] = label;
                            stack.push(next);
                        }
                    }
                }
            }
        }
        Self {
            components,
            width,
            height,
        }
    }

    /// Three or four distinct component labels insert a shared lattice corner.
    fn is_junction(&self, corner: [u32; 2]) -> bool {
        let [x, y] = [corner[0] as usize, corner[1] as usize];
        let left = (x + self.width - 1) % self.width;
        let right = x % self.width;
        let top = y.saturating_sub(1).min(self.height - 1);
        let bottom = y.min(self.height - 1);
        let at = |row: usize, column: usize| self.components[row * self.width + column];
        let block = [
            at(top, left),
            at(top, right),
            at(bottom, left),
            at(bottom, right),
        ];
        let mut distinct = 0_usize;
        for index in 0..4 {
            if !block[..index].contains(&block[index]) {
                distinct += 1;
            }
        }
        distinct > 2
    }
}

#[cfg(test)]
mod tests;
