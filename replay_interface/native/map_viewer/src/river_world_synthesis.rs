
use crate::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const RIVERS_BMP_SOURCE: &str = "map/rivers.bmp";
pub const HEIGHTMAP_BMP_SOURCE: &str = "map/heightmap.bmp";
const RIVERS_PASS_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
const RIVER_STRIP_BREAK_DISTANCE: f32 = 8.0;
const RIVER_MAX_CLASS: u8 = 11;
const RIVER_FIRST_WIDTH_CLASS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RiverWorldSynthesisParams {
    pub fade_from: f32,
    pub fade_to: f32,
}

impl Default for RiverWorldSynthesisParams {
    fn default() -> Self {
        Self {
            fade_from: 20.0,
            fade_to: 3.0,
        }
    }
}

pub fn river_world_width_for_class(class: u8) -> Option<f32> {
    (3..100).contains(&class).then(|| 0.6 + 0.17 * class as f32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiverPixelKind {
    Source,
    FlowIn,
    FlowOut,
    SmallChannel,
    LargeChannel,
    NotRiver,
}

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

#[derive(Clone, Debug, PartialEq)]
pub struct RiverExtractionStats {
    pub total_pixels: usize,
    pub palette_histogram: BTreeMap<u8, usize>,
    pub river_pixels: usize,
    pub not_river_pixels: usize,
    pub unclassified_pixels: usize,
    pub source_pixels: usize,
    pub flow_in_pixels: usize,
    pub flow_out_pixels: usize,
    pub small_channel_pixels: usize,
    pub large_channel_pixels: usize,
    pub file_small_pixels: usize,
    pub file_large_pixels: usize,
    pub present_width_classes: Vec<u8>,
    pub absent_width_classes: Vec<u8>,
}

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
    pub pair_start: usize,
    pub pair_end: usize,
    pub starts_at_source: bool,
    pub render_group: u8,
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
    pub skipped_sourceless_pixels: usize,
}

impl RiverWorldMesh {
    pub fn pair_count(&self) -> usize {
        self.vertices.len() / 2
    }

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
    decode_bmp_indexed(&resolver.read_resolved_bytes(&path)?, &path)
}

pub fn heightmap_bmp_from_resolver(resolver: &GameFileResolver) -> Result<IndexImage> {
    let path = resolver.resolve_required(HEIGHTMAP_BMP_SOURCE, "heightmap.bmp")?;
    decode_bmp_indexed(&resolver.read_resolved_bytes(&path)?, &path)
}

pub fn synthesize_river_world_mesh_from_resolver(
    resolver: &GameFileResolver,
    _params: &RiverWorldSynthesisParams,
) -> Result<RiverWorldMesh> {
    let rivers = rivers_bmp_from_resolver(resolver)?;
    let heightmap = heightmap_bmp_from_resolver(resolver)?;
    synthesize_river_world_mesh_with_defines(
        &rivers,
        &heightmap,
        RiverPaletteDefines::from_resolver(resolver)?,
        RiverFadeDefines::from_resolver(resolver)?,
    )
}

fn is_world_channel(value: u8) -> bool {
    (3..100).contains(&value)
}

pub fn synthesize_river_world_mesh(
    rivers: &IndexImage,
    heightmap: &IndexImage,
    params: &RiverWorldSynthesisParams,
) -> Result<RiverWorldMesh> {
    synthesize_river_world_mesh_with_defines(
        rivers,
        heightmap,
        RiverPaletteDefines {
            small_start_index: 0,
            small_stop_index: 6,
            large_stop_index: 11,
        },
        RiverFadeDefines {
            fade_from: params.fade_from,
            fade_to: params.fade_to,
        },
    )
}

pub fn synthesize_river_world_mesh_with_defines(
    rivers: &IndexImage,
    heightmap: &IndexImage,
    _palette: RiverPaletteDefines,
    fade: RiverFadeDefines,
) -> Result<RiverWorldMesh> {
    if rivers.width != heightmap.width || rivers.height != heightmap.height {
        return Err(format!(
            "river world synthesis blocked: rivers.bmp is {}x{} but heightmap.bmp is {}x{}",
            rivers.width, rivers.height, heightmap.width, heightmap.height
        )
        .into());
    }
    if !(fade.fade_from.is_finite()
        && fade.fade_to.is_finite()
        && fade.fade_from > fade.fade_to
        && fade.fade_to >= 0.0)
    {
        return Err("river world synthesis received invalid effective fade defines".into());
    }

    let width = rivers.width;
    let height = rivers.height;
    let mut class_histogram = BTreeMap::new();
    let mut sources = Vec::new();
    let mut markers = Vec::new();
    let mut river_pixel_count = 0usize;
    for z in 0..height {
        for x in 0..width {
            let index = decoded_index_from_world(x, z, width, height);
            let value = rivers.pixels[index];
            if value < 100 {
                river_pixel_count += 1;
                *class_histogram.entry(value).or_insert(0) += 1;
            }
            match value {
                0 => sources.push(index),
                1 | 2 => markers.push((value, index)),
                _ => {}
            }
        }
    }
    markers.sort_by_key(|(value, _)| *value);

    let mut visited = vec![false; width * height];
    let mut marker_processed = vec![false; markers.len()];
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

    for &source in &sources {
        let mut chain = vec![source];
        trace_unvisited_channels(rivers, &mut visited, source, &mut chain);
        append_strip(&mut mesh, rivers, heightmap, fade, &chain, true, 0)?;
    }
    let mut marker_wave_base = 0u8;
    loop {
        let ready = markers
            .iter()
            .enumerate()
            .filter_map(|(marker_index, &(marker_value, marker))| {
                if marker_processed[marker_index] {
                    return None;
                }
                engine_axis_neighbors(marker, width, height)
                    .iter()
                    .flatten()
                    .copied()
                    .find(|&index| visited[index] && is_world_channel(rivers.pixels[index]))
                    .map(|previous| (marker_index, marker_value, marker, previous))
            })
            .collect::<Vec<_>>();
        if ready.is_empty() {
            break;
        }
        for (marker_index, marker_value, marker, previous) in ready {
            marker_processed[marker_index] = true;
            let mut chain = vec![previous, marker];
            trace_unvisited_channels(rivers, &mut visited, marker, &mut chain);
            let render_group = marker_wave_base
                .checked_add(marker_value)
                .ok_or("river render-group byte overflow")?;
            append_strip(
                &mut mesh,
                rivers,
                heightmap,
                fade,
                &chain,
                false,
                render_group,
            )?;
        }
        marker_wave_base = marker_wave_base
            .checked_add(2)
            .ok_or("river render-group byte overflow")?;
    }
    mesh.skipped_sourceless_pixels = rivers
        .pixels
        .iter()
        .enumerate()
        .filter(|(index, value)| is_world_channel(**value) && !visited[*index])
        .count();
    Ok(mesh)
}

fn decoded_index_from_world(x: usize, z: usize, width: usize, height: usize) -> usize {
    (height - 1 - z) * width + x
}

fn world_coordinates(index: usize, width: usize, height: usize) -> (usize, usize) {
    (index % width, height - 1 - index / width)
}

fn engine_axis_neighbors(index: usize, width: usize, height: usize) -> [Option<usize>; 4] {
    let (x, z) = world_coordinates(index, width, height);
    [
        (x > 0).then(|| decoded_index_from_world(x - 1, z, width, height)),
        (x + 1 < width).then(|| decoded_index_from_world(x + 1, z, width, height)),
        (z + 1 < height).then(|| decoded_index_from_world(x, z + 1, width, height)),
        (z > 0).then(|| decoded_index_from_world(x, z - 1, width, height)),
    ]
}

fn trace_unvisited_channels(
    rivers: &IndexImage,
    visited: &mut [bool],
    start: usize,
    chain: &mut Vec<usize>,
) {
    let mut current = start;
    loop {
        let Some(next) = engine_axis_neighbors(current, rivers.width, rivers.height)
            .iter()
            .flatten()
            .copied()
            .find(|&index| !visited[index] && is_world_channel(rivers.pixels[index]))
        else {
            break;
        };
        visited[next] = true;
        chain.push(next);
        current = next;
    }
}

fn append_strip(
    mesh: &mut RiverWorldMesh,
    rivers: &IndexImage,
    heightmap: &IndexImage,
    fade_defines: RiverFadeDefines,
    chain: &[usize],
    starts_at_source: bool,
    render_group: u8,
) -> Result<()> {
    if chain.len() < 3 {
        return Ok(());
    }
    let height = rivers.height;
    let mut centers: Vec<[f32; 2]> = chain
        .iter()
        .map(|&index| {
            let x = (index % rivers.width) as f32;
            let y = (index / rivers.width) as f32;
            [x, (height - 1) as f32 - y]
        })
        .collect();
    smooth_chain(&mut centers);
    let mut engine_distances = Vec::with_capacity(centers.len() - 1);

    let pair_start = mesh.vertices.len() / 2;
    let mut last_tangent = [0.0f32; 2];
    let mut last_half = 0.0f32;
    for station_index in 0..centers.len() - 1 {
        let center = centers[station_index];
        let prev = station_index.saturating_sub(1);
        let next = station_index + 1;
        let dx = centers[next][0] - centers[prev][0];
        let dz = centers[next][1] - centers[prev][1];
        let distance = (dx * dx + dz * dz).sqrt();
        if !distance.is_finite() || distance <= 0.0 {
            return Err(format!(
                "exact river station {station_index} has a degenerate engine tangent"
            )
            .into());
        }
        let tangent = [dx / distance, dz / distance];
        engine_distances.push(distance);
        let class = rivers.pixels[chain[station_index]];
        let base_half_width = 0.3 + 0.085 * class as f32;
        let station = station_index as f32;
        let taper_station = if starts_at_source {
            station
        } else {
            (centers.len() - 2 - station_index) as f32
        };
        let half = base_half_width.min(0.05 * taper_station);
        let normal = [-tangent[1], tangent[0]];
        for (side, sign) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
            let x = center[0] + normal[0] * half * sign;
            let z = center[1] + normal[1] * half * sign;
            let y = sample_height_bilinear(heightmap, x, z) * 0.1;
            mesh.vertices.push(RiverWorldVertex {
                position: [x, y, z],
                fade: 0.0,
                side,
                station,
                tangent,
            });
        }
        last_tangent = tangent;
        last_half = half;
    }
    let total_distance = engine_distances
        .iter()
        .fold(0.0f64, |sum, distance| sum + f64::from(*distance));
    let fade_from = f64::from(fade_defines.fade_from);
    let fade_to = f64::from(fade_defines.fade_to);
    let fade_span = fade_from - fade_to;
    let fade_margin = (total_distance - fade_from).max(0.0);
    let mut accumulated = 0.0f64;
    for (station_index, distance) in engine_distances.iter().enumerate() {
        accumulated += f64::from(*distance);
        let unbounded = if starts_at_source {
            1.0 - ((total_distance - accumulated) - fade_margin) / fade_span
        } else {
            1.0 - (accumulated - fade_margin) / fade_span
        };
        let fade = unbounded.clamp(0.0, 1.0) as f32;
        mesh.vertices[(pair_start + station_index) * 2].fade = fade;
        mesh.vertices[(pair_start + station_index) * 2 + 1].fade = fade;
    }
    let endpoint_index = centers.len() - 1;
    let endpoint = centers[endpoint_index];
    let normal = [-last_tangent[1], last_tangent[0]];
    let endpoint_fade = if starts_at_source { 1.0 } else { 0.0 };
    for (side, sign) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
        let x = endpoint[0] + normal[0] * last_half * sign;
        let z = endpoint[1] + normal[1] * last_half * sign;
        let y = sample_height_bilinear(heightmap, x, z) * 0.1;
        mesh.vertices.push(RiverWorldVertex {
            position: [x, y, z],
            fade: endpoint_fade,
            side,
            station: endpoint_index as f32,
            tangent: last_tangent,
        });
    }
    mesh.strips.push(RiverWorldStrip {
        pair_start,
        pair_end: mesh.vertices.len() / 2,
        starts_at_source,
        render_group,
    });
    Ok(())
}

