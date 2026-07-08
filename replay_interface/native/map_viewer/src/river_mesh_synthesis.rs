use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

// PAN2 river draw decode: stride 32 is two f32x4 values. POSITION is
// (world_x, heightmap_r8 * 0.1, world_z, fade), TEXCOORD0 is
// (side, station, tangent_x, tangent_z), and rivers.bmp maps as
// bmp_x=round(world_x), bmp_y=2047-round(world_z) on the 5632x2048 map.
// The captured class-3 run ramps from width 0.0 to about 1.1 world units over
// the first 11 stations, then holds near 1.1.
pub(crate) const RIVER_VERTEX_STRIDE_BYTES: u32 = 32;
const RIVER_SOURCE_FADE_STEPS: f32 = 17.0;
const RIVER_SOURCE_WIDTH_RAMP_STEPS: f32 = 11.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiverMeshSynthesisStats {
    pub schema: String,
    pub map_width: usize,
    pub map_height: usize,
    pub river_pixel_count: usize,
    pub polyline_count: usize,
    pub vertex_count: usize,
    pub index_count: usize,
    pub vertex_stride_bytes: u32,
    pub index_format: u32,
    pub class_counts: BTreeMap<u8, usize>,
    pub convention: String,
}

#[derive(Clone, Debug)]
pub struct RiverMeshSynthesis {
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub stats: RiverMeshSynthesisStats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiverCaptureDecodeReport {
    pub schema: String,
    pub draw_count: u32,
    pub base_vertex_location: i32,
    pub unique_vertex_count: usize,
    pub x_range: [f32; 2],
    pub z_range: [f32; 2],
    pub first_station: RiverCaptureStation,
    pub max_height_error: f32,
    pub max_tangent_length_error: f32,
    pub convention: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RiverCaptureStation {
    pub center_x: f32,
    pub center_z: f32,
    pub width: f32,
    pub bmp_x: usize,
    pub bmp_y: usize,
    pub river_class: u8,
    pub height_sample: u8,
}

#[derive(Clone, Debug)]
pub struct MapLayerSynthesisInputs {
    pub bundle_dir: PathBuf,
    pub map_width: usize,
    pub map_height: usize,
    pub rivers_path: PathBuf,
    pub heightmap_path: PathBuf,
    pub trees_path: PathBuf,
    pub tree_width: usize,
    pub tree_height: usize,
}

impl MapLayerSynthesisInputs {
    pub fn from_bundle_dir(bundle_dir: &Path) -> Result<Self> {
        let manifest_path = bundle_dir.join("manifest.json");
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|err| {
                format!(
                    "map layer synthesis blocked: cannot read {}: {err}",
                    manifest_path.display()
                )
            })?)?;
        let map_width = manifest.dimensions.width;
        let map_height = manifest.dimensions.height;
        let tree_dims = manifest
            .source
            .visual_input_dimensions
            .get("map/trees.bmp")
            .ok_or("map layer synthesis blocked: manifest has no map/trees.bmp dimensions")?;
        Ok(Self {
            bundle_dir: bundle_dir.to_path_buf(),
            map_width,
            map_height,
            rivers_path: bundle_dir.join(&manifest.files.rivers),
            heightmap_path: bundle_dir.join(&manifest.files.heightmap),
            trees_path: bundle_dir.join(&manifest.files.trees),
            tree_width: tree_dims.width,
            tree_height: tree_dims.height,
        })
    }

    pub(crate) fn load_rivers(&self) -> Result<IndexImage> {
        load_raw_index_image(
            &self.rivers_path,
            self.map_width,
            self.map_height,
            "rivers.r8",
        )
    }

    pub(crate) fn load_heightmap(&self) -> Result<IndexImage> {
        load_raw_index_image(
            &self.heightmap_path,
            self.map_width,
            self.map_height,
            "heightmap.r8",
        )
    }
}

