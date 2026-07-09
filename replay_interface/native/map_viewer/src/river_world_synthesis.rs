//! World-wide river mesh generator from `map/rivers.bmp` + `map/heightmap.bmp`
//! (resolver-routed), with a quantitative comparison against the CAPTURED
//! world river vertex buffer (rivers pass, slot-0 VB, 180,756 verts,
//! stride 32).
//!
//! # Measured captured-mesh laws (live_PAN2 / live_B_full / live_R_rivers —
//! all three ship a byte-identical VB, sha1 590a2fe9...)
//!
//! * Pair strip: vertices come in (side 0, side 1) pairs; 90,378 pairs ~=
//!   90,353 river pixels (palette 0..=11) in rivers.bmp — one station per
//!   river pixel. 581 strips split by the merged law (centerline step > 8.0),
//!   station resets to 0 per strip and increments by EXACTLY 1.0 per station
//!   (chain index, not arc length).
//! * 317 strips start on a class-0 (source) pixel = every source pixel; the
//!   other 264 start mid-class at full width/fade (downstream continuations
//!   after junctions).
//! * [ENGINE-LAW] The tracer graph is strictly 4-CONNECTED and only reaches
//!   pixels through the source-fed network: the captured VB misses exactly
//!   the 12 four-connected river components (234 px) that contain no
//!   class-0 pixel — 7 of them are 8-connected to sourced networks, which
//!   pins the connectivity to 4. (Proven against live_PAN2, 2026-07-04.)
//! * Steady width by palette class k (median over all pairs past the ramp):
//!   `width(k) = 1.11 + 0.17 * (k - 3)` — verified for k in
//!   {3,4,6,7,9,10,11} (classes 5 and 8 are unused by the vanilla map).
//!   NOTE this replaces the older provisional table in
//!   river_mesh_synthesis.rs (which was estimated from a single class-3 run).
//! * Source ramp: width = min(0.1 * station, width(class)); fade = 0 at
//!   stations 0..=1 then ~linear to 1.0 by station ~13 (mean slope 0.092 per
//!   station; exact engine fade is smoothed and varies per strip).
//! * Centerlines are SMOOTHED pixel-center chains. The engine filter
//!   accumulates float32 error (fractions like 63/197 observed), so the
//!   exact algorithm is not recoverable offline; a 2-pass in-place
//!   neighbour-relaxation (alpha 0.4, endpoints pinned) reproduces the
//!   captured strip-0 centerline to max 0.39 px. This residual is an
//!   acknowledged [ENGINE-LAW] gap; the generator therefore stays a
//!   comparison/report tool + default-off substitute candidate and is NOT
//!   wired into any default render path.
//!
//! Mod compatibility: every game-file read goes through [`GameFileResolver`]
//! (MOD_COMPATIBILITY.md, the one rule).

use crate::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const RIVERS_BMP_SOURCE: &str = "map/rivers.bmp";
pub const HEIGHTMAP_BMP_SOURCE: &str = "map/heightmap.bmp";
const RIVERS_PASS_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
/// Merged captured-index law: a centerline step above this splits strips.
const RIVER_STRIP_BREAK_DISTANCE: f32 = 8.0;
/// `RIVER_LARGE_STOP_INDEX = 11` (common/defines/00_defines.lua:938) — the last
/// rivers.bmp palette index that is a river pixel (small 0..=6, large 7..=11
/// per 00_defines.lua:936-938; see `RiverPaletteDefines`).
const RIVER_MAX_CLASS: u8 = 11;
/// Measured: classes 0..=2 are source/flow markers with no steady width; the
/// captured-VB width law starts at class 3. NOT a shipped define.
const RIVER_FIRST_WIDTH_CLASS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverWorldSynthesisParams {
    /// In-place neighbour-relaxation passes over each traced chain.
    pub smoothing_passes: u32,
    /// Relaxation strength toward the neighbour midpoint (0 = none).
    pub smoothing_alpha: f32,
    /// Source-strip width ramp per station (measured 0.1 exactly).
    pub width_ramp_per_station: f32,
    /// Source-strip fade: clamp((station - fade_delay) / fade_ramp_stations).
    pub fade_delay_stations: f32,
    pub fade_ramp_stations: f32,
}

impl Default for RiverWorldSynthesisParams {
    fn default() -> Self {
        Self {
            smoothing_passes: 2,
            smoothing_alpha: 0.4,
            width_ramp_per_station: 0.1,
            fade_delay_stations: 1.0,
            fade_ramp_stations: 10.9,
        }
    }
}

/// [CAPTURE] Measured steady-state full width (world units) per rivers.bmp
/// palette class, from the captured world river VB. Law `width(k) = 1.11 +
/// 0.17*(k-3)` was fit to and verified against the captured medians for k in
/// {3,4,6,7,9,10,11} (`gate_river_world_comparison_against_captured_sessions`
/// keeps every delta < 0.02). Classes 5 and 8 are unused by the vanilla map,
/// so their widths here are INTERPOLATIONS inside the same measured linear law
/// (each bracketed by measured neighbours 4/6 and 7/9) — not independently
/// observed. Classes 0..=2 (source/flow markers, no steady width) and >=12
/// (not a river) return `None` — an uncited width is never fabricated.
///
/// The class->width map is not in any shipped file: the river shader passes
/// vertex POSITION through unchanged (`gfx/FX/river.shader:198`), so width is
/// baked into the VB by engine CPU mesh-generation. The only file-grounded
/// class distinction is the small/large split (`common/defines/00_defines.lua`
/// :936-938). RE target to promote this from CAPTURE to FILE: the hoi4.exe
/// routine that builds the stride-32 river VB from rivers.bmp — its per-class
/// half-width lookup is the half-distance between the paired side-0/side-1
/// POSITION.xz at each steady-state station.
pub fn river_world_width_for_class(class: u8) -> Option<f32> {
    if (RIVER_FIRST_WIDTH_CLASS..=RIVER_MAX_CLASS).contains(&class) {
        Some(1.11 + 0.17 * (class - RIVER_FIRST_WIDTH_CLASS) as f32)
    } else {
        None
    }
}

/// Render-topology bucket for a rivers.bmp palette index. The small/large/
/// not-river partition is FILE-grounded (`RiverPaletteDefines`,
/// `common/defines/00_defines.lua:936-938`); the source/flow-in/flow-out
/// meaning of indices 0/1/2 is finer than the shipped files (index 0 = source
/// is CAPTURE-confirmed — every class-0 pixel is a strip start in the captured
/// world VB; indices 1/2 = flow-in/flow-out is HOI4-wiki convention, UNKNOWN
/// in shipped files).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiverPixelKind {
    /// Palette index 0 — river source. [CAPTURE] strip-start; also the FILE
    /// small-range start (`RIVER_SMALL_START_INDEX = 0`).
    Source,
    /// Palette index 1 — flow-in marker. [wiki/UNKNOWN in files] inside the
    /// FILE small range 0..=6.
    FlowIn,
    /// Palette index 2 — flow-out marker. [wiki/UNKNOWN in files] inside the
    /// FILE small range 0..=6.
    FlowOut,
    /// Small river channel: `3 ..= RIVER_SMALL_STOP_INDEX` (FILE small minus
    /// the 0..=2 markers).
    SmallChannel,
    /// Large river channel: `RIVER_SMALL_STOP_INDEX+1 ..= RIVER_LARGE_STOP_INDEX`
    /// (FILE large, 7..=11).
    LargeChannel,
    /// Not a river: palette index `> RIVER_LARGE_STOP_INDEX` (>=12); vanilla
    /// uses 254 (land) and 255 (sea).
    NotRiver,
}