fn smooth_chain(centers: &mut [[f32; 2]]) {
    if centers.len() < 5 {
        return;
    }
    let source = centers.to_vec();
    for index in 2..centers.len() - 2 {
        centers[index][0] = source[index - 2..=index + 2]
            .iter()
            .map(|point| point[0])
            .sum::<f32>()
            / 5.0;
        centers[index][1] = source[index - 2..=index + 2]
            .iter()
            .map(|point| point[1])
            .sum::<f32>()
            / 5.0;
    }
}

fn sample_height_bilinear(heightmap: &IndexImage, x: f32, world_z: f32) -> f32 {
    let x0 = (x.trunc().max(0.0) as usize).min(heightmap.width.saturating_sub(2));
    let z0 = (world_z.trunc().max(0.0) as usize).min(heightmap.height.saturating_sub(2));
    let x1 = (x0 + 1).min(heightmap.width - 1);
    let z1 = (z0 + 1).min(heightmap.height - 1);
    let y0 = heightmap.height - 1 - z0;
    let y1 = heightmap.height - 1 - z1;
    let fx = (x - x0 as f32).clamp(0.0, 1.0);
    let fz = (world_z - z0 as f32).clamp(0.0, 1.0);
    let sample = |px: usize, py: usize| heightmap.pixels[py * heightmap.width + px] as f32;
    let top = sample(x0, y0) * (1.0 - fx) + sample(x1, y0) * fx;
    let bottom = sample(x0, y1) * (1.0 - fx) + sample(x1, y1) * fx;
    top * (1.0 - fz) + bottom * fz
}


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

