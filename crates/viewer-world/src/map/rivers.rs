//! River polylines and ribbons from the resolved river bitmap.
//!
//! [`trace`] and [`build`] produce CPU geometry; [`RiverPass`] uploads it and
//! uses the recorded content's compiled shader. Tracing inspects neighbours in
//! left/right/up/down order, clamping at bitmap edges. Body bytes 3 through 99
//! carry per-point widths. Traces shorter than three points are diagnosed and
//! dropped; meeting an already-traced river diagnoses a circular path and
//! stops the walk. Both bulk background indices are excluded.
use std::{cell::RefCell, collections::BTreeMap, io, sync::Arc};
use viewer_foundation::map::textures::load_uploadable;

use eframe::wgpu;
use installed_content_core::ResolvedLayers;
use installed_content_image::{dds, native::Decode};

use viewer_foundation::map::{
    gpu_contract::{DEPTH_FORMAT, buffer},
    province_map::{HeightField, world_from_pixel},
    textures::{Bound, ColorSpace, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Palette index of a river source, the only index that seeds a trace.
pub const SOURCE_INDEX: u8 = 0;
/// Highest palette index the engine treats as a junction marker (`b < 3`).
pub const JUNCTION_MAX_INDEX: u8 = 2;
/// Lowest palette index that is a river body pixel.
pub const BODY_MIN_INDEX: u8 = 3;
/// Highest palette index that is a river body pixel.
///
/// River-body palette indices range from 3 through 99 inclusive.
pub const BODY_MAX_INDEX: u8 = 99;
/// Points a trace must reach to be kept; below it the engine logs "Too small river found at " and
/// drops the river.
pub const MIN_RIVER_POINTS: usize = 3;

/// `RIVER_FADE_FROM` (`00_graphics.lua`): distance from an ending at which the end fade starts.
pub const RIVER_FADE_FROM: f32 = 20.0;
/// RIVER_FADE_TO: distance from an ending at which it is fully faded.
pub const RIVER_FADE_TO: f32 = 3.0;

/// Ribbon width in world units at palette index 0, the intercept of [`width_for_index`].
///
/// River vertices pair the left and right rails, with two float4 attributes and a 32-byte stride.
/// Palette index controls ribbon width.
///
/// Compute half-width in double precision from palette and float base, narrow to f32, then cap by
/// endpoint index.
pub const WIDTH_BASE: f32 = 0.60;
/// World units of ribbon width per palette index; see [`WIDTH_BASE`].
pub const WIDTH_PER_INDEX: f64 = 0.17;

/// Full body width, preserving the native double multiply and float intercept.
pub fn width_for_index(index: u8) -> f32 {
    (f64::from(index) * (WIDTH_PER_INDEX / 2.0) + f64::from(WIDTH_BASE / 2.0)) as f32 * 2.0
}

/// Whether a palette index is a river body pixel the walk may step onto.
pub fn is_body(index: u8) -> bool {
    (BODY_MIN_INDEX..=BODY_MAX_INDEX).contains(&index)
}

/// `map/rivers.bmp` as the loaders return it: one palette index per pixel, rows top-left first.
#[derive(Clone, Copy, Debug)]
pub struct RiverImage<'a> {
    pub width: u32,
    pub height: u32,
    pub indices: &'a [u8],
}

impl RiverImage<'_> {
    /// Checks the index count matches the declared dimensions.
    pub fn validate(&self) -> io::Result<()> {
        if self.width == 0 || self.height == 0 {
            return Err(invalid("the river bitmap needs a nonzero width and height"));
        }
        let count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| invalid("the river bitmap's pixel count overflows"))?;
        if self.indices.len() != count {
            return Err(invalid(format!(
                "the river bitmap holds {} indices for {} x {} pixels",
                self.indices.len(),
                self.width,
                self.height
            )));
        }
        Ok(())
    }

    fn at(&self, x: u32, y: u32) -> u8 {
        self.indices
            .get(y as usize * self.width as usize + x as usize)
            .copied()
            .unwrap_or(u8::MAX)
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        y as usize * self.width as usize + x as usize
    }
}

