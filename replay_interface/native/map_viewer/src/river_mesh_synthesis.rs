use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

// PAN2 river draw decode: stride 32 is two f32x4 values. POSITION is
// (world_x, heightmap_r8 * 0.1, world_z, fade), TEXCOORD0 is
// (side, station, tangent_x, tangent_z), and rivers.bmp maps as
// bmp_x=round(world_x), bmp_y=2047-round(world_z) on the 5632x2048 map.
pub(crate) const RIVER_VERTEX_STRIDE_BYTES: u32 = 32;
// Source-strip width ramp, world units per station. CAPTURE-measured from the
// world river VB (see river_world_synthesis: `width = min(0.1*station,
// width(class))`). This REPLACES the retired capture-FIT literal
// `RIVER_SOURCE_WIDTH_RAMP_STEPS = 11.0`. The class->width map itself is
// engine CPU mesh-gen — the river shader passes POSITION through unchanged
// (river.shader:198), so width is baked into the VB, not file-derivable; the
// only file-grounded distinction is the small/large split
// (00_defines.lua:936-938). See FIDELITY_LEDGER 2.10.
const RIVER_WIDTH_RAMP_PER_STATION: f32 = 0.1;

/// `rivers.bmp` palette-split defines, shipped in
/// `common/defines/00_defines.lua:936-938` (comment "color indices for
/// rivers"): `RIVER_SMALL_START_INDEX = 0`, `RIVER_SMALL_STOP_INDEX = 6`,
/// `RIVER_LARGE_STOP_INDEX = 11` — palette indices `0..=6` are SMALL rivers,
/// `7..=11` are LARGE rivers, and anything above `11` is not a river pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverPaletteDefines {
    /// `RIVER_SMALL_START_INDEX` (00_defines.lua:936).
    pub small_start_index: u8,
    /// `RIVER_SMALL_STOP_INDEX` (00_defines.lua:937).
    pub small_stop_index: u8,
    /// `RIVER_LARGE_STOP_INDEX` (00_defines.lua:938).
    pub large_stop_index: u8,
}

pub const RIVER_PALETTE_DEFINES_SOURCE: &str = "common/defines/00_defines.lua";
pub const RIVER_FADE_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

impl RiverPaletteDefines {
    /// Read the palette split through the mod-aware resolver.
    pub fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let path =
            resolver.resolve_required(RIVER_PALETTE_DEFINES_SOURCE, "river palette defines")?;
        let text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        Self::from_defines_source(&text)
    }

    /// Parse the three `RIVER_*_INDEX` defines from `00_defines.lua` text.
    pub fn from_defines_source(source: &str) -> Result<Self> {
        let small_start_index = parse_lua_u8_define(source, "RIVER_SMALL_START_INDEX")?;
        let small_stop_index = parse_lua_u8_define(source, "RIVER_SMALL_STOP_INDEX")?;
        let large_stop_index = parse_lua_u8_define(source, "RIVER_LARGE_STOP_INDEX")?;
        let defines = Self {
            small_start_index,
            small_stop_index,
            large_stop_index,
        };
        defines.validate()?;
        Ok(defines)
    }

    fn validate(self) -> Result<()> {
        if self.small_start_index > self.small_stop_index
            || self.small_stop_index >= self.large_stop_index
        {
            return Err(format!(
                "river palette defines are inconsistent: small {}..={} / large ..={}",
                self.small_start_index, self.small_stop_index, self.large_stop_index
            )
            .into());
        }
        Ok(())
    }

    /// Whether a `rivers.bmp` palette index is a river pixel at all.
    pub fn is_river_pixel(self, value: u8) -> bool {
        (self.small_start_index..=self.large_stop_index).contains(&value)
    }

    /// SMALL river: `RIVER_SMALL_START_INDEX ..= RIVER_SMALL_STOP_INDEX`.
    pub fn is_small_river(self, value: u8) -> bool {
        (self.small_start_index..=self.small_stop_index).contains(&value)
    }

    /// LARGE river: `RIVER_SMALL_STOP_INDEX+1 ..= RIVER_LARGE_STOP_INDEX`.
    pub fn is_large_river(self, value: u8) -> bool {
        value > self.small_stop_index && value <= self.large_stop_index
    }
}