#[allow(dead_code)]
pub fn synthesize_river_mesh_from_bundle_dir(bundle_dir: &Path) -> Result<RiverMeshSynthesis> {
    let inputs = MapLayerSynthesisInputs::from_bundle_dir(bundle_dir)?;
    synthesize_river_mesh(&inputs)
}

pub fn synthesize_river_mesh(inputs: &MapLayerSynthesisInputs) -> Result<RiverMeshSynthesis> {
    let rivers = inputs.load_rivers()?;
    let heightmap = inputs.load_heightmap()?;
    synthesize_river_mesh_from_images(&rivers, &heightmap)
}

pub(crate) fn synthesize_river_world_indices(
    vb_bytes: &[u8],
    stride: usize,
    break_distance: f32,
) -> Result<Vec<u32>> {
    if stride == 0 {
        return Err("production river world index synthesis blocked: vertex stride is zero".into());
    }
    if stride < 12 {
        return Err(format!(
            "production river world index synthesis blocked: vertex stride {stride} is smaller than position bytes"
        )
        .into());
    }
    if !break_distance.is_finite() || break_distance < 0.0 {
        return Err(format!(
            "production river world index synthesis blocked: break distance {break_distance} is invalid"
        )
        .into());
    }
    if vb_bytes.len() % stride != 0 {
        return Err(format!(
            "production river world index synthesis blocked: vertex buffer length {} is not divisible by stride {stride}",
            vb_bytes.len()
        )
        .into());
    }
    let vertex_count = vb_bytes.len() / stride;
    if vertex_count < 4 {
        return Err(format!(
            "production river world index synthesis blocked: vertex buffer has {vertex_count} vertices, expected at least 4"
        )
        .into());
    }

    let mut positions = Vec::with_capacity(vertex_count);
    for vertex_index in 0..vertex_count {
        let offset = vertex_index * stride;
        let x = f32::from_le_bytes(vb_bytes[offset..offset + 4].try_into()?);
        let y = f32::from_le_bytes(vb_bytes[offset + 4..offset + 8].try_into()?);
        let z = f32::from_le_bytes(vb_bytes[offset + 8..offset + 12].try_into()?);
        if !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(format!(
                "production river world index synthesis blocked: vertex {vertex_index} has non-finite position ({x}, {y}, {z})"
            )
            .into());
        }
        positions.push([x, y, z]);
    }

    let pair_count = vertex_count / 2;
    let break_distance_sq = break_distance * break_distance;
    let mut indices = Vec::with_capacity(pair_count.saturating_sub(1) * 6);
    for pair_index in 0..pair_count.saturating_sub(1) {
        let even = pair_index * 2;
        let next_even = even + 2;
        let dx = positions[next_even][0] - positions[even][0];
        let dy = positions[next_even][1] - positions[even][1];
        let dz = positions[next_even][2] - positions[even][2];
        if dx * dx + dy * dy + dz * dz <= break_distance_sq {
            indices.push(u32::try_from(even)?);
            indices.push(u32::try_from(next_even)?);
            indices.push(u32::try_from(even + 1)?);
            indices.push(u32::try_from(next_even)?);
            indices.push(u32::try_from(next_even + 1)?);
            indices.push(u32::try_from(even + 1)?);
        }
    }
    Ok(indices)
}