#[derive(Debug)]
pub struct CapturedRiverWorld {
    pub midpoints: Vec<[f32; 3]>,
    pub widths: Vec<f32>,
    pub stations: Vec<f32>,
    pub strip_count: usize,
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
    parse_captured_river_vertex_bytes(&bytes)
}

fn parse_captured_river_vertex_bytes(bytes: &[u8]) -> Result<CapturedRiverWorld> {
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
        if left == [0.0; 3] && right == [0.0; 3] && read_f32(a + 16)? == 0.0 {
            zero_padded_pairs += 1;
            continue;
        }
        let station = read_f32(a + 20)?;
        let fields = [
            ("left.x", left[0]),
            ("left.y", left[1]),
            ("left.z", left[2]),
            ("right.x", right[0]),
            ("right.y", right[1]),
            ("right.z", right[2]),
            ("station", station),
        ];
        for (field, value) in fields {
            if !value.is_finite() {
                return Err(
                    format!("captured river VB pair {pair}: non-finite float in {field}").into(),
                );
            }
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
        stations.push(station);
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
    // time; generated midpoints derive from u8 bitmaps); total_cmp is
    distances.sort_by(f32::total_cmp);
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

    let mut widths_by_class: BTreeMap<u8, Vec<f32>> = BTreeMap::new();
    let mut height_err_sum = 0f64;
    let mut height_err_max = 0f32;
    for (pair, mid) in captured.midpoints.iter().enumerate() {
        let expected_y = sample_height_bilinear(heightmap, mid[0], mid[2]) * 0.1;
        let err = (mid[1] - expected_y).abs();
        height_err_sum += err as f64;
        height_err_max = height_err_max.max(err);
        if captured.stations[pair] <= 26.0 {
            continue;
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
            widths.sort_by(f32::total_cmp);
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

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

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
        for y in (0..height).rev() {
            let mut row = pixels[y * width..(y + 1) * width].to_vec();
            row.resize(row_stride, 0);
            file.write_all(&row).unwrap();
        }
    }

    fn straight_river_pixels(width: usize, height: usize) -> Vec<u8> {
        let mut pixels = vec![255u8; width * height];
        pixels[3 * width + 2] = 0;
        for x in 3..width - 2 {
            pixels[3 * width + x] = 3;
        }
        pixels
    }

    fn exact_strip_fixture(
        centers: &[(usize, usize, u8)],
        starts_at_source: bool,
        fade: RiverFadeDefines,
    ) -> RiverWorldMesh {
        let width = 8;
        let height = 8;
        let mut rivers = IndexImage {
            width,
            height,
            pixels: vec![255; width * height],
        };
        let chain = centers
            .iter()
            .map(|&(x, z, class)| {
                let index = decoded_index_from_world(x, z, width, height);
                rivers.pixels[index] = class;
                index
            })
            .collect::<Vec<_>>();
        let heightmap = IndexImage {
            width,
            height,
            pixels: vec![100; width * height],
        };
        let mut mesh = RiverWorldMesh {
            vertices: Vec::new(),
            strips: Vec::new(),
            map_width: width,
            map_height: height,
            river_pixel_count: chain.len(),
            source_pixel_count: usize::from(starts_at_source),
            class_histogram: BTreeMap::new(),
            skipped_sourceless_pixels: 0,
        };
        append_strip(
            &mut mesh,
            &rivers,
            &heightmap,
            fade,
            &chain,
            starts_at_source,
            0,
        )
        .unwrap();
        mesh
    }

    fn expected_pair(
        center: [f32; 2],
        half: f32,
        fade: f32,
        station: f32,
        tangent: [f32; 2],
    ) -> [RiverWorldVertex; 2] {
        let normal = [-tangent[1], tangent[0]];
        [-1.0f32, 1.0].map(|sign| {
            let x = center[0] + normal[0] * half * sign;
            let z = center[1] + normal[1] * half * sign;
            RiverWorldVertex {
                position: [x, 10.0, z],
                fade,
                side: if sign < 0.0 { 0.0 } else { 1.0 },
                station,
                tangent,
            }
        })
    }

    #[test]
    fn curved_source_strip_matches_every_exact_engine_field() {
        let mesh = exact_strip_fixture(
            &[(1, 1, 0), (2, 1, 3), (2, 2, 3), (3, 2, 9)],
            true,
            RiverFadeDefines {
                fade_from: 4.0,
                fade_to: 1.0,
            },
        );
        let root2 = 2.0f32.sqrt();
        let diagonal = [1.0 / root2, 1.0 / root2];
        let total = 1.0f64 + f64::from(root2) * 2.0;
        let fades = [
            (1.0 - (total - 1.0) / 3.0) as f32,
            (1.0 - (total - (1.0 + f64::from(root2))) / 3.0) as f32,
            1.0,
            1.0,
        ];
        let expected = [
            expected_pair([1.0, 1.0], 0.0, fades[0], 0.0, [1.0, 0.0]),
            expected_pair([2.0, 1.0], 0.05, fades[1], 1.0, diagonal),
            expected_pair([2.0, 2.0], 0.10, fades[2], 2.0, diagonal),
            expected_pair([3.0, 2.0], 0.10, fades[3], 3.0, diagonal),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        assert_eq!(mesh.vertices, expected);
    }

    #[test]
    fn short_marker_strip_reuses_penultimate_tangent_and_zero_width() {
        let mesh = exact_strip_fixture(
            &[(1, 1, 1), (2, 1, 3), (2, 2, 9)],
            false,
            RiverFadeDefines {
                fade_from: 4.0,
                fade_to: 1.0,
            },
        );
        let root2 = 2.0f32.sqrt();
        let diagonal = [1.0 / root2, 1.0 / root2];
        let fades = [
            (1.0 - 1.0 / 3.0) as f32,
            (1.0 - (1.0 + f64::from(root2)) / 3.0) as f32,
            0.0,
        ];
        let expected = [
            expected_pair([1.0, 1.0], 0.05, fades[0], 0.0, [1.0, 0.0]),
            expected_pair([2.0, 1.0], 0.0, fades[1], 1.0, diagonal),
            expected_pair([2.0, 2.0], 0.0, fades[2], 2.0, diagonal),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        assert_eq!(mesh.vertices, expected);
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
    fn straight_river_uses_exact_a729_geometry_producer() {
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
                .expect("exact producer");
        assert_eq!(mesh.strips.len(), 1);
        assert!(mesh.vertices.len() >= 4);
        assert_eq!(mesh.vertices[0].fade, 0.0);
        assert_eq!(mesh.vertices.last().unwrap().fade, 1.0);
    }

    #[test]
    fn width_law_matches_measured_class_table() {
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
        for (class, expected) in [(5, 1.45f32), (8, 1.96), (12, 2.64)] {
            assert!(
                (river_world_width_for_class(class).unwrap() - expected).abs() < 1e-5,
                "class {class}"
            );
        }
        for marker in 0..=2 {
            assert_eq!(river_world_width_for_class(marker), None);
        }
        assert_eq!(river_world_width_for_class(254), None);
        assert_eq!(river_world_width_for_class(255), None);
    }

    #[test]
    fn exact_marker_fixpoint_emits_endpoint_strip_after_source_strip() {
        let width = 10;
        let height = 4;
        let mut pixels = vec![255u8; width * height];
        let mut set_world = |x: usize, z: usize, value: u8| {
            pixels[(height - 1 - z) * width + x] = value;
        };
        set_world(1, 1, 0);
        set_world(2, 1, 3);
        set_world(3, 1, 3);
        set_world(4, 1, 1);
        set_world(5, 1, 5);
        set_world(6, 1, 8);
        let rivers = IndexImage {
            width,
            height,
            pixels,
        };
        let heightmap = IndexImage {
            width,
            height,
            pixels: vec![100; width * height],
        };
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("exact marker producer");
        assert_eq!(mesh.strips.len(), 2);
        assert!(mesh.strips[0].starts_at_source);
        assert_eq!(mesh.strips[0].render_group, 0);
        assert!(!mesh.strips[1].starts_at_source);
        assert_eq!(mesh.strips[1].render_group, 1);
        let second = &mesh.strips[1];
        let midpoint_x = |pair: usize| {
            (mesh.vertices[pair * 2].position[0] + mesh.vertices[pair * 2 + 1].position[0]) * 0.5
        };
        assert_eq!(midpoint_x(second.pair_start), 3.0);
        assert_eq!(midpoint_x(second.pair_start + 1), 4.0);
        assert_eq!(mesh.vertices[(second.pair_end - 1) * 2].fade, 0.0);
    }

    #[test]
    fn exact_tracer_accepts_3_through_99_independent_of_palette_stops() {
        let width = 8;
        let height = 4;
        let mut pixels = vec![255u8; width * height];
        let mut set_world = |x: usize, z: usize, value: u8| {
            pixels[(height - 1 - z) * width + x] = value;
        };
        set_world(1, 1, 0);
        set_world(2, 1, 3);
        set_world(3, 1, 99);
        set_world(4, 1, 100);
        let rivers = IndexImage {
            width,
            height,
            pixels,
        };
        let heightmap = IndexImage {
            width,
            height,
            pixels: vec![100; width * height],
        };
        let palette = RiverPaletteDefines {
            small_start_index: 0,
            small_stop_index: 3,
            large_stop_index: 4,
        };
        let mesh = synthesize_river_world_mesh_with_defines(
            &rivers,
            &heightmap,
            palette,
            RiverFadeDefines {
                fade_from: 20.0,
                fade_to: 3.0,
            },
        )
        .expect("exact tracer range");
        assert_eq!(mesh.river_pixel_count, 3);
        assert_eq!(mesh.strips.len(), 1);
        assert_eq!(mesh.strips[0].pair_end - mesh.strips[0].pair_start, 3);
        assert!(mesh.class_histogram.contains_key(&99));
        assert!(!mesh.class_histogram.contains_key(&100));
    }

    #[test]
    fn effective_fade_override_regenerates_vertex_payload() {
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
        let vanilla =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .unwrap();
        let changed = synthesize_river_world_mesh(
            &rivers,
            &heightmap,
            &RiverWorldSynthesisParams {
                fade_from: 10.0,
                fade_to: 1.0,
            },
        )
        .unwrap();
        assert_ne!(
            vanilla.to_captured_layout_bytes(),
            changed.to_captured_layout_bytes()
        );
    }

    fn vanilla_palette_defines() -> RiverPaletteDefines {
        crate::test_support::with_define_fixture(
            "NDefines = { NMilitary = { RIVER_SMALL_START_INDEX = 0, RIVER_SMALL_STOP_INDEX = 2 * 3, RIVER_LARGE_STOP_INDEX = 11 } }",
            RiverPaletteDefines::from_resolver,
        )
        .expect("vanilla palette defines")
    }

    #[test]
    fn gate_river_pixel_classification_is_total() {
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
        let pixels = vec![
            0, 1, 3, 254,
            3, 254, 254, 254,
            254, 254, 254, 254,
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
        assert_eq!(stats.file_small_pixels, 4);
        assert_eq!(stats.file_large_pixels, 0);
        assert_eq!(stats.present_width_classes, vec![3]);
        assert_eq!(stats.absent_width_classes, vec![4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(
            stats.river_pixels + stats.not_river_pixels + stats.unclassified_pixels,
            stats.total_pixels
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_full_map_river_extraction_vanilla() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
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
        assert_eq!(
            stats.file_small_pixels,
            317 + 250 + 14 + 35_104 + 10_985 + 13_144
        );
        assert_eq!(stats.file_large_pixels, 1_154 + 1_677 + 43 + 27_665);

        assert_eq!(stats.present_width_classes, vec![3, 4, 6, 7, 9, 10, 11]);
        assert_eq!(stats.absent_width_classes, vec![5, 8]);
        for class in &stats.present_width_classes {
            assert!(
                river_world_width_for_class(*class).is_some(),
                "present class {class} must have a CAPTURE width"
            );
        }
        let heightmap = heightmap_bmp_from_resolver(&resolver).expect("heightmap.bmp");
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("exact offline river geometry");
        assert_eq!(mesh.source_pixel_count, 317);
        assert!(!mesh.strips.is_empty());

    }

    #[test]
    fn river_world_from_resolver_uses_mod_override() {
        let root = temp_test_dir("river_world_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_river_fixture(&vanilla, 32, 8, &straight_river_pixels(32, 8));
        let mut modded = vec![255u8; 32 * 8];
        modded[4 * 32 + 2] = 0;
        for x in 3..30 {
            modded[4 * 32 + x] = 3;
        }
        write_river_fixture(&mod_root, 32, 8, &modded);

        let resolver = GameFileResolver::with_mod_paths(vanilla.clone(), vec![mod_root.clone()])
            .expect("resolver");
        fs::create_dir_all(vanilla.join("common").join("defines")).unwrap();
        fs::write(
            vanilla.join("common").join("defines").join("00_defines.lua"),
            "NDefines = { NMilitary = { RIVER_SMALL_START_INDEX = 0, RIVER_SMALL_STOP_INDEX = 6, RIVER_LARGE_STOP_INDEX = 11 } }",
        )
        .unwrap();
        fs::write(
            vanilla
                .join("common")
                .join("defines")
                .join("00_graphics.lua"),
            "NDefines.NGraphics = { RIVER_FADE_FROM = 20.0, RIVER_FADE_TO = 3.0 }",
        )
        .unwrap();
        let mesh = synthesize_river_world_mesh_from_resolver(
            &resolver,
            &RiverWorldSynthesisParams::default(),
        )
        .expect("mod-aware exact geometry");
        assert!(!mesh.strips.is_empty());
        assert!(resolver.stats().overridden_file_count >= 2);
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
        let mesh = RiverWorldMesh {
            vertices: vec![
                RiverWorldVertex {
                    position: [1.0, 2.0, 3.0],
                    fade: 0.25,
                    side: 0.0,
                    station: 4.0,
                    tangent: [0.5, -0.5],
                },
                RiverWorldVertex {
                    position: [2.0, 2.0, 3.0],
                    fade: 0.25,
                    side: 1.0,
                    station: 4.0,
                    tangent: [0.5, -0.5],
                },
            ],
            strips: vec![],
            map_width: 32,
            map_height: 8,
            river_pixel_count: 1,
            source_pixel_count: 0,
            class_histogram: BTreeMap::new(),
            skipped_sourceless_pixels: 0,
        };
        let bytes = mesh.to_captured_layout_bytes();
        assert_eq!(bytes.len(), mesh.vertices.len() * 32);
        let x = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let side1 = f32::from_le_bytes(bytes[32 + 16..32 + 20].try_into().unwrap());
        assert_eq!(x, mesh.vertices[0].position[0]);
        assert_eq!(side1, 1.0);
    }

    fn captured_vertex_bytes(position: [f32; 3], side: f32, station: f32) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (i, v) in position.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[12..16].copy_from_slice(&1.0f32.to_le_bytes());
        bytes[16..20].copy_from_slice(&side.to_le_bytes());
        bytes[20..24].copy_from_slice(&station.to_le_bytes());
        bytes
    }

    #[test]
    fn captured_vb_parse_accepts_finite_pairs_and_zero_padding() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&captured_vertex_bytes([1.0, 2.0, 3.0], 0.0, 5.0));
        bytes.extend_from_slice(&captured_vertex_bytes([1.5, 2.0, 3.5], 1.0, 5.0));
        bytes.extend_from_slice(&[0u8; 64]);
        let world = parse_captured_river_vertex_bytes(&bytes).expect("finite pairs parse");
        assert_eq!(world.midpoints.len(), 1);
        assert_eq!(world.zero_padded_pairs, 1);
        assert_eq!(world.stations, vec![5.0]);
    }

    #[test]
    fn captured_vb_parse_fails_closed_on_non_finite_coordinate() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&captured_vertex_bytes([1.0, 2.0, 3.0], 0.0, 5.0));
        bytes.extend_from_slice(&captured_vertex_bytes([1.5, f32::NAN, 3.5], 1.0, 5.0));
        let error = parse_captured_river_vertex_bytes(&bytes)
            .expect_err("NaN coordinate must be a typed refusal, not a panic");
        let text = error.to_string();
        assert!(text.contains("pair 0"), "{text}");
        assert!(text.contains("right.y"), "{text}");
    }

    #[test]
    fn captured_vb_parse_fails_closed_on_non_finite_station() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&captured_vertex_bytes([1.0, 2.0, 3.0], 0.0, 0.0));
        bytes.extend_from_slice(&captured_vertex_bytes([1.5, 2.0, 3.5], 1.0, 0.0));
        bytes.extend_from_slice(&captured_vertex_bytes([2.0, 2.0, 3.0], 0.0, f32::INFINITY));
        bytes.extend_from_slice(&captured_vertex_bytes([2.5, 2.0, 3.5], 1.0, 1.0));
        let error = parse_captured_river_vertex_bytes(&bytes)
            .expect_err("non-finite station must be a typed refusal");
        let text = error.to_string();
        assert!(text.contains("pair 1"), "{text}");
        assert!(text.contains("station"), "{text}");
    }

    #[test]
    #[cfg_attr(
        not(gate_river_captures_with_hoi4),
        ignore = "river captures (RIVER_CAPTURE_DIRS, else the preserved live_FINAL_full capture), plus the game install"
    )]
    fn gate_river_world_comparison_against_captured_sessions() {
        let dirs =
            corpus::river_capture_dirs().expect("the river capture gate was true at build time");
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let rivers = rivers_bmp_from_resolver(&resolver).expect("rivers.bmp");
        let heightmap = heightmap_bmp_from_resolver(&resolver).expect("heightmap.bmp");
        let mesh =
            synthesize_river_world_mesh(&rivers, &heightmap, &RiverWorldSynthesisParams::default())
                .expect("world mesh");
        for dir in dirs {
            let report = compare_river_world_mesh_to_capture(&mesh, &rivers, &heightmap, &dir)
                .unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
            report.print();
            assert!(
                report.captured_to_generated.within_one_px > 0.98,
                "{}: captured->generated 1px coverage {}",
                dir.display(),
                report.captured_to_generated.within_one_px
            );
            assert!(
                report.generated_to_captured.within_one_px > 0.98,
                "{}: generated->captured 1px coverage {}",
                dir.display(),
                report.generated_to_captured.within_one_px
            );
            for row in &report.width_rows {
                assert!(
                    row.delta.abs() < 0.02,
                    "{}: class {} width delta {}",
                    dir.display(),
                    row.class,
                    row.delta
                );
            }
        }
    }
}