/// Classify a rivers.bmp palette index through the FILE-defined split
/// (`RiverPaletteDefines`, 00_defines.lua:936-938) plus the render-topology
/// markers for indices 0/1/2. Total over all u8 — every pixel lands in exactly
/// one kind (asserted by `gate_river_pixel_classification_is_total`).
pub fn classify_river_pixel(value: u8, palette: RiverPaletteDefines) -> RiverPixelKind {
    if !palette.is_river_pixel(value) {
        return RiverPixelKind::NotRiver;
    }
    match value {
        0 => RiverPixelKind::Source,
        1 => RiverPixelKind::FlowIn,
        2 => RiverPixelKind::FlowOut,
        v if palette.is_small_river(v) => RiverPixelKind::SmallChannel,
        _ => RiverPixelKind::LargeChannel,
    }
}

/// Full-map extraction/classification stats over a `rivers.bmp` index image,
/// keyed by the FILE-defined palette split. Used by the vanilla extraction
/// gate: every palette index is bucketed and `unclassified_pixels` must be 0.
#[derive(Clone, Debug, PartialEq)]
pub struct RiverExtractionStats {
    pub total_pixels: usize,
    /// Non-zero palette-index counts (compact).
    pub palette_histogram: BTreeMap<u8, usize>,
    pub river_pixels: usize,
    pub not_river_pixels: usize,
    /// Pixels in no bucket. Definitionally 0 (classify is total); gated.
    pub unclassified_pixels: usize,
    pub source_pixels: usize,
    pub flow_in_pixels: usize,
    pub flow_out_pixels: usize,
    pub small_channel_pixels: usize,
    pub large_channel_pixels: usize,
    /// FILE small range `0..=RIVER_SMALL_STOP_INDEX` total (00_defines.lua:936-937).
    pub file_small_pixels: usize,
    /// FILE large range `RIVER_SMALL_STOP_INDEX+1..=RIVER_LARGE_STOP_INDEX`
    /// total (00_defines.lua:937-938).
    pub file_large_pixels: usize,
    /// Width classes (3..=11) actually present in the map.
    pub present_width_classes: Vec<u8>,
    /// Width classes (3..=11) absent from the map (width via CAPTURE-law
    /// interpolation, unobserved here).
    pub absent_width_classes: Vec<u8>,
}