pub(crate) fn synthesize_river_mesh_from_images(
    rivers: &IndexImage,
    heightmap: &IndexImage,
) -> Result<RiverMeshSynthesis> {
    if rivers.width != heightmap.width || rivers.height != heightmap.height {
        return Err(format!(
            "river mesh synthesis blocked: rivers map is {}x{} but heightmap is {}x{}",
            rivers.width, rivers.height, heightmap.width, heightmap.height
        )
        .into());
    }
    let mut class_counts = BTreeMap::new();
    let mut river_pixel_count = 0usize;
    for value in &rivers.pixels {
        if is_river_pixel(*value) {
            *class_counts.entry(*value).or_insert(0) += 1;
            river_pixel_count += 1;
        }
    }

    let polylines = trace_river_polylines(rivers);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for polyline in &polylines {
        append_river_polyline_mesh(polyline, rivers, heightmap, &mut vertices, &mut indices)?;
    }

    let stats = RiverMeshSynthesisStats {
        schema: "battle-plan-river-mesh-synthesis-v1".to_string(),
        map_width: rivers.width,
        map_height: rivers.height,
        river_pixel_count,
        polyline_count: polylines.len(),
        vertex_count: vertices.len() / RIVER_VERTEX_STRIDE_BYTES as usize,
        index_count: indices.len(),
        vertex_stride_bytes: RIVER_VERTEX_STRIDE_BYTES,
        index_format: DXGI_FORMAT_R32_UINT,
        class_counts,
        convention: "vPosition=float4(x,heightmap*0.1,z,source_fade); vUV_Tangent=float4(side,station,tangent.x,tangent.z); world=(bmp_x,map_height-1-bmp_y)".to_string(),
    };
    Ok(RiverMeshSynthesis {
        vertices,
        indices,
        stats,
    })
}

#[allow(dead_code)]
pub(crate) fn decode_captured_river_slice(
    bundle_dir: &Path,
    draw: &MapFrameDraw,
    rivers: &IndexImage,
    heightmap: &IndexImage,
) -> Result<RiverCaptureDecodeReport> {
    let vertex_binding = draw.vertex_buffers.get("0").ok_or(
        "river capture decode blocked: captured river draw has no IA vertex buffer slot 0",
    )?;
    let index_binding = draw
        .index_buffer
        .as_ref()
        .ok_or("river capture decode blocked: captured river draw has no index buffer")?;
    let vertex_bytes = read_map_frame_buffer_bytes(bundle_dir, vertex_binding, "river vertices")?;
    let index_file = index_binding
        .file
        .as_ref()
        .ok_or("river capture decode blocked: index buffer has no file")?;
    let index_hash = index_binding
        .fnv1a64
        .as_ref()
        .ok_or("river capture decode blocked: index buffer has no hash")?;
    let index_bytes = read_verified_map_frame_file(
        bundle_dir,
        index_file,
        index_binding.byte_len,
        index_hash,
        "river indices",
    )?;
    if vertex_binding.stride != Some(RIVER_VERTEX_STRIDE_BYTES) {
        return Err(format!(
            "river capture decode blocked: captured stride {:?}, expected {RIVER_VERTEX_STRIDE_BYTES}",
            vertex_binding.stride
        )
        .into());
    }
    if draw.base_vertex_location < 0 {
        return Err("river capture decode blocked: negative base vertex location".into());
    }
    let mut unique = HashSet::new();
    for index in 0..draw.draw_count as usize {
        let raw = read_u16_le(&index_bytes, index * 2)?;
        unique.insert(draw.base_vertex_location as usize + raw as usize);
    }
    let mut x_min = f32::INFINITY;
    let mut x_max = f32::NEG_INFINITY;
    let mut z_min = f32::INFINITY;
    let mut z_max = f32::NEG_INFINITY;
    let mut max_height_error: f32 = 0.0;
    let mut max_tangent_length_error: f32 = 0.0;
    for vertex in &unique {
        let offset = vertex
            .checked_mul(RIVER_VERTEX_STRIDE_BYTES as usize)
            .ok_or("river capture decode blocked: vertex byte offset overflow")?;
        let x = read_f32_le(&vertex_bytes, offset)?;
        let y = read_f32_le(&vertex_bytes, offset + 4)?;
        let z = read_f32_le(&vertex_bytes, offset + 8)?;
        let tx = read_f32_le(&vertex_bytes, offset + 24)?;
        let tz = read_f32_le(&vertex_bytes, offset + 28)?;
        x_min = x_min.min(x);
        x_max = x_max.max(x);
        z_min = z_min.min(z);
        z_max = z_max.max(z);
        let expected_y = sample_height_world(heightmap, x, z);
        max_height_error = max_height_error.max((y - expected_y).abs());
        max_tangent_length_error =
            max_tangent_length_error.max(((tx * tx + tz * tz).sqrt() - 1.0).abs());
    }

    let first = decode_captured_station(
        &vertex_bytes,
        draw.base_vertex_location as usize,
        rivers,
        heightmap,
    )?;
    Ok(RiverCaptureDecodeReport {
        schema: "battle-plan-river-capture-decode-v1".to_string(),
        draw_count: draw.draw_count,
        base_vertex_location: draw.base_vertex_location,
        unique_vertex_count: unique.len(),
        x_range: [x_min, x_max],
        z_range: [z_min, z_max],
        first_station: first,
        max_height_error,
        max_tangent_length_error,
        convention: "captured bytes are two float4s: position.xyzw at offsets 0..15 and uv_tangent.xyzw at 16..31; BMP y is map_height-1-world_z".to_string(),
    })
}