/// What seeded a walk: a source pixel, or a junction marker with its palette index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Palette index 0.
    Source,
    /// Palette index 1 or 2, carried as the walk's initial width parameter.
    Junction(u8),
}

/// One traced river: the pixel walk, with the palette index under each point.
///
/// `points` are world `(x, z)` on pixel centres and `widths[i]` is the palette index the engine
/// carried into point `i`. The two are always the same length. A river never closes: the walk stops
/// when it meets a pixel it has already traced.
#[derive(Clone, Debug, PartialEq)]
pub struct River {
    pub points: Vec<[f32; 2]>,
    pub widths: Vec<u8>,
    pub seed: Seed,
    /// The seeding pixel, in bitmap coordinates.
    pub seed_pixel: [u32; 2],
}

/// A river the engine would have reported, in the bitmap coordinates its message prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reported {
    pub x: u32,
    pub y: u32,
    /// Points the walk produced.
    pub points: u32,
}

/// Everything [`trace`] found, including what the engine logs and drops.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    /// Rivers of at least [`MIN_RIVER_POINTS`] points, in the order the engine traces them.
    pub rivers: Vec<River>,
    /// Walks the engine logs as "Too small river found at " and drops.
    pub too_small: Vec<Reported>,
    /// Walks that met an already-traced river; the engine logs "Circular river found at " and stops
    /// the walk there, keeping whatever it had.
    pub circular: Vec<Reported>,
    /// Junction markers seen, and how many of them seeded a walk.
    pub junctions: u32,
    pub junctions_used: u32,
    /// Source pixels seen.
    pub sources: u32,
    /// Body pixels no walk ever reached. The engine does not report these; they are counted here
    /// because a river the viewer never draws is a visible difference from the game.
    pub unreachable: u32,
}

/// The four neighbours the engine examines, in its own order, each clamped into the bitmap.
///
/// Clamp neighbour x/y to the bitmap. River paths stop at the horizontal seam instead of wrapping.
fn neighbours(x: u32, y: u32, width: u32, height: u32) -> [(u32, u32); 4] {
    let clamp = |value: i64, limit: u32| value.clamp(0, i64::from(limit) - 1) as u32;
    let (ix, iy) = (i64::from(x), i64::from(y));
    [
        (clamp(ix - 1, width), clamp(iy, height)),
        (clamp(ix + 1, width), clamp(iy, height)),
        (clamp(ix, width), clamp(iy + 1, height)),
        (clamp(ix, width), clamp(iy - 1, height)),
    ]
}

