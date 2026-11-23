use viewer_gpu::gpu_contract::Targets;
use std::{cell::RefCell, collections::BTreeMap, io, sync::Arc};
use viewer_gpu::textures::load_uploadable;

use eframe::wgpu;
use content_reader_layers::ResolvedLayers;
use content_reader_defines::defines::Defines;
use content_reader_image::{dds, native::Decode};

use viewer_gpu::{
    gpu_contract::{DEPTH_FORMAT, buffer},
    province_map::{HeightField, world_from_pixel},
    textures::{Bound, ColorSpace, upload},
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub const SOURCE_INDEX: u8 = 0;
pub const JUNCTION_MAX_INDEX: u8 = 2;
pub const BODY_MIN_INDEX: u8 = 3;
pub const BODY_MAX_INDEX: u8 = 99;
pub const MIN_RIVER_POINTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverDefines {
    pub fade_from: f32,
    pub fade_to: f32,
}

impl RiverDefines {
    pub fn load(defines: &Defines) -> io::Result<Self> {
        let read = |name: &str| -> io::Result<f32> {
            let value = defines.fixed5("NGraphics", name)?;
            Ok(value as f32 / 100_000.0)
        };
        let fade_from = read("RIVER_FADE_FROM")?;
        let fade_to = read("RIVER_FADE_TO")?;
        if !(fade_from.is_finite() && fade_to.is_finite()) || fade_from <= fade_to {
            return Err(invalid(format!(
                "NDefines.NGraphics.RIVER_FADE_FROM ({fade_from}) must exceed \
                 NDefines.NGraphics.RIVER_FADE_TO ({fade_to})"
            )));
        }
        Ok(Self { fade_from, fade_to })
    }
}

pub const WIDTH_BASE: f32 = 0.60;
pub const WIDTH_PER_INDEX: f64 = 0.17;

pub fn width_for_index(index: u8) -> f32 {
    (f64::from(index) * (WIDTH_PER_INDEX / 2.0) + f64::from(WIDTH_BASE / 2.0)) as f32 * 2.0
}

pub fn is_body(index: u8) -> bool {
    (BODY_MIN_INDEX..=BODY_MAX_INDEX).contains(&index)
}

#[derive(Clone, Copy, Debug)]
pub struct RiverImage<'a> {
    pub width: u32,
    pub height: u32,
    pub indices: &'a [u8],
}