#[allow(dead_code)]
fn decode_captured_station(
    vertex_bytes: &[u8],
    base_vertex: usize,
    rivers: &IndexImage,
    heightmap: &IndexImage,
) -> Result<RiverCaptureStation> {
    let a = base_vertex * RIVER_VERTEX_STRIDE_BYTES as usize;
    let b = a + RIVER_VERTEX_STRIDE_BYTES as usize;
    let x0 = read_f32_le(vertex_bytes, a)?;
    let z0 = read_f32_le(vertex_bytes, a + 8)?;
    let x1 = read_f32_le(vertex_bytes, b)?;
    let z1 = read_f32_le(vertex_bytes, b + 8)?;
    let center_x = (x0 + x1) * 0.5;
    let center_z = (z0 + z1) * 0.5;
    let width = ((x1 - x0).powi(2) + (z1 - z0).powi(2)).sqrt();
    let bmp_x = center_x.round().clamp(0.0, (rivers.width - 1) as f32) as usize;
    let bmp_y = world_z_to_bmp_y(center_z, rivers.height);
    Ok(RiverCaptureStation {
        center_x,
        center_z,
        width,
        bmp_x,
        bmp_y,
        river_class: rivers.pixels[bmp_y * rivers.width + bmp_x],
        height_sample: heightmap.pixels[bmp_y * heightmap.width + bmp_x],
    })
}

fn append_river_polyline_mesh(
    polyline: &[usize],
    rivers: &IndexImage,
    heightmap: &IndexImage,
    vertices: &mut Vec<u8>,
    indices: &mut Vec<u32>,
) -> Result<()> {
    if polyline.len() < 2 {
        return Ok(());
    }
    let vertex_base = u32::try_from(vertices.len() / RIVER_VERTEX_STRIDE_BYTES as usize)?;
    let mut station = 0.0f32;
    for (i, pixel) in polyline.iter().copied().enumerate() {
        let (x, y) = index_xy(pixel, rivers.width);
        let center_x = x as f32;
        let center_z = (rivers.height - 1 - y) as f32;
        if i > 0 {
            let (px, py) = index_xy(polyline[i - 1], rivers.width);
            let px = px as f32;
            let pz = (rivers.height - 1 - py) as f32;
            station += ((center_x - px).powi(2) + (center_z - pz).powi(2)).sqrt();
        }
        let (tx, tz) = tangent_for(polyline, i, rivers.width, rivers.height);
        let nx = -tz;
        let nz = tx;
        let class = rivers.pixels[pixel];
        let base_width = river_width_for_class(class)
            .or_else(|| nearest_polyline_width(polyline, i, rivers))
            .unwrap_or(1.1);
        let width_ramp = (station / RIVER_SOURCE_WIDTH_RAMP_STEPS).clamp(0.0, 1.0);
        let width = base_width * width_ramp;
        let alpha = (station / RIVER_SOURCE_FADE_STEPS).clamp(0.0, 1.0);
        let half = width * 0.5;
        let left_x = center_x - nx * half;
        let left_z = center_z - nz * half;
        let right_x = center_x + nx * half;
        let right_z = center_z + nz * half;
        push_river_vertex(
            vertices,
            left_x,
            sample_height_world(heightmap, left_x, left_z),
            left_z,
            alpha,
            0.0,
            station,
            tx,
            tz,
        );
        push_river_vertex(
            vertices,
            right_x,
            sample_height_world(heightmap, right_x, right_z),
            right_z,
            alpha,
            1.0,
            station,
            tx,
            tz,
        );
    }
    for i in 0..polyline.len() - 1 {
        let left = vertex_base + u32::try_from(i * 2)?;
        let right = left + 1;
        let next_left = left + 2;
        let next_right = left + 3;
        indices.extend_from_slice(&[left, next_left, right, next_left, next_right, right]);
    }
    Ok(())
}