/// One ordered river walk.
///
/// Returns the pixel a circular river was detected at, if any.
///
/// The walk is greedy and never branches: at each step it takes the first of the four neighbours
/// that is a body pixel, is not the pixel it came from, and has not been traced yet. Where a river
/// forks, the neighbour order decides which arm is followed and the other arm is left for the
/// junction pass. A neighbour that has been traced is skipped while the carried width parameter is
/// still a seed value (0, 1 or 2) — that is how a junction walk steps past the stream it was
/// stitched onto — and reported as circular once the walk is on the river proper.
///
/// The native bitmap is bottom-up: its -y neighbour is the next row of this top-down image.
/// Rejected traced neighbours update the remembered previous pixel too.
fn walk(
    image: &RiverImage<'_>,
    visited: &mut [bool],
    seed_pixel: (u32, u32),
    seed_index: u8,
    river: &mut River,
) -> Option<(u32, u32)> {
    let (x, y) = seed_pixel;
    // Width parameter carried onto the next appended point.
    let mut carried = seed_index;
    if seed_index != SOURCE_INDEX {
        // A junction walk starts on the stream it joins, so the ribbon reaches the water it flows
        // into instead of stopping a pixel short of it.
        for (nx, ny) in neighbours(x, y, image.width, image.height) {
            if (nx, ny) == (x, y) {
                continue;
            }
            let index = image.at(nx, ny);
            if !is_body(index) || !visited[image.offset(nx, ny)] {
                continue;
            }
            carried = index;
            river.points.push(point(image, nx, ny));
            river.widths.push(index);
            break;
        }
    }
    river.points.push(point(image, x, y));
    river.widths.push(carried);
    // The seed value gates circular-path reporting until the first body pixel is taken; the carried
    // width does not.
    let mut guard = seed_index;
    let (mut previous, mut current) = ((x, y), (x, y));
    loop {
        let mut advanced = false;
        let mut remembered = current;
        for (nx, ny) in neighbours(current.0, current.1, image.width, image.height) {
            if (nx, ny) == previous {
                continue;
            }
            let index = image.at(nx, ny);
            if !is_body(index) {
                continue;
            }
            let offset = image.offset(nx, ny);
            previous = remembered;
            if !visited[offset] {
                visited[offset] = true;
                river.points.push(point(image, nx, ny));
                river.widths.push(index);
                current = (nx, ny);
                guard = index;
                advanced = true;
                break;
            }
            remembered = (nx, ny);
            if guard > JUNCTION_MAX_INDEX {
                return Some(current);
            }
        }
        if !advanced {
            return None;
        }
    }
}

fn point(image: &RiverImage<'_>, x: u32, y: u32) -> [f32; 2] {
    let (wx, wz) = world_from_pixel(image.height, x as f32, y as f32);
    [wx, wz]
}

/// Trace every river in the resolved river bitmap.
///
/// Two passes, as the engine has. The first walks from every source pixel in bitmap order and
/// collects the junction markers; the second repeatedly walks from those junction markers that now
/// touch a traced pixel, until none is left that does. The repetition is what lets a tributary of a
/// tributary reach the sea: a junction can only be walked once the stream it joins has been.
///
/// Errors are named `InvalidData` for a bitmap whose index count does not match its dimensions.
pub fn trace(image: &RiverImage<'_>) -> io::Result<Trace> {
    image.validate()?;
    let mut visited = vec![false; image.indices.len()];
    let mut trace = Trace::default();
    let mut junctions: Vec<(u32, u32, u8)> = Vec::new();
    for y in (0..image.height).rev() {
        for x in 0..image.width {
            let index = image.at(x, y);
            if index == SOURCE_INDEX {
                trace.sources += 1;
                keep(
                    &mut trace,
                    walk_one(image, &mut visited, (x, y), SOURCE_INDEX, Seed::Source),
                );
            } else if index <= JUNCTION_MAX_INDEX {
                trace.junctions += 1;
                junctions.push((x, y, index));
            }
        }
    }
    // Sort junction palette values in ascending order.
    sort_junctions(&mut junctions);
    let mut done = vec![false; junctions.len()];
    loop {
        let mut ready = Vec::new();
        for (slot, &(x, y, _)) in junctions.iter().enumerate() {
            if done[slot] {
                continue;
            }
            if neighbours(x, y, image.width, image.height)
                .into_iter()
                .any(|(nx, ny)| is_body(image.at(nx, ny)) && visited[image.offset(nx, ny)])
            {
                ready.push(slot);
            }
        }
        if ready.is_empty() {
            break;
        }
        for slot in ready {
            done[slot] = true;
            let (x, y, index) = junctions[slot];
            trace.junctions_used += 1;
            keep(
                &mut trace,
                walk_one(image, &mut visited, (x, y), index, Seed::Junction(index)),
            );
        }
    }
    trace.unreachable = image
        .indices
        .iter()
        .zip(&visited)
        .filter(|&(&index, &seen)| is_body(index) && !seen)
        .count() as u32;
    Ok(trace)
}