impl RiverImage<'_> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    Source,
    Junction(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct River {
    pub points: Vec<[f32; 2]>,
    pub widths: Vec<u8>,
    pub seed: Seed,
    pub seed_pixel: [u32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reported {
    pub x: u32,
    pub y: u32,
    pub points: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    pub rivers: Vec<River>,
    pub too_small: Vec<Reported>,
    pub circular: Vec<Reported>,
    pub junctions: u32,
    pub junctions_used: u32,
    pub sources: u32,
    pub unreachable: u32,
}

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

fn walk(
    image: &RiverImage<'_>,
    visited: &mut [bool],
    seed_pixel: (u32, u32),
    seed_index: u8,
    river: &mut River,
) -> Option<(u32, u32)> {
    let (x, y) = seed_pixel;
    let mut carried = seed_index;
    if seed_index != SOURCE_INDEX {
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

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RiverVertex {
    pub position: [f32; 4],
    pub uv_tangent: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiverMesh {
    pub vertices: Vec<RiverVertex>,
    pub indices: Vec<u32>,
    pub ribbons: usize,
    pub runs: Vec<super::river_shader::Run>,
    pub coarse_indices: Vec<u32>,
    pub coarse_runs: Vec<super::river_shader::Run>,
}

const COARSE_ALPHA_TOLERANCE: f32 = 1.0 / 32.0;
const COARSE_KEEPS_AT_MOST: f32 = 0.8;

impl RiverMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

pub fn build(
    image: &RiverImage<'_>,
    heights: &HeightField<'_>,
    defines: RiverDefines,
) -> io::Result<(RiverMesh, Trace)> {
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
        ribbon(river, heights, defines, &mut mesh);
    }
    Ok((mesh, trace))
}

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

fn river_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
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

fn quads(
    indices: &mut Vec<u32>,
    runs: &mut Vec<super::river_shader::Run>,
    base: usize,
    count: usize,
    direction: f32,
) {
    let first = indices.len() as u32;
    for segment in 0..count as u32 - 1 {
        let quad = base as u32 + segment * 2;
        indices.extend_from_slice(&[quad, quad + 2, quad + 1, quad + 2, quad + 3, quad + 1]);
    }
    let end = indices.len() as u32;
    if let Some(last) = runs.last_mut().filter(|last| last.direction == direction) {
        last.indices.end = end;
    } else {
        runs.push(super::river_shader::Run {
            indices: first..end,
            direction,
        });
    }
}

fn ribbon(river: &River, heights: &HeightField<'_>, defines: RiverDefines, mesh: &mut RiverMesh) {
    let mut points = river.points.clone();
    smooth_river(&mut points);
    let count = points.len();
    if count < 2 {
        return;
    }
    let junction = matches!(river.seed, Seed::Junction(_));
    let mut lengths = Vec::with_capacity(count - 1);
    let mut halves = Vec::with_capacity(count);
    let mut total = 0.0_f64;
    for index in 0..count - 1 {
        let length = distance(points[index.saturating_sub(1)], points[index + 1]);
        let endpoint_index = if junction { count - 2 - index } else { index };
        halves.push((width_for_index(river.widths[index]) * 0.5).min(endpoint_index as f32 * 0.05));
        lengths.push(length);
        total += f64::from(length);
    }
    halves.push(halves[count - 2]);
    let unfaded = (total - f64::from(defines.fade_from)).max(0.0);
    let mut along = 0.0_f64;
    let mut alphas: Vec<f32> = lengths
        .iter()
        .map(|&length| {
            along += f64::from(length);
            let distance = if junction { along } else { total - along };
            let alpha =
                1.0 - (distance - unfaded) / f64::from(defines.fade_from - defines.fade_to);
            (alpha as f32).clamp(0.0, 1.0)
        })
        .collect();
    alphas.push(f32::from(!junction));
    let direction = match river.seed {
        Seed::Junction(value) if value & 1 != 0 => 1.0,
        _ => -1.0,
    };
    let every: Vec<usize> = (0..count).collect();
    let fine = frames(&points, &halves, &every);
    let base = mesh.vertices.len();
    push_rails(&points, &every, &fine, &alphas, heights, &mut mesh.vertices);
    quads(&mut mesh.indices, &mut mesh.runs, base, count, direction);
    let (kept, coarse) = coarse_points(&points, &halves, &alphas, &fine, heights);
    let (coarse_base, coarse_count) = if kept.len() as f32 > count as f32 * COARSE_KEEPS_AT_MOST {
        (base, count)
    } else {
        let coarse_base = mesh.vertices.len();
        push_rails(&points, &kept, &coarse, &alphas, heights, &mut mesh.vertices);
        (coarse_base, kept.len())
    };
    quads(&mut mesh.coarse_indices, &mut mesh.coarse_runs, coarse_base, coarse_count, direction);
    mesh.ribbons += 1;
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    tangent: [f32; 2],
    offset: [f32; 2],
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    (d[0] * d[0] + d[1] * d[1]).sqrt()
}

fn rail(point: [f32; 2], offset: [f32; 2], sign: f32) -> [f32; 2] {
    [point[0] + offset[0] * sign, point[1] + offset[1] * sign]
}

fn frames(points: &[[f32; 2]], halves: &[f32], kept: &[usize]) -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::with_capacity(kept.len());
    for (at, &index) in kept.iter().enumerate() {
        let tangent = match kept.get(at + 1) {
            Some(&next) => {
                let previous = points[kept[at.saturating_sub(1)]];
                let next = points[next];
                let length = distance(previous, next);
                if length > 0.0 {
                    [(next[0] - previous[0]) / length, (next[1] - previous[1]) / length]
                } else {
                    [0.0; 2]
                }
            }
            None => frames.last().map_or([0.0; 2], |frame| frame.tangent),
        };
        let half = halves[index];
        frames.push(Frame {
            tangent,
            offset: [tangent[1] * half, -tangent[0] * half],
        });
    }
    frames
}

fn push_rails(
    points: &[[f32; 2]],
    kept: &[usize],
    frames: &[Frame],
    alphas: &[f32],
    heights: &HeightField<'_>,
    out: &mut Vec<RiverVertex>,
) {
    for (&index, frame) in kept.iter().zip(frames) {
        for (side, sign) in [(0.0_f32, 1.0_f32), (1.0, -1.0)] {
            let [x, z] = rail(points[index], frame.offset, sign);
            out.push(RiverVertex {
                position: [x, river_height(heights, x, z), z, alphas[index]],
                uv_tangent: [side, index as f32, frame.tangent[0], frame.tangent[1]],
            });
        }
    }
}

fn coarse_points(
    points: &[[f32; 2]],
    halves: &[f32],
    alphas: &[f32],
    fine: &[Frame],
    heights: &HeightField<'_>,
) -> (Vec<usize>, Vec<Frame>) {
    use viewer_scene_base::line_lod::{self, TOLERANCE, lerp};
    let centre: Vec<f32> = points.iter().map(|point| river_height(heights, point[0], point[1])).collect();
    let deviation = |a: usize, b: usize, index: usize| {
        let (off, t) = line_lod::offset(points[a], points[b], points[index]);
        let widen = (halves[index] - lerp(halves[a], halves[b], t)).abs();
        let rise = (centre[index] - lerp(centre[a], centre[b], t)).abs();
        let fade = (alphas[index] - lerp(alphas[a], alphas[b], t)).abs();
        ((off + widen) / TOLERANCE)
            .max(rise / line_lod::HEIGHT_TOLERANCE)
            .max(fade / COARSE_ALPHA_TOLERANCE)
    };
    let mut kept = line_lod::simplify(points.len(), &[], &deviation);
    loop {
        let frames = frames(points, halves, &kept);
        let mut more = Vec::new();
        for (at, &index) in kept.iter().enumerate() {
            if distance(frames[at].offset, fine[index].offset) > TOLERANCE {
                if at > 0 && kept[at - 1] + 1 < index {
                    more.push(index - 1);
                }
                if kept.get(at + 1).is_some_and(|&next| next > index + 1) {
                    more.push(index + 1);
                }
            }
            let Some(&next) = kept.get(at + 1) else {
                continue;
            };
            let mut scalar = (1.0, None);
            let mut worst = (TOLERANCE, None);
            for dropped in index + 1..next {
                let off = deviation(index, next, dropped);
                if off > scalar.0 {
                    scalar = (off, Some(dropped));
                }
                for sign in [1.0, -1.0] {
                    let (off, t) = line_lod::offset(
                        rail(points[index], frames[at].offset, sign),
                        rail(points[next], frames[at + 1].offset, sign),
                        rail(points[dropped], fine[dropped].offset, sign),
                    );
                    if off > worst.0 {
                        worst = (off, Some(dropped));
                    }
                    let fade = (alphas[dropped] - lerp(alphas[index], alphas[next], t)).abs() / COARSE_ALPHA_TOLERANCE;
                    if fade > scalar.0 {
                        scalar = (fade, Some(dropped));
                    }
                }
            }
            more.extend(worst.1);
            more.extend(scalar.1);
        }
        if more.is_empty() {
            return (kept, frames);
        }
        kept.extend(more);
        kept.sort_unstable();
        kept.dedup();
    }
}


pub struct RiverAssets {
    pub shader: Arc<shader_pipeline::library::Effect>,
    pub diffuse: dds::Texture,
    pub normal: dds::Texture,
    pub masks: dds::Texture,
}

impl RiverAssets {
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

pub struct RiverPass {
    effect: Arc<shader_pipeline::library::Effect>,
    compiled: RefCell<BTreeMap<bool, Result<super::river_shader::River, String>>>,
    samples: u32,
    format: wgpu::TextureFormat,
    runs: Vec<super::river_shader::Run>,
    coarse_runs: Vec<super::river_shader::Run>,
    diffuse: Bound,
    normal: Bound,
    masks: Bound,
    vertices: Option<wgpu::Buffer>,
    indices: Option<wgpu::Buffer>,
}

impl RiverPass {
    pub fn texel_reads(&self, high: bool, visit: &mut dyn FnMut(&str)) {
        if let Some(Ok(river)) = self.compiled.borrow().get(&high)
            && let Some(reads) = &river.texel_reads
        {
            visit(reads);
        }
    }

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
            coarse_runs: Vec::new(),
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

    pub fn upload(&mut self, device: &wgpu::Device, mesh: &RiverMesh) -> io::Result<()> {
        self.runs.clone_from(&mesh.runs);
        let index_base = mesh.indices.len() as u32;
        self.coarse_runs = mesh
            .coarse_runs
            .iter()
            .map(|run| super::river_shader::Run {
                indices: run.indices.start + index_base..run.indices.end + index_base,
                direction: run.direction,
            })
            .collect();
        if mesh.is_empty() {
            self.vertices = None;
            self.indices = None;
            return Ok(());
        }
        let indices = [mesh.indices.as_slice(), mesh.coarse_indices.as_slice()].concat();
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
            bytemuck::cast_slice(&indices),
        )?);
        Ok(())
    }

    pub fn textures(&self) -> [&wgpu::TextureView; 3] {
        [&self.diffuse.view, &self.normal.view, &self.masks.view]
    }

    pub fn warm(&self, device: &wgpu::Device, high: bool) {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            self.compiled.borrow_mut().entry(high)
        {
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
    }

    pub fn draw(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        inputs: super::river_shader::Inputs<'_>,
        high: bool,
        coarse: bool,
    ) -> io::Result<()> {
        let (Some(vertices), Some(indices)) = (&self.vertices, &self.indices) else {
            return Ok(());
        };
        self.warm(arena.device(), high);
        let compiled = self.compiled.borrow();
        viewer_gpu::gpu_contract::compiled(&compiled[&high])?.draw(
            arena,
            encoder,
            targets,
            inputs,
            super::river_shader::Geometry {
                vertices,
                indices,
                runs: if coarse { &self.coarse_runs } else { &self.runs },
            },
        )
    }
}

#[cfg(test)]
mod tests;
