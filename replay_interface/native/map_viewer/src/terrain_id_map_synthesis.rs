use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const TERRAIN_ID_MAP_WIDTH: usize = 5632;
pub const TERRAIN_ID_MAP_HEIGHT: usize = 2048;
pub const TERRAIN_ID_MAP_DXGI_FORMAT: u32 = DXGI_FORMAT_B8G8R8A8_UNORM;
pub const TERRAIN_ID_MAP_MIP_COUNT: u32 = 1;
pub const TERRAIN_ID_MAP_SHADER_SLOT: u32 = 5;

#[derive(Clone, Debug)]
pub struct SynthesizedTerrainIdMap {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
    pub descriptor: TerrainIdMapDescriptor,
    pub source: TerrainIdMapSynthesisSource,
    pub packing_law: TerrainIdMapPackingLaw,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapDescriptor {
    pub format: String,
    pub dxgi_format: u32,
    pub width: usize,
    pub height: usize,
    pub byte_len: usize,
    pub row_pitch: usize,
    pub mip_count: u32,
    pub shader_slot: u32,
    pub fnv1a64: String,
    pub channel_order: Vec<String>,
    pub sampler: TerrainIdMapSamplerDescriptor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapSamplerDescriptor {
    pub mag_filter: String,
    pub min_filter: String,
    pub mip_filter: String,
    pub address_u: String,
    pub address_v: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapSynthesisSource {
    pub terrain_bmp: PathBuf,
    pub terrain_definitions: PathBuf,
    pub default_map: PathBuf,
    pub terrain_bmp_fnv1a64: String,
    pub terrain_definitions_fnv1a64: String,
    pub default_map_fnv1a64: String,
    pub palette_to_texture: BTreeMap<u8, u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapPackingLaw {
    pub channels: Vec<TerrainIdMapChannelLaw>,
    pub all_same_marker_channel: String,
    pub all_same_marker_add: u8,
    pub source_y_orientation: String,
    pub source_x_addressing: String,
    pub source_y_addressing: String,
    pub shader_sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapChannelLaw {
    pub channel: String,
    pub shader_name: String,
    pub source_offset_x: i32,
    pub source_offset_y: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainIdMapComparisonReport {
    pub capture_dir: PathBuf,
    pub captured_file: PathBuf,
    pub generated_descriptor: TerrainIdMapDescriptor,
    pub captured_descriptor: TerrainIdMapDescriptor,
    pub byte_exact: bool,
    pub byte_mismatches: usize,
    pub texel_mismatches: usize,
    pub texel_mismatch_percent: f64,
    pub channel_mismatches: [usize; 4],
    pub mismatch_bounds: Option<TerrainIdMapMismatchBounds>,
    pub mismatch_samples: Vec<TerrainIdMapMismatchSample>,
    pub spatial_pattern: String,
    pub orientation_evidence: TerrainIdMapOrientationEvidence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapMismatchBounds {
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapMismatchSample {
    pub x: usize,
    pub y: usize,
    pub generated: [u8; 4],
    pub captured: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainIdMapOrientationEvidence {
    pub label: String,
    pub x: usize,
    pub y: usize,
    pub captured: [u8; 4],
    pub generated_flipped_y: [u8; 4],
    pub generated_unflipped_y: [u8; 4],
    pub flipped_source_palette_index: u8,
    pub unflipped_source_palette_index: u8,
}

pub(crate) fn terrain_id_map_payload_fnv1a64(bytes: &[u8]) -> String {
    fnv1a64_bytes(bytes)
}

#[derive(Debug, Deserialize)]
struct TerrainIdMapCaptureJson {
    schema: String,
    file: String,
    width: usize,
    height: usize,
    byte_len: usize,
    fnv1a64: String,
    format: String,
    source_dxgi_format: u32,
    terrain_id_srv_slot: u32,
    sampler: TerrainIdMapSamplerDescriptor,
}

pub fn synthesize_terrain_id_map(resolver: &GameFileResolver) -> Result<SynthesizedTerrainIdMap> {
    let terrain_bmp = resolver.resolve_required("map/terrain.bmp", "terrain.bmp")?;
    let terrain_definitions = resolver.resolve_required(
        "common/terrain/00_terrain.txt",
        "common/terrain/00_terrain.txt",
    )?;
    let default_map = resolver.resolve_required("map/default.map", "map/default.map")?;
    validate_default_map_terrain_binding(resolver, &default_map)?;
    let terrain_bmp_bytes = resolver.read_resolved_bytes(&terrain_bmp)?;
    let terrain_bmp_fnv1a64 = fnv1a64_bytes(&terrain_bmp_bytes);
    let terrain_definitions_fnv1a64 = fnv1a64_bytes(&resolver.read_resolved_bytes(&terrain_definitions)?);
    let default_map_fnv1a64 = fnv1a64_bytes(&resolver.read_resolved_bytes(&default_map)?);
    let cache_key = terrain_id_map_synthesis_cache_key(
        &terrain_bmp_fnv1a64,
        &terrain_definitions_fnv1a64,
        &default_map_fnv1a64,
    );
    {
        let cache = terrain_id_map_synthesis_cache().lock().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "TerrainIDMap synthesis cache lock poisoned",
            )
        })?;
        if let Some(cached) = cache.get(&cache_key).cloned() {
            return Ok(with_current_terrain_id_map_source_paths(
                cached,
                &terrain_bmp,
                &terrain_definitions,
                &default_map,
            ));
        }
    }

    let terrain_indices = decode_bmp_indexed(&terrain_bmp_bytes, &terrain_bmp)?;
    if terrain_indices.width == 0 || terrain_indices.height == 0 {
        return Err("terrain.bmp has empty dimensions".into());
    }

    let terrain_texture_map = terrain_id_palette_texture_map(resolver, &terrain_definitions)?;
    let source_texture_ids =
        terrain_texture_ids_fail_closed(&terrain_indices, &terrain_texture_map)?;
    let bytes = pack_terrain_id_map_rgba(
        &source_texture_ids,
        terrain_indices.width,
        terrain_indices.height,
    )?;
    let descriptor = TerrainIdMapDescriptor {
        format: TERRAIN_ID_MAP_FORMAT.to_string(),
        dxgi_format: TERRAIN_ID_MAP_DXGI_FORMAT,
        width: terrain_indices.width,
        height: terrain_indices.height,
        byte_len: bytes.len(),
        row_pitch: terrain_indices.width * 4,
        mip_count: TERRAIN_ID_MAP_MIP_COUNT,
        shader_slot: TERRAIN_ID_MAP_SHADER_SLOT,
        fnv1a64: terrain_id_map_payload_fnv1a64(&bytes),
        channel_order: terrain_id_map_channel_order(),
        sampler: terrain_id_map_sampler_descriptor(),
    };
    let synthesized = SynthesizedTerrainIdMap {
        width: terrain_indices.width,
        height: terrain_indices.height,
        bytes,
        descriptor,
        source: TerrainIdMapSynthesisSource {
            terrain_bmp: terrain_bmp.clone(),
            terrain_definitions: terrain_definitions.clone(),
            default_map: default_map.clone(),
            terrain_bmp_fnv1a64,
            terrain_definitions_fnv1a64,
            default_map_fnv1a64,
            palette_to_texture: terrain_texture_map,
        },
        packing_law: terrain_id_map_packing_law(),
    };
    terrain_id_map_synthesis_cache()
        .lock()
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "TerrainIDMap synthesis cache lock poisoned",
            )
        })?
        .insert(cache_key, synthesized.clone());
    Ok(synthesized)
}

fn terrain_id_map_synthesis_cache() -> &'static Mutex<BTreeMap<String, SynthesizedTerrainIdMap>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, SynthesizedTerrainIdMap>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn terrain_id_map_synthesis_cache_key(
    terrain_bmp_fnv1a64: &str,
    terrain_definitions_fnv1a64: &str,
    default_map_fnv1a64: &str,
) -> String {
    format!("{terrain_bmp_fnv1a64}|{terrain_definitions_fnv1a64}|{default_map_fnv1a64}")
}

fn with_current_terrain_id_map_source_paths(
    mut synthesized: SynthesizedTerrainIdMap,
    terrain_bmp: &Path,
    terrain_definitions: &Path,
    default_map: &Path,
) -> SynthesizedTerrainIdMap {
    synthesized.source.terrain_bmp = terrain_bmp.to_path_buf();
    synthesized.source.terrain_definitions = terrain_definitions.to_path_buf();
    synthesized.source.default_map = default_map.to_path_buf();
    synthesized
}

pub fn compare_terrain_id_map_to_capture(
    resolver: &GameFileResolver,
    capture_dir: &Path,
) -> Result<TerrainIdMapComparisonReport> {
    let synthesized = synthesize_terrain_id_map(resolver)?;
    let (capture_meta, captured_file, captured_bytes) = load_captured_terrain_id_map(capture_dir)?;
    if captured_bytes.len() != synthesized.bytes.len() {
        return Err(format!(
            "{} has {} bytes, generated TerrainIDMap has {} bytes",
            captured_file.display(),
            captured_bytes.len(),
            synthesized.bytes.len()
        )
        .into());
    }
    if captured_bytes.len() != capture_meta.byte_len {
        return Err(format!(
            "{} has {} bytes, capture JSON declares {}",
            captured_file.display(),
            captured_bytes.len(),
            capture_meta.byte_len
        )
        .into());
    }
    let captured_fnv = terrain_id_map_payload_fnv1a64(&captured_bytes);
    if captured_fnv != capture_meta.fnv1a64 {
        return Err(format!(
            "{} FNV-1a64 is {}, capture JSON declares {}",
            captured_file.display(),
            captured_fnv,
            capture_meta.fnv1a64
        )
        .into());
    }

    let mut byte_mismatches = 0usize;
    let mut texel_mismatches = 0usize;
    let mut channel_mismatches = [0usize; 4];
    let mut bounds: Option<TerrainIdMapMismatchBounds> = None;
    let mut samples = Vec::new();
    for y in 0..synthesized.height {
        for x in 0..synthesized.width {
            let offset = (y * synthesized.width + x) * 4;
            let generated = [
                synthesized.bytes[offset],
                synthesized.bytes[offset + 1],
                synthesized.bytes[offset + 2],
                synthesized.bytes[offset + 3],
            ];
            let captured = [
                captured_bytes[offset],
                captured_bytes[offset + 1],
                captured_bytes[offset + 2],
                captured_bytes[offset + 3],
            ];
            let mut texel_differs = false;
            for channel in 0..4 {
                if generated[channel] != captured[channel] {
                    byte_mismatches += 1;
                    channel_mismatches[channel] += 1;
                    texel_differs = true;
                }
            }
            if texel_differs {
                texel_mismatches += 1;
                extend_mismatch_bounds(&mut bounds, x, y);
                if samples.len() < 16 {
                    samples.push(TerrainIdMapMismatchSample {
                        x,
                        y,
                        generated,
                        captured,
                    });
                }
            }
        }
    }

    let captured_descriptor = TerrainIdMapDescriptor {
        format: capture_meta.format,
        dxgi_format: capture_meta.source_dxgi_format,
        width: capture_meta.width,
        height: capture_meta.height,
        byte_len: capture_meta.byte_len,
        row_pitch: capture_meta.width * 4,
        mip_count: TERRAIN_ID_MAP_MIP_COUNT,
        shader_slot: capture_meta.terrain_id_srv_slot,
        fnv1a64: capture_meta.fnv1a64,
        channel_order: terrain_id_map_channel_order(),
        sampler: capture_meta.sampler,
    };
    validate_capture_descriptor(&captured_descriptor)?;
    let total_texels = synthesized.width * synthesized.height;
    let texel_mismatch_percent = (texel_mismatches as f64 * 100.0) / total_texels as f64;
    Ok(TerrainIdMapComparisonReport {
        capture_dir: capture_dir.to_path_buf(),
        captured_file,
        generated_descriptor: synthesized.descriptor,
        captured_descriptor,
        byte_exact: byte_mismatches == 0,
        byte_mismatches,
        texel_mismatches,
        texel_mismatch_percent,
        channel_mismatches,
        mismatch_bounds: bounds.clone(),
        mismatch_samples: samples,
        spatial_pattern: describe_mismatch_pattern(byte_mismatches, texel_mismatches, &bounds),
        orientation_evidence: terrain_id_map_orientation_evidence(resolver, &captured_bytes)?,
    })
}

pub fn dump_terrain_id_map_comparison_bmps(
    resolver: &GameFileResolver,
    capture_dir: &Path,
    out_dir: &Path,
) -> Result<Vec<PathBuf>> {
    let synthesized = synthesize_terrain_id_map(resolver)?;
    let (_, _, captured_bytes) = load_captured_terrain_id_map(capture_dir)?;
    if captured_bytes.len() != synthesized.bytes.len() {
        return Err("captured/generated TerrainIDMap byte lengths differ; refusing dump".into());
    }
    fs::create_dir_all(out_dir)?;
    let mut paths = Vec::new();
    for (channel, name) in ["r", "g", "b", "a"].iter().enumerate() {
        let generated = extract_channel(&synthesized.bytes, channel);
        let captured = extract_channel(&captured_bytes, channel);
        let delta = generated
            .iter()
            .zip(&captured)
            .map(|(left, right)| if left == right { 0 } else { 255 })
            .collect::<Vec<_>>();
        let generated_path = out_dir.join(format!("terrain_id_map_generated_{name}.bmp"));
        let captured_path = out_dir.join(format!("terrain_id_map_captured_{name}.bmp"));
        let delta_path = out_dir.join(format!("terrain_id_map_delta_{name}.bmp"));
        write_gray8_bmp(
            &generated_path,
            synthesized.width,
            synthesized.height,
            &generated,
        )?;
        write_gray8_bmp(
            &captured_path,
            synthesized.width,
            synthesized.height,
            &captured,
        )?;
        write_gray8_bmp(&delta_path, synthesized.width, synthesized.height, &delta)?;
        paths.extend([generated_path, captured_path, delta_path]);
    }
    Ok(paths)
}

pub fn terrain_id_map_packing_law() -> TerrainIdMapPackingLaw {
    TerrainIdMapPackingLaw {
        channels: vec![
            TerrainIdMapChannelLaw {
                channel: "R".to_string(),
                shader_name: "ColorRD / IndexU.x".to_string(),
                source_offset_x: 0,
                source_offset_y: 1,
            },
            TerrainIdMapChannelLaw {
                channel: "G".to_string(),
                shader_name: "ColorLU / IndexU.y".to_string(),
                source_offset_x: 1,
                source_offset_y: 0,
            },
            TerrainIdMapChannelLaw {
                channel: "B".to_string(),
                shader_name: "ColorRU / IndexU.z plus all-same marker".to_string(),
                source_offset_x: 0,
                source_offset_y: 0,
            },
            TerrainIdMapChannelLaw {
                channel: "A".to_string(),
                shader_name: "initial diffuse alpha / IndexU.w".to_string(),
                source_offset_x: 1,
                source_offset_y: 1,
            },
        ],
        all_same_marker_channel: "B".to_string(),
        all_same_marker_add: 100,
        source_y_orientation: "source row is terrain.bmp loaded top-down, then vertically flipped before packing".to_string(),
        source_x_addressing: "clamp".to_string(),
        source_y_addressing: "clamp".to_string(),
        shader_sources: vec![
            "gfx/FX/pdxmap.shader:59-67 TerrainIDMap point/clamp sampler".to_string(),
            "gfx/FX/pdxmap.shader:234-239 samples TerrainIDMap at Input.uv - half texel".to_string(),
            "gfx/FX/standardfuncsgfx.fxh:1065-1073 decodes RGBA bytes into atlas tile U/V, with B-100 all-same marker".to_string(),
            "gfx/FX/pdxmap.shader:252-279 consumes A as base and R/G/B as ColorRD/ColorLU/ColorRU in four-way blend".to_string(),
        ],
    }
}

fn terrain_id_map_sampler_descriptor() -> TerrainIdMapSamplerDescriptor {
    let sampler = terrain_id_map_sampler();
    TerrainIdMapSamplerDescriptor {
        mag_filter: sampler.mag_filter,
        min_filter: sampler.min_filter,
        mip_filter: sampler.mip_filter,
        address_u: sampler.address_u,
        address_v: sampler.address_v,
    }
}

fn terrain_id_palette_texture_map(resolver: &GameFileResolver, path: &Path) -> Result<BTreeMap<u8, u8>> {
    let map = load_terrain_texture_map(resolver, path)?;
    let expected = [
        (0, 1),
        (1, 4),
        (2, 3),
        (3, 9),
        (4, 5),
        (5, 0),
        (6, 11),
        (7, 12),
        (8, 14),
        (9, 6),
        (10, 13),
        (11, 11),
        (12, 8),
        (13, 10),
        (14, 255),
        (15, 9),
        (16, 11),
        (19, 0),
        (17, 2),
        (18, 7),
        (20, 7),
        (21, 4),
        (22, 5),
        (27, 7),
        (31, 15),
    ];
    for (palette, texture) in expected {
        match map.get(&palette) {
            Some(actual) if *actual == texture => {}
            Some(actual) => {
                return Err(format!(
                    "{} palette index {palette} maps to texture {actual}, expected {texture}",
                    path.display()
                )
                .into())
            }
            None => {
                return Err(format!(
                    "{} is missing graphical terrain palette index {palette}",
                    path.display()
                )
                .into())
            }
        }
    }
    Ok(map)
}

fn terrain_texture_ids_fail_closed(
    terrain_indices: &IndexImage,
    terrain_texture_map: &BTreeMap<u8, u8>,
) -> Result<Vec<u8>> {
    const NULL_OBJECT_TEXTURE_ID: u8 = 0;
    let texture_ids = terrain_indices
        .pixels
        .iter()
        .map(|pal| {
            terrain_texture_map
                .get(pal)
                .copied()
                .unwrap_or(NULL_OBJECT_TEXTURE_ID)
        })
        .collect();
    Ok(texture_ids)
}

fn pack_terrain_id_map_rgba(texture_ids: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    if texture_ids.len() != width * height {
        return Err(format!(
            "terrain texture id source has {} texels, expected {}",
            texture_ids.len(),
            width * height
        )
        .into());
    }
    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let packed = pack_terrain_id_map_texel(texture_ids, width, height, x, y, true)?;
            let offset = (y * width + x) * 4;
            out[offset..offset + 4].copy_from_slice(&packed);
        }
    }
    Ok(out)
}