/// Median partition and insertion sort preserve equal-key junction order for overlapping ribbons.
/// Only keys one and two enter this sort; the general heapsort fallback is unreachable.
fn sort_junctions(items: &mut [(u32, u32, u8)]) {
    let count = items.len();
    if count <= 32 {
        for i in 1..count {
            let mut j = i;
            while j > 0 && items[j].2 < items[j - 1].2 {
                items.swap(j, j - 1);
                j -= 1;
            }
        }
        return;
    }
    let mid = count / 2;
    let last = count - 1;
    if last > 40 {
        let step = count / 8;
        median_junction(items, 0, step, step * 2);
        median_junction(items, mid - step, mid, mid + step);
        median_junction(items, last - step * 2, last - step, last);
        median_junction(items, step, mid, last - step);
    } else {
        median_junction(items, 0, mid, last);
    }
    let mut first_equal = mid;
    let mut after_equal = mid + 1;
    while first_equal > 0 && items[first_equal - 1].2 == items[first_equal].2 {
        first_equal -= 1;
    }
    while after_equal < count && items[after_equal].2 == items[first_equal].2 {
        after_equal += 1;
    }
    let mut right = after_equal;
    let mut left = first_equal;
    loop {
        while right < count {
            match items[right].2.cmp(&items[first_equal].2) {
                std::cmp::Ordering::Less => break,
                std::cmp::Ordering::Equal => {
                    items.swap(after_equal, right);
                    after_equal += 1;
                }
                std::cmp::Ordering::Greater => {}
            }
            right += 1;
        }
        while left > 0 {
            match items[left - 1].2.cmp(&items[first_equal].2) {
                std::cmp::Ordering::Greater => break,
                std::cmp::Ordering::Equal => {
                    first_equal -= 1;
                    items.swap(first_equal, left - 1);
                }
                std::cmp::Ordering::Less => {}
            }
            left -= 1;
        }
        if left == 0 && right == count {
            break;
        }
        if left == 0 {
            if after_equal != right {
                items.swap(first_equal, after_equal);
            }
            after_equal += 1;
            items.swap(first_equal, right);
            first_equal += 1;
            right += 1;
        } else if right == count {
            left -= 1;
            first_equal -= 1;
            items.swap(left, first_equal);
            after_equal -= 1;
            items.swap(first_equal, after_equal);
        } else {
            left -= 1;
            items.swap(right, left);
            right += 1;
        }
    }
    sort_junctions(&mut items[..first_equal]);
    sort_junctions(&mut items[after_equal..]);
}

fn median_junction(items: &mut [(u32, u32, u8)], first: usize, mid: usize, last: usize) {
    if items[mid].2 < items[first].2 {
        items.swap(first, mid);
    }
    if items[last].2 < items[mid].2 {
        items.swap(mid, last);
        if items[mid].2 < items[first].2 {
            items.swap(first, mid);
        }
    }
}

/// One walk plus the engine's two reports, before the length test.
struct Walked {
    river: River,
    circular: Option<(u32, u32)>,
}

fn walk_one(
    image: &RiverImage<'_>,
    visited: &mut [bool],
    seed_pixel: (u32, u32),
    seed_index: u8,
    seed: Seed,
) -> Walked {
    let mut river = River {
        points: Vec::new(),
        widths: Vec::new(),
        seed,
        seed_pixel: [seed_pixel.0, seed_pixel.1],
    };
    let circular = walk(image, visited, seed_pixel, seed_index, &mut river);
    Walked { river, circular }
}

fn keep(trace: &mut Trace, walked: Walked) {
    let Walked { river, circular } = walked;
    let points = river.points.len() as u32;
    if let Some((x, y)) = circular {
        trace.circular.push(Reported { x, y, points });
    }
    if river.points.len() < MIN_RIVER_POINTS {
        trace.too_small.push(Reported {
            x: river.seed_pixel[0],
            y: river.seed_pixel[1],
            points,
        });
        return;
    }
    trace.rivers.push(river);
}