fn push_river_vertex(
    out: &mut Vec<u8>,
    x: f32,
    y: f32,
    z: f32,
    alpha: f32,
    side: f32,
    station: f32,
    tangent_x: f32,
    tangent_z: f32,
) {
    for value in [x, y, z, alpha, side, station, tangent_x, tangent_z] {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn trace_river_polylines(rivers: &IndexImage) -> Vec<Vec<usize>> {
    let river_pixels = rivers
        .pixels
        .iter()
        .enumerate()
        .filter_map(|(index, value)| is_river_pixel(*value).then_some(index))
        .collect::<Vec<_>>();
    let river_set = river_pixels.iter().copied().collect::<HashSet<_>>();
    let mut visited_edges = HashSet::new();
    let mut polylines = Vec::new();
    let starts = river_pixels
        .iter()
        .copied()
        .filter(|index| river_neighbors(*index, rivers.width, rivers.height, &river_set).len() != 2)
        .chain(river_pixels.iter().copied())
        .collect::<Vec<_>>();
    for start in starts {
        for next in river_neighbors(start, rivers.width, rivers.height, &river_set) {
            let edge = normalized_edge(start, next);
            if visited_edges.contains(&edge) {
                continue;
            }
            let path = follow_river_edge(
                start,
                next,
                rivers.width,
                rivers.height,
                &river_set,
                &mut visited_edges,
            );
            if path.len() >= 2 {
                polylines.push(path);
            }
        }
    }
    polylines
}

fn follow_river_edge(
    start: usize,
    next: usize,
    width: usize,
    height: usize,
    river_set: &HashSet<usize>,
    visited_edges: &mut HashSet<(usize, usize)>,
) -> Vec<usize> {
    let mut path = vec![start, next];
    visited_edges.insert(normalized_edge(start, next));
    let mut prev = start;
    let mut current = next;
    loop {
        let mut candidates = river_neighbors(current, width, height, river_set)
            .into_iter()
            .filter(|candidate| *candidate != prev)
            .filter(|candidate| !visited_edges.contains(&normalized_edge(current, *candidate)))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            break;
        }
        candidates.sort_by_key(|candidate| turn_cost(prev, current, *candidate, width, height));
        let chosen = candidates[0];
        visited_edges.insert(normalized_edge(current, chosen));
        path.push(chosen);
        prev = current;
        current = chosen;
        if river_neighbors(current, width, height, river_set).len() != 2 {
            break;
        }
    }
    path
}

fn river_neighbors(
    index: usize,
    width: usize,
    height: usize,
    river_set: &HashSet<usize>,
) -> Vec<usize> {
    let (x, y) = index_xy(index, width);
    let mut out = Vec::new();
    for dy in -1isize..=1 {
        for dx in -1isize..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let nx = x as isize + dx;
            let ny = y as isize + dy;
            if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                continue;
            }
            let candidate = ny as usize * width + nx as usize;
            if river_set.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

fn turn_cost(prev: usize, current: usize, next: usize, width: usize, height: usize) -> i32 {
    let (px, py) = world_xy(prev, width, height);
    let (cx, cy) = world_xy(current, width, height);
    let (nx, ny) = world_xy(next, width, height);
    let ax = cx - px;
    let ay = cy - py;
    let bx = nx - cx;
    let by = ny - cy;
    let dot = ax * bx + ay * by;
    -dot
}

fn tangent_for(polyline: &[usize], index: usize, width: usize, height: usize) -> (f32, f32) {
    let previous = if index == 0 {
        polyline[index]
    } else {
        polyline[index - 1]
    };
    let next = if index + 1 >= polyline.len() {
        polyline[index]
    } else {
        polyline[index + 1]
    };
    let (px, pz) = world_xy(previous, width, height);
    let (nx, nz) = world_xy(next, width, height);
    let dx = (nx - px) as f32;
    let dz = (nz - pz) as f32;
    let len = (dx * dx + dz * dz).sqrt();
    if len <= f32::EPSILON {
        (0.0, 1.0)
    } else {
        (dx / len, dz / len)
    }
}

fn nearest_polyline_width(polyline: &[usize], index: usize, rivers: &IndexImage) -> Option<f32> {
    let mut queue = VecDeque::new();
    queue.push_back(index);
    let mut visited = HashSet::new();
    while let Some(candidate) = queue.pop_front() {
        if !visited.insert(candidate) {
            continue;
        }
        if let Some(width) = river_width_for_class(rivers.pixels[polyline[candidate]]) {
            return Some(width);
        }
        if candidate > 0 {
            queue.push_back(candidate - 1);
        }
        if candidate + 1 < polyline.len() {
            queue.push_back(candidate + 1);
        }
    }
    None
}

fn river_width_for_class(class: u8) -> Option<f32> {
    match class {
        3 => Some(1.1),
        4 => Some(1.4),
        5 => Some(1.7),
        6 => Some(2.0),
        7 => Some(2.4),
        8 => Some(2.8),
        9 => Some(3.2),
        10 => Some(3.7),
        11 => Some(4.2),
        _ => None,
    }
}

fn sample_height_world(heightmap: &IndexImage, x: f32, z: f32) -> f32 {
    let xi = x.round().clamp(0.0, (heightmap.width - 1) as f32) as usize;
    let yi = world_z_to_bmp_y(z, heightmap.height);
    heightmap.pixels[yi * heightmap.width + xi] as f32 * 0.1
}

fn world_z_to_bmp_y(z: f32, height: usize) -> usize {
    ((height - 1) as f32 - z.round()).clamp(0.0, (height - 1) as f32) as usize
}

fn is_river_pixel(value: u8) -> bool {
    value <= 11
}

fn normalized_edge(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn index_xy(index: usize, width: usize) -> (usize, usize) {
    (index % width, index / width)
}

fn world_xy(index: usize, width: usize, height: usize) -> (i32, i32) {
    let (x, y) = index_xy(index, width);
    (x as i32, (height - 1 - y) as i32)
}

fn load_raw_index_image(
    path: &Path,
    width: usize,
    height: usize,
    label: &str,
) -> Result<IndexImage> {
    let pixels = fs::read(path).map_err(|err| {
        format!(
            "map layer synthesis blocked: cannot read {} at {}: {err}",
            label,
            path.display()
        )
    })?;
    let expected = width
        .checked_mul(height)
        .ok_or("map layer synthesis blocked: image dimensions overflow")?;
    if pixels.len() != expected {
        return Err(format!(
            "map layer synthesis blocked: {} has {} bytes, expected {} for {}x{}",
            path.display(),
            pixels.len(),
            expected,
            width,
            height
        )
        .into());
    }
    Ok(IndexImage {
        width,
        height,
        pixels,
    })
}

#[allow(dead_code)]
fn read_f32_le(bytes: &[u8], offset: usize) -> Result<f32> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or("river capture decode blocked: truncated f32")?;
    Ok(f32::from_le_bytes(raw.try_into()?))
}

#[allow(dead_code)]
fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16> {
    let raw = bytes
        .get(offset..offset + 2)
        .ok_or("river capture decode blocked: truncated u16")?;
    Ok(u16::from_le_bytes(raw.try_into()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_world_river_vertex(bytes: &mut Vec<u8>, x: f32, y: f32, z: f32) {
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes.extend_from_slice(&z.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 20]);
    }

    #[test]
    fn synthesized_river_world_indices_split_at_distance_breaks() {
        let mut vb = Vec::new();
        push_world_river_vertex(&mut vb, 0.0, 11.7, 0.0);
        push_world_river_vertex(&mut vb, 0.0, 11.7, 1.0);
        push_world_river_vertex(&mut vb, 3.0, 11.7, 0.0);
        push_world_river_vertex(&mut vb, 3.0, 11.7, 1.0);
        push_world_river_vertex(&mut vb, 20.0, 11.7, 0.0);
        push_world_river_vertex(&mut vb, 20.0, 11.7, 1.0);
        push_world_river_vertex(&mut vb, 23.0, 11.7, 0.0);
        push_world_river_vertex(&mut vb, 23.0, 11.7, 1.0);

        let indices = synthesize_river_world_indices(&vb, 32, 8.0).expect("indices");
        assert_eq!(indices, vec![0, 2, 1, 2, 3, 1, 4, 6, 5, 6, 7, 5]);
    }

    #[test]
    fn synthesized_river_world_indices_reject_invalid_vertex_buffers() {
        let mut short_vb = Vec::new();
        for vertex in 0..3 {
            push_world_river_vertex(&mut short_vb, vertex as f32, 11.7, 0.0);
        }
        let err = synthesize_river_world_indices(&short_vb, 32, 8.0).unwrap_err();
        assert!(err.to_string().contains("expected at least 4"));

        let err = synthesize_river_world_indices(&[0u8; 33], 32, 8.0).unwrap_err();
        assert!(err.to_string().contains("not divisible by stride"));

        let mut nan_vb = Vec::new();
        push_world_river_vertex(&mut nan_vb, 0.0, 11.7, 0.0);
        push_world_river_vertex(&mut nan_vb, 0.0, 11.7, 1.0);
        push_world_river_vertex(&mut nan_vb, f32::NAN, 11.7, 0.0);
        push_world_river_vertex(&mut nan_vb, 3.0, 11.7, 1.0);
        let err = synthesize_river_world_indices(&nan_vb, 32, 8.0).unwrap_err();
        assert!(err.to_string().contains("non-finite position"));
    }

    #[test]
    fn synthesized_river_mesh_uses_float4_layout_and_flipped_world_z() {
        let rivers = IndexImage {
            width: 4,
            height: 4,
            pixels: vec![
                255, 255, 255, 255, 255, 3, 3, 255, 255, 255, 3, 255, 255, 255, 255, 255,
            ],
        };
        let heightmap = IndexImage {
            width: 4,
            height: 4,
            pixels: vec![100; 16],
        };
        let mesh = synthesize_river_mesh_from_images(&rivers, &heightmap).expect("mesh");
        assert_eq!(mesh.stats.vertex_stride_bytes, 32);
        assert_eq!(mesh.stats.index_format, DXGI_FORMAT_R32_UINT);
        assert!(mesh.stats.vertex_count >= 6);
        assert!(mesh.stats.index_count >= 6);
        let x = f32::from_le_bytes(mesh.vertices[0..4].try_into().unwrap());
        let y = f32::from_le_bytes(mesh.vertices[4..8].try_into().unwrap());
        let z = f32::from_le_bytes(mesh.vertices[8..12].try_into().unwrap());
        assert!(x.is_finite());
        assert_eq!(y, 10.0);
        assert!(z.is_finite());
    }
}