fn pack_terrain_id_map_texel(
    texture_ids: &[u8],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    flip_y: bool,
) -> Result<[u8; 4]> {
    let source_y = if flip_y { height - 1 - y } else { y };
    let sample = |dx: usize, dy: usize| -> u8 {
        let sx = (x + dx).min(width - 1);
        let sy = if flip_y {
            source_y.saturating_sub(dy)
        } else {
            (source_y + dy).min(height - 1)
        };
        texture_ids[sy * width + sx]
    };
    let down = sample(0, 1);
    let right = sample(1, 0);
    let center = sample(0, 0);
    let right_down = sample(1, 1);
    let mut center_or_marker = center;
    if down == right && down == center && down == right_down {
        center_or_marker = center.checked_add(100).ok_or_else(|| {
            format!("terrain texture id {center} cannot carry +100 all-same marker")
        })?;
    }
    Ok([down, right, center_or_marker, right_down])
}

fn validate_default_map_terrain_binding(resolver: &GameFileResolver, default_map: &Path) -> Result<()> {
    let text = strip_comments(&resolver.read_resolved_text(default_map)?);
    let Some(value) = parse_quoted_value(&text, "terrain") else {
        return Err(format!(
            "{} does not declare terrain = \"terrain.bmp\"",
            default_map.display()
        )
        .into());
    };
    if value != "terrain.bmp" {
        return Err(format!(
            "{} binds terrain = {:?}, expected \"terrain.bmp\"",
            default_map.display(),
            value
        )
        .into());
    }
    Ok(())
}