/// Classify every `rivers.bmp` pixel through the FILE palette split.
pub fn river_extraction_stats(
    rivers: &IndexImage,
    palette: RiverPaletteDefines,
) -> RiverExtractionStats {
    let mut palette_histogram: BTreeMap<u8, usize> = BTreeMap::new();
    let mut source_pixels = 0usize;
    let mut flow_in_pixels = 0usize;
    let mut flow_out_pixels = 0usize;
    let mut small_channel_pixels = 0usize;
    let mut large_channel_pixels = 0usize;
    let mut not_river_pixels = 0usize;
    for &value in &rivers.pixels {
        *palette_histogram.entry(value).or_insert(0) += 1;
        match classify_river_pixel(value, palette) {
            RiverPixelKind::Source => source_pixels += 1,
            RiverPixelKind::FlowIn => flow_in_pixels += 1,
            RiverPixelKind::FlowOut => flow_out_pixels += 1,
            RiverPixelKind::SmallChannel => small_channel_pixels += 1,
            RiverPixelKind::LargeChannel => large_channel_pixels += 1,
            RiverPixelKind::NotRiver => not_river_pixels += 1,
        }
    }
    palette_histogram.retain(|_, count| *count > 0);
    let total_pixels = rivers.pixels.len();
    let river_pixels = source_pixels
        + flow_in_pixels
        + flow_out_pixels
        + small_channel_pixels
        + large_channel_pixels;
    let unclassified_pixels = total_pixels - river_pixels - not_river_pixels;
    let file_small_pixels = (palette.small_start_index..=palette.small_stop_index)
        .map(|index| palette_histogram.get(&index).copied().unwrap_or(0))
        .sum();
    let file_large_pixels = (palette.small_stop_index.saturating_add(1)..=palette.large_stop_index)
        .map(|index| palette_histogram.get(&index).copied().unwrap_or(0))
        .sum();
    let mut present_width_classes = Vec::new();
    let mut absent_width_classes = Vec::new();
    for class in RIVER_FIRST_WIDTH_CLASS..=RIVER_MAX_CLASS {
        if palette_histogram.get(&class).copied().unwrap_or(0) > 0 {
            present_width_classes.push(class);
        } else {
            absent_width_classes.push(class);
        }
    }
    RiverExtractionStats {
        total_pixels,
        palette_histogram,
        river_pixels,
        not_river_pixels,
        unclassified_pixels,
        source_pixels,
        flow_in_pixels,
        flow_out_pixels,
        small_channel_pixels,
        large_channel_pixels,
        file_small_pixels,
        file_large_pixels,
        present_width_classes,
        absent_width_classes,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverWorldVertex {
    pub position: [f32; 3],
    pub fade: f32,
    pub side: f32,
    pub station: f32,
    pub tangent: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct RiverWorldStrip {
    /// Pair index range into the vertex list (vertices 2*start..2*end).
    pub pair_start: usize,
    pub pair_end: usize,
    pub starts_at_source: bool,
}

#[derive(Clone, Debug)]
pub struct RiverWorldMesh {
    pub vertices: Vec<RiverWorldVertex>,
    pub strips: Vec<RiverWorldStrip>,
    pub map_width: usize,
    pub map_height: usize,
    pub river_pixel_count: usize,
    pub source_pixel_count: usize,
    pub class_histogram: BTreeMap<u8, usize>,
    /// River pixels in 4-connected components with no class-0 source —
    /// dropped, matching the engine (vanilla: 234 across 12 components).
    pub skipped_sourceless_pixels: usize,
}

impl RiverWorldMesh {
    pub fn pair_count(&self) -> usize {
        self.vertices.len() / 2
    }

    /// Serialize in the captured stride-32 layout (two float4s:
    /// pos.xyz+fade, side/station/tangent.xz).
    pub fn to_captured_layout_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.vertices.len() * 32);
        for vertex in &self.vertices {
            for value in [
                vertex.position[0],
                vertex.position[1],
                vertex.position[2],
                vertex.fade,
                vertex.side,
                vertex.station,
                vertex.tangent[0],
                vertex.tangent[1],
            ] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }
}

pub fn rivers_bmp_from_resolver(resolver: &GameFileResolver) -> Result<IndexImage> {
    let path = resolver.resolve_required(RIVERS_BMP_SOURCE, "rivers.bmp")?;
    load_bmp_indexed(&path)
}

pub fn heightmap_bmp_from_resolver(resolver: &GameFileResolver) -> Result<IndexImage> {
    let path = resolver.resolve_required(HEIGHTMAP_BMP_SOURCE, "heightmap.bmp")?;
    load_bmp_indexed(&path)
}

pub fn synthesize_river_world_mesh_from_resolver(
    resolver: &GameFileResolver,
    params: &RiverWorldSynthesisParams,
) -> Result<RiverWorldMesh> {
    let rivers = rivers_bmp_from_resolver(resolver)?;
    let heightmap = heightmap_bmp_from_resolver(resolver)?;
    synthesize_river_world_mesh(&rivers, &heightmap, params)
}

fn is_world_river_pixel(value: u8) -> bool {
    value <= RIVER_MAX_CLASS
}

pub fn synthesize_river_world_mesh(
    rivers: &IndexImage,
    heightmap: &IndexImage,
    params: &RiverWorldSynthesisParams,
) -> Result<RiverWorldMesh> {
    if rivers.width != heightmap.width || rivers.height != heightmap.height {
        return Err(format!(
            "river world synthesis blocked: rivers.bmp is {}x{} but heightmap.bmp is {}x{}",
            rivers.width, rivers.height, heightmap.width, heightmap.height
        )
        .into());
    }
    if params.smoothing_alpha < 0.0 || params.smoothing_alpha > 1.0 {
        return Err(format!(
            "river world synthesis blocked: smoothing alpha {} outside 0..=1",
            params.smoothing_alpha
        )
        .into());
    }

    let width = rivers.width;
    let height = rivers.height;
    let mut class_histogram = BTreeMap::new();
    let mut sources = Vec::new();
    let mut river_pixel_count = 0usize;
    for (index, value) in rivers.pixels.iter().enumerate() {
        if is_world_river_pixel(*value) {
            river_pixel_count += 1;
            *class_histogram.entry(*value).or_insert(0) += 1;
            if *value == 0 {
                sources.push(index);
            }
        }
    }

    let mut visited = vec![false; width * height];
    let mut mesh = RiverWorldMesh {
        vertices: Vec::with_capacity(river_pixel_count * 2),
        strips: Vec::new(),
        map_width: width,
        map_height: height,
        river_pixel_count,
        source_pixel_count: sources.len(),
        class_histogram,
        skipped_sourceless_pixels: 0,
    };

    // Pass 1: every source pixel starts a strip (matches capture: all 317
    // class-0 pixels are strip starts).
    for &source in &sources {
        if !visited[source] {
            let chain = trace_chain(rivers, &mut visited, source);
            append_strip(&mut mesh, rivers, heightmap, params, &chain, true);
        }
    }
    // Pass 2 [ENGINE-LAW, proven 2026-07-04]: the engine's tracer is strictly
    // 4-CONNECTED and reaches pixels only through the source-fed network.
    // Continuation strips seed from unvisited pixels 4-adjacent to visited
    // ones, iterated to a fixpoint. River pixels in 4-connected components
    // with no class-0 source are dropped by the engine (vanilla: exactly 12
    // components / 234 px missing from the captured world VB, including 7
    // components that are 8-connected to sourced networks — which proves the
    // 4-connectivity).
    loop {
        let mut seeded = false;
        for index in 0..rivers.pixels.len() {
            if !is_world_river_pixel(rivers.pixels[index]) || visited[index] {
                continue;
            }
            let adjacent_to_visited = axis_neighbors(index, width, height)
                .into_iter()
                .flatten()
                .any(|neighbor| visited[neighbor]);
            if adjacent_to_visited {
                let chain = trace_chain(rivers, &mut visited, index);
                append_strip(&mut mesh, rivers, heightmap, params, &chain, false);
                seeded = true;
            }
        }
        if !seeded {
            break;
        }
    }
    mesh.skipped_sourceless_pixels = (0..rivers.pixels.len())
        .filter(|&index| is_world_river_pixel(rivers.pixels[index]) && !visited[index])
        .count();
    Ok(mesh)
}

fn axis_neighbors(index: usize, width: usize, height: usize) -> [Option<usize>; 4] {
    let x = index % width;
    let y = index / width;
    [
        (x + 1 < width).then(|| index + 1),
        (x > 0).then(|| index - 1),
        (y + 1 < height).then(|| index + width),
        (y > 0).then(|| index - width),
    ]
}

/// Walk from `start` consuming unvisited river pixels. Strictly 4-connected
/// (engine law), preferring straight continuation over turns (the captured
/// station count matches the traced pixel count, so the engine walks pixel
/// by pixel; the exact junction tie-break is unknown).
fn trace_chain(rivers: &IndexImage, visited: &mut [bool], start: usize) -> Vec<usize> {
    let width = rivers.width as isize;
    let height = rivers.height as isize;
    let mut chain = vec![start];
    visited[start] = true;
    let mut current = start;
    let mut direction: Option<(isize, isize)> = None;
    loop {
        let cx = (current % rivers.width) as isize;
        let cy = (current / rivers.width) as isize;
        let mut best: Option<((isize, isize), i32)> = None;
        for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
            let nx = cx + dx;
            let ny = cy + dy;
            if nx < 0 || ny < 0 || nx >= width || ny >= height {
                continue;
            }
            let index = (ny * width + nx) as usize;
            if visited[index] || !is_world_river_pixel(rivers.pixels[index]) {
                continue;
            }
            // Prefer the straightest continuation.
            let straightness = match direction {
                Some((px, py)) => -((px * dx + py * dy) as i32),
                None => 0,
            };
            if best.map_or(true, |(_, best_cost)| straightness < best_cost) {
                best = Some(((dx, dy), straightness));
            }
        }
        let Some(((dx, dy), _)) = best else {
            break;
        };
        let next = ((cy + dy) * width + cx + dx) as usize;
        visited[next] = true;
        chain.push(next);
        direction = Some((dx, dy));
        current = next;
    }
    chain
}

fn append_strip(
    mesh: &mut RiverWorldMesh,
    rivers: &IndexImage,
    heightmap: &IndexImage,
    params: &RiverWorldSynthesisParams,
    chain: &[usize],
    starts_at_source: bool,
) {
    if chain.len() < 2 {
        return;
    }
    let height = rivers.height;
    // Pixel centers in world space (world_z flips bmp y).
    let mut centers: Vec<[f32; 2]> = chain
        .iter()
        .map(|&index| {
            let x = (index % rivers.width) as f32;
            let y = (index / rivers.width) as f32;
            [x, (height - 1) as f32 - y]
        })
        .collect();
    smooth_chain(&mut centers, params);

    let pair_start = mesh.vertices.len() / 2;
    for (station_index, center) in centers.iter().enumerate() {
        let class = rivers.pixels[chain[station_index]];
        let base_width = river_world_width_for_class(class)
            .or_else(|| nearest_chain_class_width(rivers, chain, station_index))
            .unwrap_or(1.11);
        let station = station_index as f32;
        let (width_value, fade) = if starts_at_source {
            (
                (params.width_ramp_per_station * station).min(base_width),
                ((station - params.fade_delay_stations) / params.fade_ramp_stations)
                    .clamp(0.0, 1.0),
            )
        } else {
            (base_width, 1.0)
        };
        let tangent = chain_tangent(&centers, station_index);
        let normal = [-tangent[1], tangent[0]];
        let half = width_value * 0.5;
        for (side, sign) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
            let x = center[0] + normal[0] * half * sign;
            let z = center[1] + normal[1] * half * sign;
            let y = sample_height_bilinear(heightmap, x, z) * 0.1;
            mesh.vertices.push(RiverWorldVertex {
                position: [x, y, z],
                fade,
                side,
                station,
                tangent,
            });
        }
    }
    mesh.strips.push(RiverWorldStrip {
        pair_start,
        pair_end: mesh.vertices.len() / 2,
        starts_at_source,
    });
}