/// One ribbon vertex, in `river.shader`'s `VS_INPUT` layout.
///
/// `position.xyz` is world position; `position.w` carries the one-ended transparency fade.
/// `uv_tangent.zw` carries the 2D flow tangent. The texture coordinates use this order:
///
/// * `uv_tangent.x` runs across the ribbon from zero on one rail to one on the other. The shader
///   maps it to its second texture axis, mirrors the lower half around the centre and folds both
///   halves onto the bank profile.
/// * `uv_tangent.y` is the integer trace-point index. Smoothing changes geometric spacing while
///   preserving point count; this coordinate keeps the along-flow texture increment at one per
///   point before the shader applies its scale.
///
/// The point index also selects the first point for the endpoint profile. Transparency remains
/// independent in `position.w`; see [`ribbon`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RiverVertex {
    pub position: [f32; 4],
    pub uv_tangent: [f32; 4],
}

/// The finished ribbon geometry: one vertex buffer and one index buffer over every river.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiverMesh {
    pub vertices: Vec<RiverVertex>,
    pub indices: Vec<u32>,
    /// Ribbons the mesh holds, one per traced river.
    pub ribbons: usize,
    pub runs: Vec<super::river_shader::Run>,
}

impl RiverMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Traces `map/rivers.bmp` and widens every river into a ribbon.
///
/// Heights use the native bilinear sampler, without water-plane clamping (B = 0).
/// The strip's +0.5 offset cancels the sampler's -0.5 texel-centre adjustment.
///
/// Errors are named `InvalidData` for a bitmap or height field whose sample count does not match
/// its dimensions, or for a height field of a different size than the river bitmap.
pub fn build(image: &RiverImage<'_>, heights: &HeightField<'_>) -> io::Result<(RiverMesh, Trace)> {
    heights.validate()?;
    if heights.width != image.width || heights.height != image.height {
        return Err(invalid(format!(
            "the height field is {} x {} but the river bitmap is {} x {}",
            heights.width, heights.height, image.width, image.height
        )));
    }
    let trace = trace(image)?;
    let mut mesh = RiverMesh::default();
    for river in &trace.rivers {
        ribbon(river, heights, &mut mesh);
    }
    Ok((mesh, trace))
}

/// Apply one forward in-place five-point mean, leaving two points at each end.
fn smooth_river(points: &mut [[f32; 2]]) {
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
}

/// Clamp the integer cell, retain fractional offsets, bilinearly interpolate height bytes and scale
/// by .1f.
fn river_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
    // Preserve both operations: their float rounding survives near the map edges.
    let x = (x + 0.5) - 0.5;
    let z = (z + 0.5) - 0.5;
    let ix = (x as i32).clamp(0, heights.width.saturating_sub(2) as i32);
    let iz = (z as i32).clamp(0, heights.height.saturating_sub(2) as i32);
    let sample = |dx: i32, dz: i32| {
        let column = (ix + dx).min(heights.width as i32 - 1) as usize;
        let row =
            (heights.height as i32 - 1 - iz - dz).clamp(0, heights.height as i32 - 1) as usize;
        f32::from(heights.values[row * heights.width as usize + column])
    };
    let fx = x - ix as f32;
    let fz = z - iz as f32;
    let a = sample(0, 0);
    let b = sample(0, 1);
    let low = (sample(1, 0) - a) * fx + a;
    let high = (sample(1, 1) - b) * fx + b;
    ((high - low) * fz + low) * 0.1
}