fn load_captured_terrain_id_map(
    capture_dir: &Path,
) -> Result<(TerrainIdMapCaptureJson, PathBuf, Vec<u8>)> {
    let capture_dir = require_dir(capture_dir, "TerrainIDMap capture directory")?;
    let capture_json = capture_dir.join("terrain_id_map_capture.json");
    let capture_meta: TerrainIdMapCaptureJson = serde_json::from_slice(&fs::read(&capture_json)?)?;
    if capture_meta.schema != "battle-plan-terrain-id-map-capture-v1" {
        return Err(format!(
            "{} has unsupported schema {}",
            capture_json.display(),
            capture_meta.schema
        )
        .into());
    }
    let captured_file = capture_dir.join(&capture_meta.file);
    let captured_bytes = fs::read(&captured_file)?;
    Ok((capture_meta, captured_file, captured_bytes))
}

fn validate_capture_descriptor(descriptor: &TerrainIdMapDescriptor) -> Result<()> {
    if descriptor.format != TERRAIN_ID_MAP_FORMAT {
        return Err(format!(
            "captured TerrainIDMap format is {}, expected {}",
            descriptor.format, TERRAIN_ID_MAP_FORMAT
        )
        .into());
    }
    if descriptor.dxgi_format != TERRAIN_ID_MAP_DXGI_FORMAT {
        return Err(format!(
            "captured TerrainIDMap source DXGI format is {}, expected {}",
            descriptor.dxgi_format, TERRAIN_ID_MAP_DXGI_FORMAT
        )
        .into());
    }
    if descriptor.width != TERRAIN_ID_MAP_WIDTH || descriptor.height != TERRAIN_ID_MAP_HEIGHT {
        return Err(format!(
            "captured TerrainIDMap dimensions are {}x{}, expected {}x{}",
            descriptor.width, descriptor.height, TERRAIN_ID_MAP_WIDTH, TERRAIN_ID_MAP_HEIGHT
        )
        .into());
    }
    if descriptor.mip_count != TERRAIN_ID_MAP_MIP_COUNT {
        return Err(format!(
            "captured TerrainIDMap mip count is {}, expected {}",
            descriptor.mip_count, TERRAIN_ID_MAP_MIP_COUNT
        )
        .into());
    }
    if descriptor.shader_slot != TERRAIN_ID_MAP_SHADER_SLOT {
        return Err(format!(
            "captured TerrainIDMap shader slot is {}, expected {}",
            descriptor.shader_slot, TERRAIN_ID_MAP_SHADER_SLOT
        )
        .into());
    }
    Ok(())
}