fn smooth_chain(centers: &mut [[f32; 2]], params: &RiverWorldSynthesisParams) {
    if centers.len() < 3 {
        return;
    }
    let alpha = params.smoothing_alpha;
    for _ in 0..params.smoothing_passes {
        for i in 1..centers.len() - 1 {
            let mid_x = (centers[i - 1][0] + centers[i + 1][0]) * 0.5;
            let mid_z = (centers[i - 1][1] + centers[i + 1][1]) * 0.5;
            centers[i][0] = centers[i][0] * (1.0 - alpha) + mid_x * alpha;
            centers[i][1] = centers[i][1] * (1.0 - alpha) + mid_z * alpha;
        }
    }
}

fn chain_tangent(centers: &[[f32; 2]], index: usize) -> [f32; 2] {
    let prev = if index == 0 { index } else { index - 1 };
    let next = if index + 1 >= centers.len() {
        index
    } else {
        index + 1
    };
    let dx = centers[next][0] - centers[prev][0];
    let dz = centers[next][1] - centers[prev][1];
    let len = (dx * dx + dz * dz).sqrt();
    if len <= f32::EPSILON {
        [0.0, 1.0]
    } else {
        [dx / len, dz / len]
    }
}

fn nearest_chain_class_width(rivers: &IndexImage, chain: &[usize], index: usize) -> Option<f32> {
    for distance in 1..chain.len() {
        for candidate in [index.checked_sub(distance), index.checked_add(distance)]
            .into_iter()
            .flatten()
        {
            if candidate < chain.len() {
                if let Some(width) = river_world_width_for_class(rivers.pixels[chain[candidate]]) {
                    return Some(width);
                }
            }
        }
    }
    None
}