/// Sources taper/fade at the first point; junction
/// walks taper/fade at their far end. Fade distances accumulate the lengths of the *unnormalized
/// centred tangents*, including the current point, in double precision. The last pair reuses
/// the preceding offset/tangent, with alpha 1 for sources and 0 for junctions.
fn ribbon(river: &River, heights: &HeightField<'_>, mesh: &mut RiverMesh) {
    let mut points = river.points.clone();
    smooth_river(&mut points);
    let count = points.len();
    if count < 2 {
        return;
    }
    let junction = matches!(river.seed, Seed::Junction(_));
    let base = mesh.vertices.len();
    let mut lengths = Vec::with_capacity(count - 1);
    let mut total = 0.0_f64;
    let mut tangent = [0.0; 2];
    let mut offset = [0.0; 2];
    for index in 0..count {
        if index + 1 < count {
            let previous = points[index.saturating_sub(1)];
            let next = points[index + 1];
            let direction = [next[0] - previous[0], next[1] - previous[1]];
            let length = (direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
            tangent = if length > 0.0 {
                [direction[0] / length, direction[1] / length]
            } else {
                [0.0; 2]
            };
            let endpoint_index = if junction { count - 2 - index } else { index };
            let half =
                (width_for_index(river.widths[index]) * 0.5).min(endpoint_index as f32 * 0.05);
            offset = [tangent[1] * half, -tangent[0] * half];
            lengths.push(length);
            total += f64::from(length);
        }
        for (side, sign) in [(0.0_f32, 1.0_f32), (1.0, -1.0)] {
            let x = points[index][0] + offset[0] * sign;
            let z = points[index][1] + offset[1] * sign;
            mesh.vertices.push(RiverVertex {
                position: [x, river_height(heights, x, z), z, f32::from(!junction)],
                uv_tangent: [side, index as f32, tangent[0], tangent[1]],
            });
        }
    }
    let unfaded = (total - f64::from(RIVER_FADE_FROM)).max(0.0);
    let mut along = 0.0_f64;
    for (index, length) in lengths.into_iter().enumerate() {
        along += f64::from(length);
        let distance = if junction { along } else { total - along };
        let alpha =
            (1.0 - (distance - unfaded) / f64::from(RIVER_FADE_FROM - RIVER_FADE_TO)) as f32;
        mesh.vertices[base + index * 2].position[3] = alpha.clamp(0.0, 1.0);
        mesh.vertices[base + index * 2 + 1].position[3] = alpha.clamp(0.0, 1.0);
    }
    let first = mesh.indices.len() as u32;
    for segment in 0..count as u32 - 1 {
        let quad = base as u32 + segment * 2;
        mesh.indices
            .extend_from_slice(&[quad, quad + 2, quad + 1, quad + 2, quad + 3, quad + 1]);
    }
    // Pack group as twice the wave index plus palette index, then select its parity.
    let direction = match river.seed {
        Seed::Junction(value) if value & 1 != 0 => 1.0,
        _ => -1.0,
    };
    let end = mesh.indices.len() as u32;
    if let Some(last) = mesh
        .runs
        .last_mut()
        .filter(|last| last.direction == direction)
    {
        last.indices.end = end;
    } else {
        mesh.runs.push(super::river_shader::Run {
            indices: first..end,
            direction,
        });
    }
    mesh.ribbons += 1;
}

// ------------------------------------------------------------------ the GPU pass

/// The three `map/terrain/RiverSurface_*` textures, decoded but not uploaded.
pub struct RiverAssets {
    pub shader: Arc<shader_pipeline::library::Effect>,
    /// Quality-selected `RiverSurface_diffuse_*.dds`, the bank strip: `u` along the flow, `v` bank to centre.
    pub diffuse: dds::Texture,
    /// Quality-selected `RiverSurface_normal_*.dds`, the bank strip's normal map.
    pub normal: dds::Texture,
    /// `RiverSurface_masks.dds`, the `RiverData` sampler: blue is the bank blend and red the
    /// opacity. It carries no quality suffix.
    pub masks: dds::Texture,
}

impl RiverAssets {
    /// Reads the three textures through the presentation layers, so a selected visual mod's
    /// river surface replaces them. Recorded map content is hash-verified.
    pub fn load(
        layers: &ResolvedLayers,
        map: [u32; 2],
        texture_quality: u8,
        decode: Decode,
    ) -> io::Result<Self> {
        let texture = |name: &str, space: ColorSpace| -> io::Result<dds::Texture> {
            let path = format!("map/terrain/{name}");
            layers.artwork(&path, |stack| {
                load_uploadable(&path, &|request| stack.read_path(request), decode, space)
            })
        };
        Ok(Self {
            shader: super::river_shader::load(layers, map, decode)?,
            diffuse: texture(
                &format!("RiverSurface_diffuse_{texture_quality}.dds"),
                ColorSpace::Srgb,
            )?,
            normal: texture(
                &format!("RiverSurface_normal_{texture_quality}.dds"),
                ColorSpace::Linear,
            )?,
            masks: texture("RiverSurface_masks.dds", ColorSpace::Linear)?,
        })
    }
}

/// The GPU river pass.
pub struct RiverPass {
    effect: Arc<shader_pipeline::library::Effect>,
    compiled: RefCell<BTreeMap<bool, Result<super::river_shader::River, String>>>,
    samples: u32,
    format: wgpu::TextureFormat,
    runs: Vec<super::river_shader::Run>,
    diffuse: Bound,
    normal: Bound,
    masks: Bound,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
}

impl RiverPass {
    /// Builds the pipeline and uploads the three river surface textures.
    pub fn new(
        device: &wgpu::Device,
        samples: u32,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        assets: &RiverAssets,
    ) -> io::Result<Self> {
        Ok(Self {
            effect: assets.shader.clone(),
            compiled: RefCell::new(BTreeMap::new()),
            samples,
            format,
            runs: Vec::new(),
            diffuse: upload(
                device,
                queue,
                "RiverSurface diffuse",
                &assets.diffuse,
                ColorSpace::Srgb,
            )?,
            normal: upload(
                device,
                queue,
                "RiverSurface normal",
                &assets.normal,
                ColorSpace::Linear,
            )?,
            masks: upload(
                device,
                queue,
                "RiverSurface_masks",
                &assets.masks,
                ColorSpace::Linear,
            )?,
            vertices: None,
            indices: None,
        })
    }

    /// Uploads the ribbon geometry. Called once per document: the mesh depends only on content.
    pub fn upload(&mut self, device: &wgpu::Device, mesh: &RiverMesh) -> io::Result<()> {
        self.runs.clone_from(&mesh.runs);
        if mesh.is_empty() {
            self.vertices = None;
            self.indices = None;
            return Ok(());
        }
        self.vertices = Some(buffer(
            device,
            "river ribbons",
            wgpu::BufferUsages::VERTEX,
            bytemuck::cast_slice(&mesh.vertices),
        )?);
        self.indices = Some(buffer(
            device,
            "river indices",
            wgpu::BufferUsages::INDEX,
            bytemuck::cast_slice(&mesh.indices),
        )?);
        Ok(())
    }

    pub fn textures(&self) -> [&wgpu::TextureView; 3] {
        [&self.diffuse.view, &self.normal.view, &self.masks.view]
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        target: super::border_shader::Target<'_>,
        inputs: super::river_shader::Inputs<'_>,
        high: bool,
    ) -> io::Result<()> {
        let device = arena.device();
        let (Some(vertices), Some(indices)) = (&self.vertices, &self.indices) else {
            return Ok(());
        };
        let mut compiled = self.compiled.borrow_mut();
        if let std::collections::btree_map::Entry::Vacant(entry) = compiled.entry(high) {
            entry.insert(
                super::river_shader::River::new(
                    device,
                    &self.effect,
                    self.samples,
                    self.format,
                    DEPTH_FORMAT,
                    high,
                )
                .map_err(|error| error.to_string()),
            );
        }
        viewer_foundation::map::gpu_contract::compiled(&compiled[&high])?.draw(
            arena,
            encoder,
            target,
            inputs,
            super::river_shader::Geometry {
                vertices,
                indices,
                runs: &self.runs,
            },
        )
    }
}

#[cfg(test)]
mod tests;