/// River ending fade window, shipped in `common/defines/00_graphics.lua:993-994`
/// (`RIVER_FADE_FROM = 20.0` "the last river endings got faded out, X distance
/// from the ending...", `RIVER_FADE_TO = 3.0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverFadeDefines {
    /// `RIVER_FADE_FROM` (00_graphics.lua:993).
    pub fade_from: f32,
    /// `RIVER_FADE_TO` (00_graphics.lua:994).
    pub fade_to: f32,
}

impl RiverFadeDefines {
    /// Read the fade window through the mod-aware resolver.
    pub fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let path = resolver.resolve_required(RIVER_FADE_DEFINES_SOURCE, "river fade defines")?;
        let text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        Self::from_graphics_source(&text)
    }

    /// Parse `RIVER_FADE_FROM`/`RIVER_FADE_TO` from `00_graphics.lua` text.
    pub fn from_graphics_source(source: &str) -> Result<Self> {
        let fade_from = parse_lua_f32_define(source, "RIVER_FADE_FROM")?;
        let fade_to = parse_lua_f32_define(source, "RIVER_FADE_TO")?;
        if !(fade_from.is_finite() && fade_to.is_finite() && fade_from > fade_to && fade_to >= 0.0)
        {
            return Err(format!(
                "river fade defines are inconsistent: RIVER_FADE_FROM {fade_from} / RIVER_FADE_TO {fade_to}"
            )
            .into());
        }
        Ok(Self { fade_from, fade_to })
    }

    /// The fade span in stations (map units along the centerline):
    /// `RIVER_FADE_FROM − RIVER_FADE_TO` = 17.0 for vanilla — exactly the
    /// value the old capture-fitted `RIVER_SOURCE_FADE_STEPS = 17.0` carried
    /// (decoded from the PAN2 class-3 run), so replacing the uncited literal
    /// with this defines-derived span is byte-identical for vanilla. The
    /// identification "source-fade ramp length == FADE_FROM − FADE_TO" is a
    /// documented reading grounded by that numeric match; the engine's exact
    /// per-strip fade smoothing remains UNTRACED (see river_world_synthesis).
    pub fn fade_span_stations(self) -> f32 {
        self.fade_from - self.fade_to
    }
}

fn parse_lua_f32_define(source: &str, key: &str) -> Result<f32> {
    let mut parsed = None;
    for line in source.lines() {
        let code = line.split("--").next().unwrap_or(line).trim();
        let Some(rest) = code.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let token = rest[1..]
            .trim_start()
            .trim_end_matches(',')
            .split_whitespace()
            .next()
            .ok_or_else(|| format!("{key} has no numeric value"))?
            .trim_end_matches(',');
        let value = token
            .parse::<f32>()
            .map_err(|err| format!("{key} value {token:?} is not a number: {err}"))?;
        if parsed.replace(value).is_some() {
            return Err(format!("{key} is defined more than once").into());
        }
    }
    parsed.ok_or_else(|| format!("defines source is missing {key}").into())
}

fn parse_lua_u8_define(source: &str, key: &str) -> Result<u8> {
    let value = parse_lua_f32_define(source, key)?;
    if value < 0.0 || value > u8::MAX as f32 || value.fract() != 0.0 {
        return Err(format!("{key} value {value} is not a u8 palette index").into());
    }
    Ok(value as u8)
}

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
/// Synthesize from a bundle dir, reading the river palette/fade defines
/// through the recording's mod-aware resolver (MOD_COMPATIBILITY.md).
pub fn synthesize_river_mesh_from_bundle_dir(
    bundle_dir: &Path,
    resolver: &GameFileResolver,
) -> Result<RiverMeshSynthesis> {
    let inputs = MapLayerSynthesisInputs::from_bundle_dir(bundle_dir)?;
    synthesize_river_mesh(
        &inputs,
        RiverPaletteDefines::from_resolver(resolver)?,
        RiverFadeDefines::from_resolver(resolver)?,
    )
}