fn sample_height_bilinear(heightmap: &IndexImage, x: f32, world_z: f32) -> f32 {
    let bmp_y = (heightmap.height - 1) as f32 - world_z;
    let x0 = (x.floor().max(0.0) as usize).min(heightmap.width - 1);
    let y0 = (bmp_y.floor().max(0.0) as usize).min(heightmap.height - 1);
    let x1 = (x0 + 1).min(heightmap.width - 1);
    let y1 = (y0 + 1).min(heightmap.height - 1);
    let fx = (x - x0 as f32).clamp(0.0, 1.0);
    let fy = (bmp_y - y0 as f32).clamp(0.0, 1.0);
    let sample = |px: usize, py: usize| heightmap.pixels[py * heightmap.width + px] as f32;
    let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
    let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

// ---------------------------------------------------------------------------
// Captured-VB comparison
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct RiverDistanceStats {
    pub count: usize,
    pub mean: f32,
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub max: f32,
    pub within_half_px: f32,
    pub within_one_px: f32,
}

#[derive(Clone, Debug)]
pub struct RiverWidthClassRow {
    pub class: u8,
    pub captured_pairs: usize,
    pub captured_median_width: f32,
    pub generated_width: f32,
    pub delta: f32,
}

#[derive(Clone, Debug)]
pub struct RiverWorldComparisonReport {
    pub capture_dir: String,
    pub captured_pair_count: usize,
    pub captured_zero_padded_pairs: usize,
    pub captured_strip_count: usize,
    pub generated_pair_count: usize,
    pub generated_strip_count: usize,
    pub river_pixel_count: usize,
    pub captured_to_generated: RiverDistanceStats,
    pub generated_to_captured: RiverDistanceStats,
    pub width_rows: Vec<RiverWidthClassRow>,
    pub captured_height_error_mean: f32,
    pub captured_height_error_max: f32,
}

impl RiverWorldComparisonReport {
    pub fn print(&self) {
        println!("river world synthesis vs capture: {}", self.capture_dir);
        println!(
            "  pairs: captured {} ({} zero-padded excluded) vs generated {} (river pixels {}); strips: captured {} vs generated {}",
            self.captured_pair_count,
            self.captured_zero_padded_pairs,
            self.generated_pair_count,
            self.river_pixel_count,
            self.captured_strip_count,
            self.generated_strip_count
        );
        for (label, stats) in [
            ("captured->generated", &self.captured_to_generated),
            ("generated->captured", &self.generated_to_captured),
        ] {
            println!(
                "  {label}: n={} mean={:.4} p50={:.4} p95={:.4} p99={:.4} max={:.3} <=0.5px {:.2}% <=1px {:.2}%",
                stats.count,
                stats.mean,
                stats.p50,
                stats.p95,
                stats.p99,
                stats.max,
                stats.within_half_px * 100.0,
                stats.within_one_px * 100.0
            );
        }
        println!("  width by class (captured median steady vs generated law):");
        for row in &self.width_rows {
            println!(
                "    class {:2}: captured {:.4} (n={}) generated {:.4} delta {:+.4}",
                row.class,
                row.captured_median_width,
                row.captured_pairs,
                row.generated_width,
                row.delta
            );
        }
        println!(
            "  captured y vs 0.1*bilinear(heightmap): mean err {:.4} max err {:.4}",
            self.captured_height_error_mean, self.captured_height_error_max
        );
    }
}

/// Decoded captured world river VB (pair midpoints + per-pair width).
pub struct CapturedRiverWorld {
    pub midpoints: Vec<[f32; 3]>,
    pub widths: Vec<f32>,
    pub stations: Vec<f32>,
    pub strip_count: usize,
    /// All-zero pairs excluded from the stats (capture pool-buffer padding).
    pub zero_padded_pairs: usize,
}

pub fn load_captured_river_world(capture_dir: &Path) -> Result<CapturedRiverWorld> {
    let bundle_path = capture_dir.join(RIVERS_PASS_BUNDLE_FILE);
    let bundle: MapFrameRuntimeBundle = serde_json::from_slice(
        &fs::read(&bundle_path).map_err(|err| format!("read {}: {err}", bundle_path.display()))?,
    )
    .map_err(|err| format!("parse {}: {err}", bundle_path.display()))?;
    let binding = bundle
        .active_draw
        .vertex_buffers
        .get("0")
        .ok_or("rivers pass bundle has no IA slot-0 vertex buffer")?;
    if binding.stride != Some(RIVER_VERTEX_STRIDE_BYTES) {
        return Err(format!(
            "captured river VB stride {:?}, expected {RIVER_VERTEX_STRIDE_BYTES}",
            binding.stride
        )
        .into());
    }
    let file = binding
        .file
        .as_ref()
        .ok_or("captured river VB has no payload file")?;
    let bytes =
        fs::read(capture_dir.join(file)).map_err(|err| format!("read river VB {}: {err}", file))?;
    if bytes.len() % RIVER_VERTEX_STRIDE_BYTES as usize != 0 {
        return Err(format!(
            "captured river VB byte length {} not divisible by stride",
            bytes.len()
        )
        .into());
    }
    let vertex_count = bytes.len() / RIVER_VERTEX_STRIDE_BYTES as usize;
    if vertex_count % 2 != 0 {
        return Err(format!("captured river VB has odd vertex count {vertex_count}").into());
    }
    let read_f32 = |offset: usize| -> Result<f32> {
        let raw = bytes
            .get(offset..offset + 4)
            .ok_or("captured river VB truncated")?;
        Ok(f32::from_le_bytes(raw.try_into()?))
    };
    let mut midpoints = Vec::with_capacity(vertex_count / 2);
    let mut widths = Vec::with_capacity(vertex_count / 2);
    let mut stations = Vec::with_capacity(vertex_count / 2);
    let mut strip_count = 0usize;
    let mut zero_padded_pairs = 0usize;
    let mut previous: Option<[f32; 3]> = None;
    for pair in 0..vertex_count / 2 {
        let a = pair * 2 * RIVER_VERTEX_STRIDE_BYTES as usize;
        let b = a + RIVER_VERTEX_STRIDE_BYTES as usize;
        let left = [read_f32(a)?, read_f32(a + 4)?, read_f32(a + 8)?];
        let right = [read_f32(b)?, read_f32(b + 4)?, read_f32(b + 8)?];
        // Captured pool buffers under-record and are zero padded (the known
        // +16B pad law); all-zero pairs are padding, not geometry.
        if left == [0.0; 3] && right == [0.0; 3] && read_f32(a + 16)? == 0.0 {
            zero_padded_pairs += 1;
            continue;
        }
        let mid = [
            (left[0] + right[0]) * 0.5,
            (left[1] + right[1]) * 0.5,
            (left[2] + right[2]) * 0.5,
        ];
        let width = ((left[0] - right[0]).powi(2) + (left[2] - right[2]).powi(2)).sqrt();
        match previous {
            Some(prev) => {
                let step = ((left[0] - prev[0]).powi(2)
                    + (left[1] - prev[1]).powi(2)
                    + (left[2] - prev[2]).powi(2))
                .sqrt();
                if step > RIVER_STRIP_BREAK_DISTANCE {
                    strip_count += 1;
                }
            }
            None => strip_count += 1,
        }
        previous = Some(left);
        stations.push(read_f32(a + 20)?);
        midpoints.push(mid);
        widths.push(width);
    }
    Ok(CapturedRiverWorld {
        midpoints,
        widths,
        stations,
        strip_count,
        zero_padded_pairs,
    })
}

struct PointGrid {
    cell: f32,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
}

impl PointGrid {
    fn build(points: &[[f32; 2]], cell: f32) -> Self {
        let mut cells: BTreeMap<(i64, i64), Vec<usize>> = BTreeMap::new();
        for (index, p) in points.iter().enumerate() {
            cells
                .entry(((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64))
                .or_default()
                .push(index);
        }
        Self { cell, cells }
    }

    fn nearest_distance(&self, points: &[[f32; 2]], query: [f32; 2]) -> f32 {
        let cx = (query[0] / self.cell).floor() as i64;
        let cz = (query[1] / self.cell).floor() as i64;
        let mut best = f32::INFINITY;
        // Expand rings until the best hit provably beats anything in an
        // unvisited ring (points can be arbitrarily sparse near map edges).
        let max_ring = 4096i64;
        for ring in 0..=max_ring {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    if let Some(indices) = self.cells.get(&(cx + dx, cz + dz)) {
                        for &index in indices {
                            let p = points[index];
                            let d = ((p[0] - query[0]).powi(2) + (p[1] - query[1]).powi(2)).sqrt();
                            best = best.min(d);
                        }
                    }
                }
            }
            if best < ring as f32 * self.cell {
                break;
            }
        }
        best
    }
}

fn distance_stats(distances: &mut [f32]) -> RiverDistanceStats {
    if distances.is_empty() {
        return RiverDistanceStats::default();
    }
    distances.sort_by(|a, b| a.partial_cmp(b).expect("finite distances"));
    let count = distances.len();
    let pick = |q: f64| distances[((count - 1) as f64 * q) as usize];
    RiverDistanceStats {
        count,
        mean: distances.iter().sum::<f32>() / count as f32,
        p50: pick(0.50),
        p95: pick(0.95),
        p99: pick(0.99),
        max: distances[count - 1],
        within_half_px: distances.iter().filter(|d| **d <= 0.5).count() as f32 / count as f32,
        within_one_px: distances.iter().filter(|d| **d <= 1.0).count() as f32 / count as f32,
    }
}