fn extend_mismatch_bounds(bounds: &mut Option<TerrainIdMapMismatchBounds>, x: usize, y: usize) {
    match bounds {
        Some(bounds) => {
            bounds.min_x = bounds.min_x.min(x);
            bounds.min_y = bounds.min_y.min(y);
            bounds.max_x = bounds.max_x.max(x);
            bounds.max_y = bounds.max_y.max(y);
        }
        None => {
            *bounds = Some(TerrainIdMapMismatchBounds {
                min_x: x,
                min_y: y,
                max_x: x,
                max_y: y,
            });
        }
    }
}

fn describe_mismatch_pattern(
    byte_mismatches: usize,
    texel_mismatches: usize,
    bounds: &Option<TerrainIdMapMismatchBounds>,
) -> String {
    if byte_mismatches == 0 {
        return "byte_exact".to_string();
    }
    match bounds {
        Some(bounds) => format!(
            "{texel_mismatches} texel(s) mismatched inside x={}..{}, y={}..{}",
            bounds.min_x, bounds.max_x, bounds.min_y, bounds.max_y
        ),
        None => format!("{texel_mismatches} texel(s) mismatched"),
    }
}

fn terrain_id_map_orientation_evidence(
    resolver: &GameFileResolver,
    captured_bytes: &[u8],
) -> Result<TerrainIdMapOrientationEvidence> {
    let x = 3000usize;
    let y = 950usize;
    let terrain_bmp = resolver.resolve_required("map/terrain.bmp", "terrain.bmp")?;
    let terrain_definitions = resolver.resolve_required(
        "common/terrain/00_terrain.txt",
        "common/terrain/00_terrain.txt",
    )?;
    let terrain_indices = decode_bmp_indexed(&resolver.read_resolved_bytes(&terrain_bmp)?, &terrain_bmp)?;
    let terrain_texture_map = terrain_id_palette_texture_map(resolver, &terrain_definitions)?;
    let source_texture_ids =
        terrain_texture_ids_fail_closed(&terrain_indices, &terrain_texture_map)?;
    let generated_flipped_y = pack_terrain_id_map_texel(
        &source_texture_ids,
        terrain_indices.width,
        terrain_indices.height,
        x,
        y,
        true,
    )?;
    let generated_unflipped_y = pack_terrain_id_map_texel(
        &source_texture_ids,
        terrain_indices.width,
        terrain_indices.height,
        x,
        y,
        false,
    )?;
    let offset = (y * terrain_indices.width + x) * 4;
    let captured = [
        captured_bytes[offset],
        captured_bytes[offset + 1],
        captured_bytes[offset + 2],
        captured_bytes[offset + 3],
    ];
    Ok(TerrainIdMapOrientationEvidence {
        label: "Italy coastline sample; flipped terrain.bmp row matches captured TerrainIDMap, unflipped row does not".to_string(),
        x,
        y,
        captured,
        generated_flipped_y,
        generated_unflipped_y,
        flipped_source_palette_index: terrain_indices.pixels
            [(terrain_indices.height - 1 - y) * terrain_indices.width + x],
        unflipped_source_palette_index: terrain_indices.pixels[y * terrain_indices.width + x],
    })
}