pub fn synthesize_river_mesh(
    inputs: &MapLayerSynthesisInputs,
    palette: RiverPaletteDefines,
    fade: RiverFadeDefines,
) -> Result<RiverMeshSynthesis> {
    let rivers = inputs.load_rivers()?;
    let heightmap = inputs.load_heightmap()?;
    synthesize_river_mesh_from_images(&rivers, &heightmap, palette, fade)
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
    palette: RiverPaletteDefines,
    fade: RiverFadeDefines,
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
        if palette.is_river_pixel(*value) {
            *class_counts.entry(*value).or_insert(0) += 1;
            river_pixel_count += 1;
        }
    }

    let polylines = trace_river_polylines(rivers, palette);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for polyline in &polylines {
        append_river_polyline_mesh(
            polyline,
            rivers,
            heightmap,
            fade,
            &mut vertices,
            &mut indices,
        )?;
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
    fade: RiverFadeDefines,
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
            .ok_or("river polyline has no width-classified pixel (class 3..=11)")?;
        // Measured source ramp: width grows RIVER_WIDTH_RAMP_PER_STATION world
        // units per station up to the class steady width (river_world_synthesis,
        // CAPTURE). Fail-closed above: an uncited plausible fallback width is
        // forbidden (lane rule), so a fully-unclassified polyline errors.
        let width = (RIVER_WIDTH_RAMP_PER_STATION * station).min(base_width);
        // Source-ending fade over RIVER_FADE_FROM - RIVER_FADE_TO stations
        // (00_graphics.lua:993-994; vanilla span 17.0 == the old PAN2-fitted
        // RIVER_SOURCE_FADE_STEPS literal, so vanilla output is byte-identical).
        let alpha = (station / fade.fade_span_stations()).clamp(0.0, 1.0);
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

fn trace_river_polylines(rivers: &IndexImage, palette: RiverPaletteDefines) -> Vec<Vec<usize>> {
    let river_pixels = rivers
        .pixels
        .iter()
        .enumerate()
        .filter_map(|(index, value)| palette.is_river_pixel(*value).then_some(index))
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

/// Steady-state world-space width for a rivers.bmp palette class. Single
/// source of truth = the CAPTURE-measured world-VB law in
/// `river_world_synthesis` (`width(k) = 1.11 + 0.17*(k-3)`, verified for k in
/// {3,4,6,7,9,10,11}, classes 5/8 interpolated inside the same measured linear
/// law). The old fabricated per-class table (1.1..4.2, uncited) is retired.
/// Classes 0..=2 (source/flow markers) and >=12 (not-river) have no steady
/// width and return `None`.
fn river_width_for_class(class: u8) -> Option<f32> {
    crate::river_world_synthesis::river_world_width_for_class(class)
}

fn sample_height_world(heightmap: &IndexImage, x: f32, z: f32) -> f32 {
    let xi = x.round().clamp(0.0, (heightmap.width - 1) as f32) as usize;
    let yi = world_z_to_bmp_y(z, heightmap.height);
    heightmap.pixels[yi * heightmap.width + xi] as f32 * 0.1
}

fn world_z_to_bmp_y(z: f32, height: usize) -> usize {
    ((height - 1) as f32 - z.round()).clamp(0.0, (height - 1) as f32) as usize
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
        let mesh = synthesize_river_mesh_from_images(
            &rivers,
            &heightmap,
            vanilla_river_palette_defines(),
            vanilla_river_fade_defines(),
        )
        .expect("mesh");
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

    /// The vanilla palette values, parsed from the VERBATIM shipped define
    /// lines (00_defines.lua:936-938) rather than typed as bare numbers.
    fn vanilla_river_palette_defines() -> RiverPaletteDefines {
        RiverPaletteDefines::from_defines_source(
            r#"
	RIVER_SMALL_START_INDEX = 0,                   -- color indices for rivers
	RIVER_SMALL_STOP_INDEX = 6,
	RIVER_LARGE_STOP_INDEX = 11,
"#,
        )
        .expect("vanilla river palette defines")
    }

    /// Vanilla fade window, from the verbatim 00_graphics.lua:993-994 lines.
    fn vanilla_river_fade_defines() -> RiverFadeDefines {
        RiverFadeDefines::from_graphics_source(
            r#"
	RIVER_FADE_FROM = 20.0, -- the last river endings got faded out, X distance from the ending...
	RIVER_FADE_TO = 3.0,
"#,
        )
        .expect("vanilla river fade defines")
    }

    #[test]
    fn gate_river_palette_defines_boundaries() {
        // Item gate: the small/large/river-at-all boundaries land exactly on
        // the shipped indices 0 / 6 / 11 (00_defines.lua:936-938).
        let palette = vanilla_river_palette_defines();
        assert_eq!(
            (
                palette.small_start_index,
                palette.small_stop_index,
                palette.large_stop_index
            ),
            (0, 6, 11)
        );
        // River-at-all boundary: 0..=11 in, 12 out.
        assert!(palette.is_river_pixel(0), "index 0 is a river pixel");
        assert!(
            palette.is_river_pixel(11),
            "index 11 is the last river pixel"
        );
        assert!(!palette.is_river_pixel(12), "index 12 is not a river pixel");
        assert!(!palette.is_river_pixel(255), "sea/land fill is not a river");
        // Small/large split boundary: 6 small, 7 large.
        assert!(palette.is_small_river(0) && palette.is_small_river(6));
        assert!(!palette.is_small_river(7));
        assert!(palette.is_large_river(7) && palette.is_large_river(11));
        assert!(!palette.is_large_river(6) && !palette.is_large_river(12));
        // Every index is exactly one of small/large/not-river.
        for value in 0u8..=20 {
            let states = [palette.is_small_river(value), palette.is_large_river(value)];
            assert_eq!(
                states.iter().filter(|s| **s).count() == 1,
                palette.is_river_pixel(value),
                "index {value} must be exactly one class iff it is a river pixel"
            );
        }
        // Malformed orderings fail closed.
        assert!(RiverPaletteDefines::from_defines_source(
            "RIVER_SMALL_START_INDEX = 7,\nRIVER_SMALL_STOP_INDEX = 6,\nRIVER_LARGE_STOP_INDEX = 11,"
        )
        .is_err());
        assert!(RiverPaletteDefines::from_defines_source(
            "RIVER_SMALL_START_INDEX = 0,\nRIVER_SMALL_STOP_INDEX = 11,\nRIVER_LARGE_STOP_INDEX = 11,"
        )
        .is_err());
    }

    #[test]
    fn gate_river_fade_defines_span_matches_retired_fitted_literal() {
        // RIVER_FADE_FROM − RIVER_FADE_TO = 20.0 − 3.0 = 17.0 — bit-exactly
        // the retired capture-fitted RIVER_SOURCE_FADE_STEPS literal, so the
        // defines-driven fade span is a byte-noop for vanilla synthesis.
        let fade = vanilla_river_fade_defines();
        assert_eq!(fade.fade_from, 20.0);
        assert_eq!(fade.fade_to, 3.0);
        assert_eq!(fade.fade_span_stations().to_bits(), 17.0f32.to_bits());
        // Inconsistent windows fail closed.
        assert!(RiverFadeDefines::from_graphics_source(
            "RIVER_FADE_FROM = 3.0,\nRIVER_FADE_TO = 20.0,"
        )
        .is_err());
    }

    #[test]
    fn gate_vanilla_defines_files_carry_the_river_palette_and_fade() {
        // End-to-end resolver-shaped proof against the real shipped files.
        let game_dir =
            std::path::PathBuf::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV");
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla river defines gate: install absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let palette = RiverPaletteDefines::from_resolver(&resolver).expect("palette defines");
        assert_eq!(
            (
                palette.small_start_index,
                palette.small_stop_index,
                palette.large_stop_index
            ),
            (0, 6, 11),
            "shipped 00_defines.lua river indices drifted"
        );
        let fade = RiverFadeDefines::from_resolver(&resolver).expect("fade defines");
        assert_eq!(fade.fade_span_stations().to_bits(), 17.0f32.to_bits());
        println!("vanilla river defines gate: palette 0/6/11, fade span 17.0 (files)");
    }
}