pub fn compare_river_world_mesh_to_capture(
    mesh: &RiverWorldMesh,
    rivers: &IndexImage,
    heightmap: &IndexImage,
    capture_dir: &Path,
) -> Result<RiverWorldComparisonReport> {
    let captured = load_captured_river_world(capture_dir)?;
    let generated_midpoints: Vec<[f32; 2]> = (0..mesh.pair_count())
        .map(|pair| {
            let a = mesh.vertices[pair * 2].position;
            let b = mesh.vertices[pair * 2 + 1].position;
            [(a[0] + b[0]) * 0.5, (a[2] + b[2]) * 0.5]
        })
        .collect();
    let captured_midpoints: Vec<[f32; 2]> =
        captured.midpoints.iter().map(|m| [m[0], m[2]]).collect();

    let generated_grid = PointGrid::build(&generated_midpoints, 4.0);
    let captured_grid = PointGrid::build(&captured_midpoints, 4.0);
    let mut c_to_g: Vec<f32> = captured_midpoints
        .iter()
        .map(|p| generated_grid.nearest_distance(&generated_midpoints, *p))
        .collect();
    let mut g_to_c: Vec<f32> = generated_midpoints
        .iter()
        .map(|p| captured_grid.nearest_distance(&captured_midpoints, *p))
        .collect();

    // Width by class: captured pairs past the source ramp, keyed by the
    // rivers.bmp class under the midpoint.
    let mut widths_by_class: BTreeMap<u8, Vec<f32>> = BTreeMap::new();
    let mut height_err_sum = 0f64;
    let mut height_err_max = 0f32;
    for (pair, mid) in captured.midpoints.iter().enumerate() {
        let expected_y = sample_height_bilinear(heightmap, mid[0], mid[2]) * 0.1;
        let err = (mid[1] - expected_y).abs();
        height_err_sum += err as f64;
        height_err_max = height_err_max.max(err);
        if captured.stations[pair] <= 26.0 {
            continue; // widest class ramp (2.47 / 0.1 per station) still rising
        }
        let bx = mid[0].round();
        let bz = mid[2].round();
        if bx < 0.0 || bz < 0.0 || bx >= rivers.width as f32 || bz >= rivers.height as f32 {
            continue;
        }
        let by = rivers.height - 1 - bz as usize;
        let class = rivers.pixels[by * rivers.width + bx as usize];
        if river_world_width_for_class(class).is_some() {
            widths_by_class
                .entry(class)
                .or_default()
                .push(captured.widths[pair]);
        }
    }
    let width_rows = widths_by_class
        .into_iter()
        .map(|(class, mut widths)| {
            widths.sort_by(|a, b| a.partial_cmp(b).expect("finite widths"));
            let median = widths[widths.len() / 2];
            let generated =
                river_world_width_for_class(class).expect("class filtered to width classes");
            RiverWidthClassRow {
                class,
                captured_pairs: widths.len(),
                captured_median_width: median,
                generated_width: generated,
                delta: generated - median,
            }
        })
        .collect();

    Ok(RiverWorldComparisonReport {
        capture_dir: capture_dir.display().to_string(),
        captured_pair_count: captured.midpoints.len(),
        captured_zero_padded_pairs: captured.zero_padded_pairs,
        captured_strip_count: captured.strip_count,
        generated_pair_count: mesh.pair_count(),
        generated_strip_count: mesh.strips.len(),
        river_pixel_count: mesh.river_pixel_count,
        captured_to_generated: distance_stats(&mut c_to_g),
        generated_to_captured: distance_stats(&mut g_to_c),
        width_rows,
        captured_height_error_mean: (height_err_sum / captured.midpoints.len().max(1) as f64)
            as f32,
        captured_height_error_max: height_err_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_bmp8(path: &Path, width: usize, height: usize, pixels: &[u8]) {
        assert_eq!(pixels.len(), width * height);
        let row_stride = (width + 3) & !3;
        let palette_bytes = 256usize * 4;
        let pixel_offset = 14 + 40 + palette_bytes;
        let file_size = pixel_offset + row_stride * height;
        fs::create_dir_all(path.parent().expect("bmp parent")).expect("create bmp dir");
        let mut file = fs::File::create(path).expect("create bmp");
        file.write_all(b"BM").unwrap();
        file.write_all(&(file_size as u32).to_le_bytes()).unwrap();
        file.write_all(&[0u8; 4]).unwrap();
        file.write_all(&(pixel_offset as u32).to_le_bytes())
            .unwrap();
        file.write_all(&40u32.to_le_bytes()).unwrap();
        file.write_all(&(width as i32).to_le_bytes()).unwrap();
        file.write_all(&(height as i32).to_le_bytes()).unwrap();
        file.write_all(&1u16.to_le_bytes()).unwrap();
        file.write_all(&8u16.to_le_bytes()).unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap();
        file.write_all(&((row_stride * height) as u32).to_le_bytes())
            .unwrap();
        file.write_all(&2835u32.to_le_bytes()).unwrap();
        file.write_all(&2835u32.to_le_bytes()).unwrap();
        file.write_all(&256u32.to_le_bytes()).unwrap();
        file.write_all(&256u32.to_le_bytes()).unwrap();
        for i in 0..=255u8 {
            file.write_all(&[i, i, i, 0]).unwrap();
        }
        // bottom-up rows
        for y in (0..height).rev() {
            let mut row = pixels[y * width..(y + 1) * width].to_vec();
            row.resize(row_stride, 0);
            file.write_all(&row).unwrap();
        }
    }

    fn straight_river_pixels(width: usize, height: usize) -> Vec<u8> {
        // Source at (2,3), then class-3 run to the east.
        let mut pixels = vec![255u8; width * height];
        pixels[3 * width + 2] = 0;
        for x in 3..width - 2 {
            pixels[3 * width + x] = 3;
        }
        pixels
    }

    fn write_river_fixture(root: &Path, width: usize, height: usize, pixels: &[u8]) {
        write_bmp8(&root.join("map").join("rivers.bmp"), width, height, pixels);
        write_bmp8(
            &root.join("map").join("heightmap.bmp"),
            width,
            height,
            &vec![100u8; width * height],
        );
    }

    #[test]
    fn straight_river_synthesizes_pair_strip_with_measured_laws() {
        let rivers = IndexImage {
            width: 32,
            height: 8,
            pixels: straight_river_pixels(32, 8),
        };
        let heightmap = IndexImage {
            width: 32,
            height: 8,
            pixels: vec![100u8; 32 * 8],
        };
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("mesh");
        assert_eq!(mesh.strips.len(), 1);
        assert!(mesh.strips[0].starts_at_source);
        assert_eq!(mesh.pair_count(), mesh.river_pixel_count);
        // Station = chain index.
        assert_eq!(mesh.vertices[0].station, 0.0);
        assert_eq!(mesh.vertices[2].station, 1.0);
        // Source ramp: width 0.1 per station.
        let pair_width = |pair: usize| {
            let a = mesh.vertices[pair * 2].position;
            let b = mesh.vertices[pair * 2 + 1].position;
            ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        assert!((pair_width(0) - 0.0).abs() < 1e-4);
        assert!((pair_width(1) - 0.1).abs() < 1e-4);
        assert!((pair_width(5) - 0.5).abs() < 1e-4);
        // Steady class-3 width 1.11.
        assert!((pair_width(20) - 1.11).abs() < 1e-4);
        // Fade 0 at stations 0..=1, 1.0 past the ramp.
        assert_eq!(mesh.vertices[0].fade, 0.0);
        assert_eq!(mesh.vertices[2].fade, 0.0);
        assert_eq!(mesh.vertices[40].fade, 1.0);
        // Height: flat heightmap 100 -> y = 10 everywhere.
        assert!((mesh.vertices[10].position[1] - 10.0).abs() < 1e-4);
        // Sides 0/1 per pair.
        assert_eq!(mesh.vertices[0].side, 0.0);
        assert_eq!(mesh.vertices[1].side, 1.0);
    }

    #[test]
    fn width_law_matches_measured_class_table() {
        // Measured medians from the captured world VB (live_PAN2).
        let expected = [
            (3u8, 1.11f32),
            (4, 1.28),
            (6, 1.62),
            (7, 1.79),
            (9, 2.13),
            (10, 2.30),
            (11, 2.47),
        ];
        for (class, width) in expected {
            let generated = river_world_width_for_class(class).expect("width class");
            assert!(
                (generated - width).abs() < 5e-3,
                "class {class}: law {generated} vs measured {width}"
            );
        }
        // Classes 5 and 8 are absent from the vanilla map: the law interpolates
        // them inside the measured linear range (bracketed by 4/6 and 7/9), so
        // they still resolve to Some — never fabricated, never out of range.
        let w5 = river_world_width_for_class(5).expect("interpolated class 5");
        let w8 = river_world_width_for_class(8).expect("interpolated class 8");
        assert!(
            river_world_width_for_class(4).unwrap() < w5
                && w5 < river_world_width_for_class(6).unwrap(),
            "class 5 width must sit between measured 4 and 6"
        );
        assert!(
            river_world_width_for_class(7).unwrap() < w8
                && w8 < river_world_width_for_class(9).unwrap(),
            "class 8 width must sit between measured 7 and 9"
        );
        // Source/flow markers and non-river indices have no steady width.
        assert_eq!(river_world_width_for_class(0), None);
        assert_eq!(river_world_width_for_class(1), None);
        assert_eq!(river_world_width_for_class(2), None);
        assert_eq!(river_world_width_for_class(12), None);
        assert_eq!(river_world_width_for_class(254), None);
        assert_eq!(river_world_width_for_class(255), None);
    }

    fn vanilla_palette_defines() -> RiverPaletteDefines {
        RiverPaletteDefines::from_defines_source(
            "RIVER_SMALL_START_INDEX = 0,\nRIVER_SMALL_STOP_INDEX = 6,\nRIVER_LARGE_STOP_INDEX = 11,",
        )
        .expect("vanilla palette defines")
    }

    #[test]
    fn gate_river_pixel_classification_is_total() {
        // Every u8 lands in exactly one kind, and the kind agrees with the FILE
        // palette predicates for river-or-not.
        let palette = vanilla_palette_defines();
        for value in 0u8..=255 {
            let kind = classify_river_pixel(value, palette);
            match kind {
                RiverPixelKind::NotRiver => {
                    assert!(!palette.is_river_pixel(value), "index {value} misbucketed")
                }
                _ => assert!(palette.is_river_pixel(value), "index {value} misbucketed"),
            }
        }
        assert_eq!(classify_river_pixel(0, palette), RiverPixelKind::Source);
        assert_eq!(classify_river_pixel(1, palette), RiverPixelKind::FlowIn);
        assert_eq!(classify_river_pixel(2, palette), RiverPixelKind::FlowOut);
        assert_eq!(
            classify_river_pixel(3, palette),
            RiverPixelKind::SmallChannel
        );
        assert_eq!(
            classify_river_pixel(6, palette),
            RiverPixelKind::SmallChannel
        );
        assert_eq!(
            classify_river_pixel(7, palette),
            RiverPixelKind::LargeChannel
        );
        assert_eq!(
            classify_river_pixel(11, palette),
            RiverPixelKind::LargeChannel
        );
        assert_eq!(classify_river_pixel(12, palette), RiverPixelKind::NotRiver);
        assert_eq!(classify_river_pixel(254, palette), RiverPixelKind::NotRiver);
        assert_eq!(classify_river_pixel(255, palette), RiverPixelKind::NotRiver);
    }

    #[test]
    fn extraction_stats_reconcile_on_synthetic_map() {
        // 4x4: one source(0), one flow-in(1), two class-3, rest not-river(254).
        let pixels = vec![
            0, 1, 3, 254, //
            3, 254, 254, 254, //
            254, 254, 254, 254, //
            254, 254, 254, 255,
        ];
        let rivers = IndexImage {
            width: 4,
            height: 4,
            pixels,
        };
        let stats = river_extraction_stats(&rivers, vanilla_palette_defines());
        assert_eq!(stats.total_pixels, 16);
        assert_eq!(stats.unclassified_pixels, 0);
        assert_eq!(stats.river_pixels, 4);
        assert_eq!(stats.not_river_pixels, 12);
        assert_eq!(stats.source_pixels, 1);
        assert_eq!(stats.flow_in_pixels, 1);
        assert_eq!(stats.flow_out_pixels, 0);
        assert_eq!(stats.small_channel_pixels, 2);
        assert_eq!(stats.large_channel_pixels, 0);
        assert_eq!(stats.file_small_pixels, 4); // 0,1,3,3
        assert_eq!(stats.file_large_pixels, 0);
        assert_eq!(stats.present_width_classes, vec![3]);
        assert_eq!(stats.absent_width_classes, vec![4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(
            stats.river_pixels + stats.not_river_pixels + stats.unclassified_pixels,
            stats.total_pixels
        );
    }

    /// Full-map extraction/classification gate against the real shipped vanilla
    /// `rivers.bmp`. Runs offline (no capture needed); skipped if the install
    /// is absent. Proves: every one of the 5632x2048 pixels is classified with
    /// zero unclassified, the exact per-index histogram, the FILE small/large
    /// split totals, which width classes are present vs absent, and that the
    /// synthesized world mesh turns every reachable river pixel into one
    /// station (pairs + engine-dropped sourceless pixels == river pixels).
    #[test]
    fn gate_full_map_river_extraction_vanilla() {
        let game_dir = PathBuf::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV");
        if !game_dir.is_dir() {
            eprintln!("skipping full-map river extraction gate: vanilla install absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let rivers = rivers_bmp_from_resolver(&resolver).expect("rivers.bmp");
        let palette = RiverPaletteDefines::from_resolver(&resolver).expect("palette defines");
        let stats = river_extraction_stats(&rivers, palette);

        assert_eq!(stats.total_pixels, 5632 * 2048);
        assert_eq!(stats.total_pixels, 11_534_336);
        assert_eq!(
            stats.unclassified_pixels, 0,
            "every rivers.bmp pixel must be classified"
        );
        assert_eq!(
            stats.river_pixels + stats.not_river_pixels,
            stats.total_pixels
        );

        // Exact vanilla per-index histogram of the shipped rivers.bmp.
        let expected: &[(u8, usize)] = &[
            (0, 317),
            (1, 250),
            (2, 14),
            (3, 35_104),
            (4, 10_985),
            (6, 13_144),
            (7, 1_154),
            (9, 1_677),
            (10, 43),
            (11, 27_665),
            (254, 7_479_319),
            (255, 3_964_664),
        ];
        for (index, count) in expected {
            assert_eq!(
                stats.palette_histogram.get(index).copied().unwrap_or(0),
                *count,
                "rivers.bmp index {index} count drifted"
            );
        }
        assert_eq!(
            stats.palette_histogram.len(),
            expected.len(),
            "rivers.bmp has unexpected extra palette indices: {:?}",
            stats.palette_histogram
        );

        assert_eq!(stats.river_pixels, 90_353);
        assert_eq!(stats.source_pixels, 317);
        assert_eq!(stats.flow_in_pixels, 250);
        assert_eq!(stats.flow_out_pixels, 14);
        // FILE small/large split (00_defines.lua:936-938).
        assert_eq!(
            stats.file_small_pixels,
            317 + 250 + 14 + 35_104 + 10_985 + 13_144
        );
        assert_eq!(stats.file_large_pixels, 1_154 + 1_677 + 43 + 27_665);

        // Width classes present vs absent; every present class has a CAPTURE
        // width, and no non-channel index resolves to a width.
        assert_eq!(stats.present_width_classes, vec![3, 4, 6, 7, 9, 10, 11]);
        assert_eq!(stats.absent_width_classes, vec![5, 8]);
        for class in &stats.present_width_classes {
            assert!(
                river_world_width_for_class(*class).is_some(),
                "present class {class} must have a CAPTURE width"
            );
        }
        for index in [0u8, 1, 2, 12, 254, 255] {
            assert!(
                river_world_width_for_class(index).is_none(),
                "index {index} must not resolve to a width"
            );
        }

        // Segments per class from the synthesized world mesh (default params).
        let heightmap = heightmap_bmp_from_resolver(&resolver).expect("heightmap.bmp");
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("world mesh");
        assert_eq!(mesh.river_pixel_count, 90_353);
        assert_eq!(mesh.source_pixel_count, 317);
        // Every river pixel is one of: a station/pair in a strip, a singleton
        // chain (traced but too short for geometry: length < 2, no pair), or a
        // sourceless 4-connected pixel dropped by the engine (CAPTURE law).
        // Pairs + skipped can never exceed the total, and the singleton
        // remainder must be small.
        let reached_pairs = mesh.pair_count();
        assert!(
            reached_pairs + mesh.skipped_sourceless_pixels <= 90_353,
            "pairs {reached_pairs} + skipped {} exceed river pixels",
            mesh.skipped_sourceless_pixels
        );
        let singleton_pixels = 90_353 - reached_pairs - mesh.skipped_sourceless_pixels;
        assert!(
            singleton_pixels < 1_000 && mesh.skipped_sourceless_pixels < 1_000,
            "unexpected reconciliation: singletons {singleton_pixels}, skipped {}",
            mesh.skipped_sourceless_pixels
        );
        let mut per_class_pairs: BTreeMap<u8, usize> = BTreeMap::new();
        for strip in &mesh.strips {
            for pair in strip.pair_start..strip.pair_end {
                // Class under the pair midpoint (round to nearest pixel).
                let a = mesh.vertices[pair * 2].position;
                let b = mesh.vertices[pair * 2 + 1].position;
                let bx = ((a[0] + b[0]) * 0.5).round();
                let bz = ((a[2] + b[2]) * 0.5).round();
                if bx < 0.0 || bz < 0.0 || bx >= rivers.width as f32 || bz >= rivers.height as f32 {
                    continue;
                }
                let by = rivers.height - 1 - bz as usize;
                let class = rivers.pixels[by * rivers.width + bx as usize];
                *per_class_pairs.entry(class).or_insert(0) += 1;
            }
        }
        println!(
            "full-map river extraction: {} river px ({} not-river), {} strips, {} reached pairs, \
             {} singleton px, {} skipped sourceless; classes present {:?} absent(interp) {:?}; \
             FILE small {} / large {}; per-class pairs {:?}",
            stats.river_pixels,
            stats.not_river_pixels,
            mesh.strips.len(),
            reached_pairs,
            singleton_pixels,
            mesh.skipped_sourceless_pixels,
            stats.present_width_classes,
            stats.absent_width_classes,
            stats.file_small_pixels,
            stats.file_large_pixels,
            per_class_pairs
        );
    }

    #[test]
    fn river_world_from_resolver_uses_mod_override() {
        let root = temp_test_dir("river_world_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_river_fixture(&vanilla, 32, 8, &straight_river_pixels(32, 8));
        // Mod moves the river one row down.
        let mut modded = vec![255u8; 32 * 8];
        modded[4 * 32 + 2] = 0;
        for x in 3..30 {
            modded[4 * 32 + x] = 3;
        }
        write_river_fixture(&mod_root, 32, 8, &modded);

        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let mesh = synthesize_river_world_mesh_from_resolver(
            &resolver,
            &RiverWorldSynthesisParams::default(),
        )
        .expect("modded mesh");
        // Modded river row 4 -> world z = height-1-4 = 3.
        let mid_z = (mesh.vertices[0].position[2] + mesh.vertices[1].position[2]) * 0.5;
        assert!(
            (mid_z - 3.0).abs() < 1e-4,
            "mod override must win, z {mid_z}"
        );
        assert_eq!(resolver.stats().overridden_file_count, 2);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn river_world_resolver_honors_replace_path_exclusion() {
        let root = temp_test_dir("river_world_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_river_fixture(&vanilla, 32, 8, &straight_river_pixels(32, 8));
        fs::create_dir_all(&mod_root).expect("mod root");
        fs::write(
            mod_root.join("descriptor.mod"),
            "name=\"ReplaceMap\"\nreplace_path=\"map\"\n",
        )
        .expect("descriptor");

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let err = synthesize_river_world_mesh_from_resolver(
            &resolver,
            &RiverWorldSynthesisParams::default(),
        )
        .expect_err("replace_path removes vanilla rivers.bmp");
        assert!(err.to_string().contains("rivers.bmp"), "{err}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn captured_layout_bytes_round_trip_stride32() {
        let rivers = IndexImage {
            width: 32,
            height: 8,
            pixels: straight_river_pixels(32, 8),
        };
        let heightmap = IndexImage {
            width: 32,
            height: 8,
            pixels: vec![100u8; 32 * 8],
        };
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("mesh");
        let bytes = mesh.to_captured_layout_bytes();
        assert_eq!(bytes.len(), mesh.vertices.len() * 32);
        let x = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let side1 = f32::from_le_bytes(bytes[32 + 16..32 + 20].try_into().unwrap());
        assert_eq!(x, mesh.vertices[0].position[0]);
        assert_eq!(side1, 1.0);
    }

    /// Full comparison gate against real captured sessions. Skipped unless
    /// RIVER_CAPTURE_DIRS (semicolon-separated capture dirs) is set and the
    /// vanilla install is present.
    #[test]
    fn gate_river_world_comparison_against_captured_sessions() {
        let Ok(dirs) = std::env::var("RIVER_CAPTURE_DIRS") else {
            eprintln!("skipping river world capture gate: RIVER_CAPTURE_DIRS unset");
            return;
        };
        let game_dir = PathBuf::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV");
        if !game_dir.is_dir() {
            eprintln!("skipping river world capture gate: vanilla install absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let rivers = rivers_bmp_from_resolver(&resolver).expect("rivers.bmp");
        let heightmap = heightmap_bmp_from_resolver(&resolver).expect("heightmap.bmp");
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("world mesh");
        for dir in dirs.split(';').filter(|dir| !dir.is_empty()) {
            let report =
                compare_river_world_mesh_to_capture(&mesh, &rivers, &heightmap, Path::new(dir))
                    .expect(dir);
            report.print();
            assert!(
                report.captured_to_generated.within_one_px > 0.98,
                "{dir}: captured->generated 1px coverage {}",
                report.captured_to_generated.within_one_px
            );
            assert!(
                report.generated_to_captured.within_one_px > 0.98,
                "{dir}: generated->captured 1px coverage {}",
                report.generated_to_captured.within_one_px
            );
            for row in &report.width_rows {
                assert!(
                    row.delta.abs() < 0.02,
                    "{dir}: class {} width delta {}",
                    row.class,
                    row.delta
                );
            }
        }
    }
}