fn extract_channel(bytes: &[u8], channel: usize) -> Vec<u8> {
    bytes.chunks_exact(4).map(|texel| texel[channel]).collect()
}

fn write_gray8_bmp(path: &Path, width: usize, height: usize, pixels: &[u8]) -> Result<()> {
    if pixels.len() != width * height {
        return Err(format!(
            "{} gray8 pixel length is {}, expected {}",
            path.display(),
            pixels.len(),
            width * height
        )
        .into());
    }
    let row_stride = (width + 3) & !3;
    let palette_bytes = 256usize * 4;
    let pixel_offset = 14usize + 40usize + palette_bytes;
    let image_bytes = row_stride * height;
    let file_size = pixel_offset + image_bytes;
    let mut file = fs::File::create(path)?;
    file.write_all(b"BM")?;
    file.write_all(&(file_size as u32).to_le_bytes())?;
    file.write_all(&[0u8; 4])?;
    file.write_all(&(pixel_offset as u32).to_le_bytes())?;
    file.write_all(&40u32.to_le_bytes())?;
    file.write_all(&(width as i32).to_le_bytes())?;
    file.write_all(&(height as i32).to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&8u16.to_le_bytes())?;
    file.write_all(&0u32.to_le_bytes())?;
    file.write_all(&(image_bytes as u32).to_le_bytes())?;
    file.write_all(&2835u32.to_le_bytes())?;
    file.write_all(&2835u32.to_le_bytes())?;
    file.write_all(&256u32.to_le_bytes())?;
    file.write_all(&256u32.to_le_bytes())?;
    for value in 0..=255u8 {
        file.write_all(&[value, value, value, 0])?;
    }
    let padding = vec![0u8; row_stride - width];
    for y in (0..height).rev() {
        let start = y * width;
        file.write_all(&pixels[start..start + width])?;
        file.write_all(&padding)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::fs;

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    const SIBLING_CAPTURE_DIR: &str = r"C:\Users\Example\AppData\Local\hoi4_captures\live_B_full";

    #[test]
    fn terrain_id_map_pack_law_marks_all_same_in_b_channel() {
        let source = vec![1u8, 2, 3, 4];
        let packed = pack_terrain_id_map_texel(&source, 2, 2, 0, 0, false).unwrap();
        assert_eq!(packed, [3, 2, 1, 4]);
        let source = vec![5u8, 5, 5, 5];
        let packed = pack_terrain_id_map_texel(&source, 2, 2, 0, 0, false).unwrap();
        assert_eq!(packed, [5, 5, 105, 5]);
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn terrain_texture_table_matches_vanilla_atlas_order_when_available() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).unwrap();
        let terrain_definitions = resolver
            .resolve_required(
                "common/terrain/00_terrain.txt",
                "common/terrain/00_terrain.txt",
            )
            .unwrap();
        let table = terrain_id_palette_texture_map(&resolver, &terrain_definitions).unwrap();
        let ordered = [
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 19, 17, 18, 20, 21, 22, 27,
            31,
        ]
        .into_iter()
        .map(|index| (index, *table.get(&index).unwrap()))
        .collect::<Vec<_>>();
        assert_eq!(
            ordered,
            vec![
                (0, 1),
                (1, 4),
                (2, 3),
                (3, 9),
                (4, 5),
                (5, 0),
                (6, 11),
                (7, 12),
                (8, 14),
                (9, 6),
                (10, 13),
                (11, 11),
                (12, 8),
                (13, 10),
                (14, 255),
                (15, 9),
                (16, 11),
                (19, 0),
                (17, 2),
                (18, 7),
                (20, 7),
                (21, 4),
                (22, 5),
                (27, 7),
                (31, 15),
            ]
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_terrain_id_capture_with_hoi4),
        ignore = "the captured TerrainIDMap directory (the legacy session_20260702d path, else the preserved gate18_final capture), plus the game install"
    )]
    fn terrain_id_map_synthesis_matches_captured_reference_when_available() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let capture_dir = corpus::terrain_id_capture_dir()
            .expect("the TerrainIDMap capture gate was true at build time");

        let resolver = GameFileResolver::vanilla(game_dir).unwrap();
        let report = compare_terrain_id_map_to_capture(&resolver, &capture_dir).unwrap();
        println!(
            "TerrainIDMap main verdict: byte_exact={} byte_mismatches={} texel_mismatches={} ({:.8}%) channel_mismatches={:?} pattern={} fnv={}",
            report.byte_exact,
            report.byte_mismatches,
            report.texel_mismatches,
            report.texel_mismatch_percent,
            report.channel_mismatches,
            report.spatial_pattern,
            report.generated_descriptor.fnv1a64
        );
        println!(
            "TerrainIDMap orientation evidence: {} @({}, {}) captured={:?} flipped={:?} unflipped={:?} flipped_palette={} unflipped_palette={}",
            report.orientation_evidence.label,
            report.orientation_evidence.x,
            report.orientation_evidence.y,
            report.orientation_evidence.captured,
            report.orientation_evidence.generated_flipped_y,
            report.orientation_evidence.generated_unflipped_y,
            report.orientation_evidence.flipped_source_palette_index,
            report.orientation_evidence.unflipped_source_palette_index
        );
        assert!(report.byte_exact, "{report:#?}");

        let sibling_capture_dir = Path::new(SIBLING_CAPTURE_DIR);
        if sibling_capture_dir.is_dir() {
            let sibling_report =
                compare_terrain_id_map_to_capture(&resolver, sibling_capture_dir).unwrap();
            println!(
                "TerrainIDMap sibling verdict: dir={} byte_exact={} byte_mismatches={} texel_mismatches={} fnv={}",
                sibling_capture_dir.display(),
                sibling_report.byte_exact,
                sibling_report.byte_mismatches,
                sibling_report.texel_mismatches,
                sibling_report.generated_descriptor.fnv1a64
            );
            assert!(sibling_report.byte_exact, "{sibling_report:#?}");
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn terrain_id_map_orphan_palette_set_is_narrow() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        use std::collections::BTreeSet;
        let resolver = GameFileResolver::vanilla(game_dir).unwrap();
        let terrain_bmp = resolver
            .resolve_required("map/terrain.bmp", "terrain.bmp")
            .unwrap();
        let terrain_definitions = resolver
            .resolve_required(
                "common/terrain/00_terrain.txt",
                "common/terrain/00_terrain.txt",
            )
            .unwrap();
        let terrain_indices = load_bmp_indexed(&terrain_bmp).unwrap();
        let terrain_texture_map = terrain_id_palette_texture_map(&resolver, &terrain_definitions).unwrap();
        let used = terrain_indices
            .pixels
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let orphan_used = used
            .into_iter()
            .filter(|index| !terrain_texture_map.contains_key(index))
            .collect::<Vec<_>>();
        assert_eq!(orphan_used, vec![48, 61, 63, 64, 74, 77, 80]);
        let resolved =
            terrain_texture_ids_fail_closed(&terrain_indices, &terrain_texture_map).unwrap();
        for (pixel, tile) in terrain_indices.pixels.iter().zip(resolved.iter()) {
            if orphan_used.contains(pixel) {
                assert_eq!(
                    *tile, 0,
                    "orphan palette index {pixel} must resolve to tile 0"
                );
            }
        }
    }

    #[test]
    fn terrain_id_map_synthesis_resolver_uses_modded_terrain_bmp() {
        let root = temp_test_dir("terrain_id_map_resolver_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_synthetic_terrain_id_map_game_files(&vanilla, 0);
        let mod_terrain_bmp = mod_root.join("map/terrain.bmp");
        fs::create_dir_all(mod_terrain_bmp.parent().expect("mod terrain parent"))
            .expect("create mod terrain parent");
        write_indexed_bmp(
            &mod_terrain_bmp,
            TERRAIN_ID_MAP_WIDTH,
            TERRAIN_ID_MAP_HEIGHT,
            1,
        );

        let vanilla_resolver =
            GameFileResolver::vanilla(vanilla.clone()).expect("vanilla resolver");
        let mod_resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()])
            .expect("mod resolver");

        let vanilla_map = synthesize_terrain_id_map(&vanilla_resolver).expect("vanilla synth");
        let modded_map = synthesize_terrain_id_map(&mod_resolver).expect("modded synth");

        assert_ne!(
            vanilla_map.descriptor.fnv1a64,
            modded_map.descriptor.fnv1a64
        );
        assert!(modded_map.source.terrain_bmp.starts_with(mod_root));
        assert_eq!(mod_resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }
}
