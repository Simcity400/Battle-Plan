use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkSynthesisWorldRect {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkSynthesisLodMapping {
    pub tile_size: u32,
    pub cb1_scale: f32,
    pub template_chunk_id: String,
    pub draw_call: String,
    pub draw_count: u32,
    pub shared_slot0_vertex_fnv1a64: Option<String>,
    pub lod_slot1_vertex_fnv1a64: Option<String>,
    pub slot1_vertex_tile_stride_bytes: u32,
    pub index_fnv1a64: Option<String>,
    pub captured_record_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkSynthesisTile {
    pub tile_x: u32,
    pub tile_z: u32,
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
    pub quad_offset_scale_is_detail: [f32; 4],
    pub detail_edge_flags: [f32; 4],
    pub slot1_vertex_offset: u32,
}

#[derive(Clone, Debug)]
pub struct TerrainChunkSynthesisPlan {
    pub(crate) world_rect: TerrainChunkSynthesisWorldRect,
    pub(crate) target_tile_size: u32,
    pub(crate) cb1_scale: f32,
    pub(crate) template_chunk_id: String,
    pub(crate) template_draw: MapFrameDraw,
    pub(crate) tiles: Vec<TerrainChunkSynthesisTile>,
    pub(crate) lod_mappings: Vec<TerrainChunkSynthesisLodMapping>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkSynthesisComparison {
    pub record_chunk_id: String,
    pub tile_size: u32,
    pub cb1_scale: f32,
    pub quad_offset_scale_is_detail: [f32; 4],
    pub captured_fnv1a64: String,
    pub synthesized_fnv1a64: String,
    pub byte_len: usize,
    pub identical: bool,
    pub first_mismatch_byte: Option<usize>,
    pub first_mismatch_texel: Option<[u32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkSynthesisValidationReport {
    pub schema: String,
    pub status: String,
    pub compared_records: Vec<TerrainChunkSynthesisComparison>,
    pub lod_mappings: Vec<TerrainChunkSynthesisLodMapping>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkSynthesisSmokeReport {
    pub schema: String,
    pub status: String,
    pub target_tile_size: u32,
    pub expected_tile_count: usize,
    pub rendered_tile_count: usize,
    pub elapsed_ms: f64,
    pub target_width: u32,
    pub target_height: u32,
    pub target_format: u32,
    /// Map frame bundle whose scene CB0 (sun/time/snow/fog state) replaced the
    /// chunk templates' per-frame scene constants; None means the templates'
    /// own captured frame state was replayed verbatim.
    #[serde(default)]
    pub scene_frame_bundle: Option<String>,
    pub lod_mappings: Vec<TerrainChunkSynthesisLodMapping>,
}

impl TerrainChunkSynthesisWorldRect {
    pub fn new(min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> Result<Self> {
        let rect = Self {
            min_x,
            min_z,
            max_x,
            max_z,
        };
        rect.validate()?;
        Ok(rect)
    }

    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f32 {
        self.max_z - self.min_z
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if [self.min_x, self.min_z, self.max_x, self.max_z]
            .iter()
            .any(|value| !value.is_finite())
            || self.min_x >= self.max_x
            || self.min_z >= self.max_z
        {
            return Err("production terrain chunk synthesis blocked: world rect is invalid".into());
        }
        Ok(())
    }
}

impl From<TerrainChunkWorldRect> for TerrainChunkSynthesisWorldRect {
    fn from(value: TerrainChunkWorldRect) -> Self {
        Self {
            min_x: value.min_x,
            min_z: value.min_z,
            max_x: value.max_x,
            max_z: value.max_z,
        }
    }
}

impl TerrainChunkSynthesisPlan {
    pub fn target_tile_size(&self) -> u32 {
        self.target_tile_size
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn template_chunk_id(&self) -> &str {
        &self.template_chunk_id
    }

    pub fn cb1_scale(&self) -> f32 {
        self.cb1_scale
    }

    pub fn world_rect(&self) -> TerrainChunkSynthesisWorldRect {
        self.world_rect
    }

    pub fn tiles(&self) -> &[TerrainChunkSynthesisTile] {
        &self.tiles
    }

    pub fn lod_mappings(&self) -> &[TerrainChunkSynthesisLodMapping] {
        &self.lod_mappings
    }
}

pub fn terrain_chunk_synthesis_plan(
    library: &NativeD3d11TerrainChunkLibrary,
    target_tile_size: u32,
    world_rect: TerrainChunkSynthesisWorldRect,
) -> Result<TerrainChunkSynthesisPlan> {
    terrain_chunk_synthesis_plan_with_detail_flags(library, target_tile_size, world_rect, false)
}

pub(crate) fn terrain_chunk_synthesis_validation_plan(
    library: &NativeD3d11TerrainChunkLibrary,
    target_tile_size: u32,
    world_rect: TerrainChunkSynthesisWorldRect,
) -> Result<TerrainChunkSynthesisPlan> {
    terrain_chunk_synthesis_plan_with_detail_flags(library, target_tile_size, world_rect, true)
}

fn terrain_chunk_synthesis_plan_with_detail_flags(
    library: &NativeD3d11TerrainChunkLibrary,
    target_tile_size: u32,
    world_rect: TerrainChunkSynthesisWorldRect,
    use_captured_detail_flags: bool,
) -> Result<TerrainChunkSynthesisPlan> {
    world_rect.validate()?;
    if target_tile_size == 0 {
        return Err(
            "production terrain chunk synthesis blocked: target LOD tile size must be positive"
                .into(),
        );
    }
    let lods = terrain_chunk_synthesis_lod_sources(library)?;
    let source = lods.get(&target_tile_size).ok_or_else(|| {
        format!(
            "production terrain chunk synthesis blocked: no captured template for {target_tile_size} world-unit terrain chunks; captured LODs are [{}]",
            lods.keys()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let tiles = terrain_chunk_synthesis_tiles(
        world_rect,
        library.world_rect(),
        target_tile_size,
        source.cb1_scale,
        source.slot1_vertex_tile_stride_bytes,
        if use_captured_detail_flags {
            Some(&source.cb1_detail_edge_flags_by_origin)
        } else {
            None
        },
    )?;
    Ok(TerrainChunkSynthesisPlan {
        world_rect,
        target_tile_size,
        cb1_scale: source.cb1_scale,
        template_chunk_id: source.template.chunk_id.clone(),
        template_draw: source.template.draw.clone(),
        tiles,
        lod_mappings: lods.values().map(TerrainChunkLodSource::mapping).collect(),
    })
}

pub(crate) fn terrain_chunk_synthesis_lod_sources(
    library: &NativeD3d11TerrainChunkLibrary,
) -> Result<BTreeMap<u32, TerrainChunkLodSource<'_>>> {
    let mut sources: BTreeMap<u32, TerrainChunkLodSource<'_>> = BTreeMap::new();
    for chunk in &library.manifest.chunks {
        let cb1 = terrain_chunk_draw_quad_offset_scale(&chunk.draw, library.bundle_dir())?;
        let tile_size = terrain_chunk_world_tile_size(chunk)?;
        terrain_chunk_validate_cb1_matches_bounds(chunk, cb1, tile_size)?;
        let entry = sources
            .entry(tile_size)
            .or_insert_with(|| TerrainChunkLodSource {
                tile_size,
                cb1_scale: cb1[2],
                template: chunk,
                record_count: 0,
                slot0_vertex_hashes: BTreeSet::new(),
                slot1_vertex_hashes: BTreeSet::new(),
                index_hashes: BTreeSet::new(),
                draw_identities: BTreeSet::new(),
                slot1_vertex_offsets: Vec::new(),
                slot1_vertex_tile_stride_bytes: 0,
                cb1_detail_edge_flags_by_origin: BTreeMap::new(),
            });
        if (entry.cb1_scale - cb1[2]).abs() > f32::EPSILON {
            return Err(format!(
                "production terrain chunk synthesis blocked: tile size {tile_size} has inconsistent CB1 scales {} and {}",
                entry.cb1_scale, cb1[2]
            )
            .into());
        }
        entry.record_count += 1;
        entry.cb1_detail_edge_flags_by_origin.insert(
            terrain_chunk_origin_key(chunk.bounds.min_x, chunk.bounds.min_z)?,
            terrain_chunk_draw_detail_edge_flags(&chunk.draw, library.bundle_dir())?,
        );
        if let Some(hash) = chunk
            .draw
            .vertex_buffers
            .get("0")
            .and_then(|binding| binding.fnv1a64.clone())
        {
            entry.slot0_vertex_hashes.insert(hash);
        }
        if let Some(binding) = chunk.draw.vertex_buffers.get("1") {
            if let Some(hash) = binding.fnv1a64.clone() {
                entry.slot1_vertex_hashes.insert(hash);
            }
            let offset = binding.offset.ok_or_else(|| {
                format!(
                    "production terrain chunk synthesis blocked: chunk {} slot-1 vertex buffer has no offset",
                    chunk.chunk_id
                )
            })?;
            entry
                .slot1_vertex_offsets
                .push(TerrainChunkSlot1OffsetRecord {
                    chunk_id: chunk.chunk_id.clone(),
                    min_x: chunk.bounds.min_x,
                    min_z: chunk.bounds.min_z,
                    offset,
                });
        }
        if let Some(hash) = chunk
            .draw
            .index_buffer
            .as_ref()
            .and_then(|binding| binding.fnv1a64.clone())
        {
            entry.index_hashes.insert(hash);
        }
        entry.draw_identities.insert(format!(
            "{}:{}:{}:{}:{:?}:{:?}:{:?}",
            chunk.draw.draw_call,
            chunk.draw.draw_count,
            chunk.draw.draw_start,
            chunk.draw.base_vertex_location,
            chunk.draw.per_instance_count,
            chunk.draw.instance_count,
            chunk.draw.start_instance_location
        ));
    }
    let source_world_rect = library.world_rect();
    for source in sources.values_mut() {
        source.slot1_vertex_tile_stride_bytes =
            terrain_chunk_lod_slot1_vertex_tile_stride_bytes(source)?;
        terrain_chunk_validate_lod_slot1_vertex_offsets(source_world_rect, source)?;
        source.validate()?;
    }
    Ok(sources)
}

pub(crate) fn terrain_chunk_synthesis_record_samples(
    library: &NativeD3d11TerrainChunkLibrary,
) -> Result<Vec<&TerrainChunkRecord>> {
    let lods = terrain_chunk_synthesis_lod_sources(library)?;
    let mut out = Vec::new();
    for source in lods.values() {
        let mut matching = library
            .manifest
            .chunks
            .iter()
            .filter(|chunk| terrain_chunk_world_tile_size(chunk).ok() == Some(source.tile_size))
            .collect::<Vec<_>>();
        matching.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));
        let sample = matching
            .iter()
            .copied()
            .find(|chunk| chunk.chunk_id != source.template.chunk_id)
            .or_else(|| matching.first().copied())
            .ok_or_else(|| {
                format!(
                    "production terrain chunk synthesis blocked: no validation sample for {} world-unit chunks",
                    source.tile_size
                )
            })?;
        out.push(sample);
    }
    if out.len() < 3 {
        return Err(format!(
            "production terrain chunk synthesis blocked: byte-exact gate requires at least 3 LOD samples, found {}",
            out.len()
        )
        .into());
    }
    Ok(out)
}

pub(crate) fn terrain_chunk_draw_quad_offset_scale(
    draw: &MapFrameDraw,
    bundle_dir: &std::path::Path,
) -> Result<[f32; 4]> {
    let binding = draw
        .constant_buffers
        .iter()
        .find(|binding| binding.stage == "VS" && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT)
        .ok_or("production terrain chunk synthesis blocked: draw has no VS terrain draw CB1")?;
    let bytes = read_map_frame_buffer_bytes(bundle_dir, binding, "constant buffer")?;
    terrain_chunk_cb1_quad_offset_scale_from_bytes(&bytes)
}

pub(crate) fn terrain_chunk_draw_detail_edge_flags(
    draw: &MapFrameDraw,
    bundle_dir: &std::path::Path,
) -> Result<[f32; 4]> {
    let binding = draw
        .constant_buffers
        .iter()
        .find(|binding| binding.stage == "VS" && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT)
        .ok_or("production terrain chunk synthesis blocked: draw has no VS terrain draw CB1")?;
    let bytes = read_map_frame_buffer_bytes(bundle_dir, binding, "constant buffer")?;
    terrain_chunk_cb1_detail_edge_flags_from_bytes(&bytes)
}

pub(crate) fn terrain_chunk_cb1_quad_offset_scale_from_bytes(bytes: &[u8]) -> Result<[f32; 4]> {
    let offset = TERRAIN_DRAW_CB1_QUAD_OFFSET_SCALE_BYTE_OFFSET;
    if bytes.len() < offset + 16 {
        return Err(format!(
            "production terrain chunk synthesis blocked: terrain draw CB1 has {} bytes, need at least {} for QuadOffset_Scale_IsDetail",
            bytes.len(),
            offset + 16
        )
        .into());
    }
    Ok([
        read_f32_le_at(bytes, offset)?,
        read_f32_le_at(bytes, offset + 4)?,
        read_f32_le_at(bytes, offset + 8)?,
        read_f32_le_at(bytes, offset + 12)?,
    ])
}

pub(crate) fn terrain_chunk_cb1_detail_edge_flags_from_bytes(bytes: &[u8]) -> Result<[f32; 4]> {
    let offset = TERRAIN_DRAW_CB1_DETAIL_EDGE_FLAGS_BYTE_OFFSET;
    if bytes.len() < offset + 16 {
        return Err(format!(
            "production terrain chunk synthesis blocked: terrain draw CB1 has {} bytes, need at least {} for detail edge flags",
            bytes.len(),
            offset + 16
        )
        .into());
    }
    Ok([
        read_f32_le_at(bytes, offset)?,
        read_f32_le_at(bytes, offset + 4)?,
        read_f32_le_at(bytes, offset + 8)?,
        read_f32_le_at(bytes, offset + 12)?,
    ])
}

pub(crate) fn write_terrain_chunk_cb1_quad_offset_scale(
    bytes: &mut [u8],
    quad_offset_scale_is_detail: [f32; 4],
) -> Result<()> {
    let offset = TERRAIN_DRAW_CB1_QUAD_OFFSET_SCALE_BYTE_OFFSET;
    if bytes.len() < offset + 16 {
        return Err(format!(
            "production terrain chunk synthesis blocked: terrain draw CB1 has {} bytes, need at least {} for QuadOffset_Scale_IsDetail",
            bytes.len(),
            offset + 16
        )
        .into());
    }
    for (index, value) in quad_offset_scale_is_detail.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(
                "production terrain chunk synthesis blocked: CB1 QuadOffset_Scale_IsDetail contains a non-finite value"
                    .into(),
            );
        }
        let start = offset + index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

pub(crate) fn write_terrain_chunk_cb1_detail_edge_flags(
    bytes: &mut [u8],
    detail_edge_flags: [f32; 4],
) -> Result<()> {
    let offset = TERRAIN_DRAW_CB1_DETAIL_EDGE_FLAGS_BYTE_OFFSET;
    if bytes.len() < offset + 16 {
        return Err(format!(
            "production terrain chunk synthesis blocked: terrain draw CB1 has {} bytes, need at least {} for detail edge flags",
            bytes.len(),
            offset + 16
        )
        .into());
    }
    for (index, value) in detail_edge_flags.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(
                "production terrain chunk synthesis blocked: CB1 detail edge flags contain a non-finite value"
                    .into(),
            );
        }
        bytes[offset + index * 4..offset + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

pub(crate) fn terrain_chunk_synthesis_expected_tile_count(
    world_rect: TerrainChunkSynthesisWorldRect,
    target_tile_size: u32,
) -> Result<usize> {
    terrain_chunk_synthesis_grid_dims(world_rect, target_tile_size)
        .and_then(|(x, z)| usize::try_from(x.saturating_mul(z)).map_err(Into::into))
}

pub(crate) fn terrain_chunk_synthesis_elapsed_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Clone, Debug)]
pub(crate) struct TerrainChunkLodSource<'a> {
    pub(crate) tile_size: u32,
    pub(crate) cb1_scale: f32,
    pub(crate) template: &'a TerrainChunkRecord,
    pub(crate) record_count: usize,
    pub(crate) slot0_vertex_hashes: BTreeSet<String>,
    pub(crate) slot1_vertex_hashes: BTreeSet<String>,
    pub(crate) index_hashes: BTreeSet<String>,
    pub(crate) draw_identities: BTreeSet<String>,
    pub(crate) slot1_vertex_offsets: Vec<TerrainChunkSlot1OffsetRecord>,
    pub(crate) slot1_vertex_tile_stride_bytes: u32,
    pub(crate) cb1_detail_edge_flags_by_origin: BTreeMap<(i32, i32), [f32; 4]>,
}

#[derive(Clone, Debug)]
pub(crate) struct TerrainChunkSlot1OffsetRecord {
    pub(crate) chunk_id: String,
    pub(crate) min_x: f32,
    pub(crate) min_z: f32,
    pub(crate) offset: u32,
}

impl TerrainChunkLodSource<'_> {
    fn validate(&self) -> Result<()> {
        if self.record_count == 0 {
            return Err(format!(
                "production terrain chunk synthesis blocked: LOD {} has no captured records",
                self.tile_size
            )
            .into());
        }
        if self.slot1_vertex_hashes.len() != 1 {
            return Err(format!(
                "production terrain chunk synthesis blocked: LOD {} has {} slot-1 vertex-buffer payload variants; expected exactly one",
                self.tile_size,
                self.slot1_vertex_hashes.len()
            )
            .into());
        }
        if self.index_hashes.len() != 1 {
            return Err(format!(
                "production terrain chunk synthesis blocked: LOD {} has {} index-buffer payload variants; expected exactly one",
                self.tile_size,
                self.index_hashes.len()
            )
            .into());
        }
        if self.draw_identities.len() != 1 {
            return Err(format!(
                "production terrain chunk synthesis blocked: LOD {} has inconsistent draw arguments",
                self.tile_size
            )
            .into());
        }
        Ok(())
    }

    pub(crate) fn mapping(&self) -> TerrainChunkSynthesisLodMapping {
        TerrainChunkSynthesisLodMapping {
            tile_size: self.tile_size,
            cb1_scale: self.cb1_scale,
            template_chunk_id: self.template.chunk_id.clone(),
            draw_call: self.template.draw.draw_call.clone(),
            draw_count: self.template.draw.draw_count,
            shared_slot0_vertex_fnv1a64: self.slot0_vertex_hashes.iter().next().cloned(),
            lod_slot1_vertex_fnv1a64: self.slot1_vertex_hashes.iter().next().cloned(),
            slot1_vertex_tile_stride_bytes: self.slot1_vertex_tile_stride_bytes,
            index_fnv1a64: self.index_hashes.iter().next().cloned(),
            captured_record_count: self.record_count,
        }
    }
}

fn terrain_chunk_lod_slot1_vertex_tile_stride_bytes(
    source: &TerrainChunkLodSource<'_>,
) -> Result<u32> {
    let slot1 = source.template.draw.vertex_buffers.get("1").ok_or_else(|| {
        format!(
            "production terrain chunk synthesis blocked: LOD {} template has no slot-1 vertex buffer",
            source.tile_size
        )
    })?;
    let slot1_stride = slot1.stride.ok_or_else(|| {
        format!(
            "production terrain chunk synthesis blocked: LOD {} template slot-1 vertex buffer has no stride",
            source.tile_size
        )
    })?;
    let quads = source
        .template
        .draw
        .draw_count
        .checked_div(6)
        .ok_or("draw_count division by zero")?;
    if quads.saturating_mul(6) != source.template.draw.draw_count {
        return Err(format!(
            "production terrain chunk synthesis blocked: LOD {} draw_count {} is not a six-index quad grid",
            source.tile_size, source.template.draw.draw_count
        )
        .into());
    }
    let side = (f64::from(quads).sqrt().round()) as u32;
    if side.saturating_mul(side) != quads {
        return Err(format!(
            "production terrain chunk synthesis blocked: LOD {} draw_count {} is not a square quad grid",
            source.tile_size, source.template.draw.draw_count
        )
        .into());
    }
    let vertices_per_side = side
        .checked_add(1)
        .ok_or("terrain chunk vertex side overflow")?;
    let expected_stride = vertices_per_side
        .checked_mul(vertices_per_side)
        .and_then(|vertices| vertices.checked_mul(slot1_stride))
        .ok_or("terrain chunk slot-1 vertex offset stride overflow")?;
    let mut observed_stride = 0;
    let mut offsets = source
        .slot1_vertex_offsets
        .iter()
        .map(|record| record.offset)
        .collect::<Vec<_>>();
    offsets.sort_unstable();
    offsets.dedup();
    for pair in offsets.windows(2) {
        let delta = pair[1] - pair[0];
        if delta > 0 {
            observed_stride = gcd_u32(observed_stride, delta);
        }
    }
    if observed_stride != 0 && observed_stride != expected_stride {
        return Err(format!(
            "production terrain chunk synthesis blocked: LOD {} slot-1 vertex offset stride {} does not match template grid stride {}",
            source.tile_size, observed_stride, expected_stride
        )
        .into());
    }
    Ok(expected_stride)
}

fn terrain_chunk_validate_lod_slot1_vertex_offsets(
    source_world_rect: TerrainChunkSynthesisWorldRect,
    source: &TerrainChunkLodSource<'_>,
) -> Result<()> {
    let (source_tiles_x, _) =
        terrain_chunk_synthesis_grid_dims(source_world_rect, source.tile_size)?;
    for record in &source.slot1_vertex_offsets {
        let expected = terrain_chunk_slot1_vertex_offset_for_world_tile(
            source_world_rect,
            source.tile_size,
            source_tiles_x,
            source.slot1_vertex_tile_stride_bytes,
            record.min_x,
            record.min_z,
        )?;
        if record.offset != expected {
            return Err(format!(
                "production terrain chunk synthesis blocked: chunk {} slot-1 vertex offset {} does not match synthesized offset {}",
                record.chunk_id, record.offset, expected
            )
            .into());
        }
    }
    Ok(())
}

fn terrain_chunk_slot1_vertex_offset_for_world_tile(
    source_world_rect: TerrainChunkSynthesisWorldRect,
    target_tile_size: u32,
    source_tiles_x: u32,
    stride_bytes: u32,
    min_x: f32,
    min_z: f32,
) -> Result<u32> {
    let step = target_tile_size as f32;
    let tile_x = exact_tile_index(min_x - source_world_rect.min_x, step, "x")?;
    let tile_z = exact_tile_index(min_z - source_world_rect.min_z, step, "z")?;
    let linear = u64::from(tile_z)
        .checked_mul(u64::from(source_tiles_x))
        .and_then(|base| base.checked_add(u64::from(tile_x)))
        .ok_or("terrain chunk slot-1 vertex offset index overflow")?;
    let offset = linear
        .checked_mul(u64::from(stride_bytes))
        .ok_or("terrain chunk slot-1 vertex offset overflow")?;
    if offset > u64::from(u32::MAX) {
        return Err("terrain chunk slot-1 vertex offset exceeds u32".into());
    }
    Ok(offset as u32)
}

fn terrain_chunk_origin_key(min_x: f32, min_z: f32) -> Result<(i32, i32)> {
    if !min_x.is_finite() || !min_z.is_finite() {
        return Err("production terrain chunk synthesis blocked: non-finite tile origin".into());
    }
    let rounded_x = min_x.round();
    let rounded_z = min_z.round();
    if (min_x - rounded_x).abs() > f32::EPSILON
        || (min_z - rounded_z).abs() > f32::EPSILON
        || rounded_x < i32::MIN as f32
        || rounded_x > i32::MAX as f32
        || rounded_z < i32::MIN as f32
        || rounded_z > i32::MAX as f32
    {
        return Err(format!(
            "production terrain chunk synthesis blocked: tile origin {min_x},{min_z} is not integer-aligned"
        )
        .into());
    }
    Ok((rounded_x as i32, rounded_z as i32))
}

fn exact_tile_index(offset: f32, step: f32, axis: &str) -> Result<u32> {
    let index = offset / step;
    let rounded = index.round();
    if (index - rounded).abs() > f32::EPSILON || rounded < 0.0 || rounded > u32::MAX as f32 {
        return Err(format!(
            "production terrain chunk synthesis blocked: tile {axis} offset {offset} is not aligned to target tile size {step}"
        )
        .into());
    }
    Ok(rounded as u32)
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

pub(crate) fn terrain_chunk_world_tile_size(chunk: &TerrainChunkRecord) -> Result<u32> {
    let width = chunk.bounds.max_x - chunk.bounds.min_x;
    let height = chunk.bounds.max_z - chunk.bounds.min_z;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(format!(
            "production terrain chunk synthesis blocked: chunk {} has invalid bounds",
            chunk.chunk_id
        )
        .into());
    }
    if (width - height).abs() > f32::EPSILON {
        return Err(format!(
            "production terrain chunk synthesis blocked: chunk {} is not square: {}x{}",
            chunk.chunk_id, width, height
        )
        .into());
    }
    let rounded = width.round();
    if (width - rounded).abs() > f32::EPSILON || rounded <= 0.0 || rounded > u32::MAX as f32 {
        return Err(format!(
            "production terrain chunk synthesis blocked: chunk {} tile size {width} is not an integer world-unit size",
            chunk.chunk_id
        )
        .into());
    }
    Ok(rounded as u32)
}

fn terrain_chunk_validate_cb1_matches_bounds(
    chunk: &TerrainChunkRecord,
    cb1: [f32; 4],
    tile_size: u32,
) -> Result<()> {
    if cb1.iter().any(|value| !value.is_finite()) || cb1[2] <= 0.0 {
        return Err(format!(
            "production terrain chunk synthesis blocked: chunk {} CB1 QuadOffset_Scale_IsDetail is invalid",
            chunk.chunk_id
        )
        .into());
    }
    let expected = [chunk.bounds.min_x, chunk.bounds.min_z];
    if (cb1[0] - expected[0]).abs() > f32::EPSILON || (cb1[1] - expected[1]).abs() > f32::EPSILON {
        return Err(format!(
            "production terrain chunk synthesis blocked: chunk {} CB1 offset {},{} does not match bounds {},{}",
            chunk.chunk_id, cb1[0], cb1[1], expected[0], expected[1]
        )
        .into());
    }
    let template_extent = f64::from(tile_size) / f64::from(cb1[2]);
    if (template_extent - template_extent.round()).abs() > f64::EPSILON {
        return Err(format!(
            "production terrain chunk synthesis blocked: chunk {} tile size {tile_size} is not an integer multiple of CB1 scale {}",
            chunk.chunk_id, cb1[2]
        )
        .into());
    }
    Ok(())
}

fn terrain_chunk_synthesis_tiles(
    world_rect: TerrainChunkSynthesisWorldRect,
    source_world_rect: TerrainChunkSynthesisWorldRect,
    target_tile_size: u32,
    cb1_scale: f32,
    slot1_vertex_tile_stride_bytes: u32,
    captured_detail_edge_flags_by_origin: Option<&BTreeMap<(i32, i32), [f32; 4]>>,
) -> Result<Vec<TerrainChunkSynthesisTile>> {
    let (tiles_x, tiles_z) = terrain_chunk_synthesis_grid_dims(world_rect, target_tile_size)?;
    let (source_tiles_x, _) =
        terrain_chunk_synthesis_grid_dims(source_world_rect, target_tile_size)?;
    let mut tiles = Vec::with_capacity(usize::try_from(tiles_x.saturating_mul(tiles_z))?);
    let step = target_tile_size as f32;
    for tile_z in 0..tiles_z {
        for tile_x in 0..tiles_x {
            let min_x = world_rect.min_x + tile_x as f32 * step;
            let min_z = world_rect.min_z + tile_z as f32 * step;
            let max_x = (min_x + step).min(world_rect.max_x);
            let max_z = (min_z + step).min(world_rect.max_z);
            let slot1_vertex_offset = terrain_chunk_slot1_vertex_offset_for_world_tile(
                source_world_rect,
                target_tile_size,
                source_tiles_x,
                slot1_vertex_tile_stride_bytes,
                min_x,
                min_z,
            )?;
            let origin_key = terrain_chunk_origin_key(min_x, min_z)?;
            let detail_edge_flags = captured_detail_edge_flags_by_origin
                .and_then(|flags| flags.get(&origin_key))
                .copied()
                .unwrap_or([0.0, 0.0, 0.0, 0.0]);
            tiles.push(TerrainChunkSynthesisTile {
                tile_x,
                tile_z,
                min_x,
                min_z,
                max_x,
                max_z,
                quad_offset_scale_is_detail: [min_x, min_z, cb1_scale, 0.0],
                detail_edge_flags,
                slot1_vertex_offset,
            });
        }
    }
    Ok(tiles)
}

fn terrain_chunk_synthesis_grid_dims(
    world_rect: TerrainChunkSynthesisWorldRect,
    target_tile_size: u32,
) -> Result<(u32, u32)> {
    world_rect.validate()?;
    if target_tile_size == 0 {
        return Err(
            "production terrain chunk synthesis blocked: target LOD tile size must be positive"
                .into(),
        );
    }
    let step = target_tile_size as f32;
    let width = world_rect.width();
    let height = world_rect.height();
    let tiles_x = exact_tile_count(width, step, "width")?;
    let tiles_z = exact_tile_count(height, step, "height")?;
    Ok((tiles_x, tiles_z))
}

fn exact_tile_count(length: f32, step: f32, axis: &str) -> Result<u32> {
    let count = length / step;
    let rounded = count.round();
    if (count - rounded).abs() > f32::EPSILON || rounded <= 0.0 || rounded > u32::MAX as f32 {
        return Err(format!(
            "production terrain chunk synthesis blocked: world rect {axis} {length} is not divisible by target tile size {step}"
        )
        .into());
    }
    Ok(rounded as u32)
}

fn read_f32_le_at(bytes: &[u8], offset: usize) -> Result<f32> {
    let raw = bytes.get(offset..offset + 4).ok_or_else(|| {
        format!("production terrain chunk synthesis blocked: missing f32 bytes at {offset}")
    })?;
    Ok(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::fs;

    fn write_cb1(root: &std::path::Path, file: &str, values: [f32; 4]) -> String {
        let mut bytes = test_terrain_draw_cb1_bytes();
        write_terrain_chunk_cb1_quad_offset_scale(&mut bytes, values).expect("patch cb1");
        fs::write(root.join(file), &bytes).expect("write cb1");
        fnv1a64_file(&root.join(file)).expect("hash cb1")
    }

    fn synthesis_library(root_name: &str) -> NativeD3d11TerrainChunkLibrary {
        let root = temp_test_dir(root_name);
        fs::create_dir_all(&root).expect("create root");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                let mut state = valid_map_frame_draw_state();
                state.output_merger = Some(valid_output_merger_state());
                draw.draw_state = Some(state);
            }
            let mut draws = Vec::new();
            for (index, (tile_size, cb_scale, min_x, min_z)) in [
                (128.0f32, 2.0f32, 0.0f32, 0.0f32),
                (256.0, 4.0, 0.0, 256.0),
                (512.0, 8.0, 512.0, 0.0),
            ]
            .iter()
            .copied()
            .enumerate()
            {
                let mut draw = frame.draws[index % frame.draws.len()].clone();
                draw.draw_index = index as u32;
                draw.draw_count = 24_576;
                let source_tiles_x = (1024.0 / tile_size) as u32;
                let tile_x = (min_x / tile_size) as u32;
                let tile_z = (min_z / tile_size) as u32;
                let slot1_offset = (tile_z * source_tiles_x + tile_x) * 16_900;
                draw.vertex_buffers
                    .get_mut("1")
                    .expect("slot-1 vertex buffer")
                    .offset = Some(slot1_offset);
                let cb1_file = format!("synthesis_cb1_{index}.bin");
                let cb1_hash = write_cb1(&root, &cb1_file, [min_x, min_z, cb_scale, 0.0]);
                let cb1 = draw
                    .constant_buffers
                    .iter_mut()
                    .find(|binding| {
                        binding.stage == "VS" && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT
                    })
                    .expect("cb1");
                cb1.file = Some(cb1_file);
                cb1.fnv1a64 = Some(cb1_hash);
                draws.push((
                    TerrainChunkBounds {
                        min_x,
                        min_y: 0.0,
                        min_z,
                        max_x: min_x + tile_size,
                        max_y: 0.0,
                        max_z: min_z + tile_size,
                    },
                    draw,
                ));
            }
            frame.active_draw = draws[0].1.clone();
            frame.draws = draws.iter().map(|(_, draw)| draw.clone()).collect();
            frame
        });
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(path).expect("read bundle")).expect("frame");
        let chunks = frame
            .draws
            .iter()
            .enumerate()
            .map(|(index, draw)| TerrainChunkRecord {
                chunk_id: format!("chunk_{index}"),
                identity: terrain_chunk_identity_from_draw(draw),
                bounds: match index {
                    0 => TerrainChunkBounds {
                        min_x: 0.0,
                        min_y: 0.0,
                        min_z: 0.0,
                        max_x: 128.0,
                        max_y: 0.0,
                        max_z: 128.0,
                    },
                    1 => TerrainChunkBounds {
                        min_x: 0.0,
                        min_y: 0.0,
                        min_z: 256.0,
                        max_x: 256.0,
                        max_y: 0.0,
                        max_z: 512.0,
                    },
                    _ => TerrainChunkBounds {
                        min_x: 512.0,
                        min_y: 0.0,
                        min_z: 0.0,
                        max_x: 1024.0,
                        max_y: 0.0,
                        max_z: 512.0,
                    },
                },
                draw: draw.clone(),
            })
            .collect::<Vec<_>>();
        let bounds = chunks.iter().map(|chunk| chunk.bounds).collect::<Vec<_>>();
        let manifest = TerrainChunkLibraryManifest {
            schema: TERRAIN_CHUNK_LIBRARY_SCHEMA.to_string(),
            provenance: "unit_test".to_string(),
            capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT.to_string(),
            session_manifest: MAP_CAPTURE_SESSION_FILE.to_string(),
            shader_cache_constants: frame.shader_cache_constants.clone(),
            chunks,
            vanilla_sources: frame.vanilla_sources.clone(),
            runtime_files: frame.runtime_files.clone(),
            coverage: coverage_for_chunk_bounds(
                &bounds,
                TerrainChunkWorldRect {
                    min_x: 0.0,
                    min_z: 0.0,
                    max_x: 1024.0,
                    max_z: 512.0,
                },
            ),
            blockers: Vec::new(),
        };
        fs::write(
            root.join(TERRAIN_CHUNK_LIBRARY_FILE),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        NativeD3d11TerrainChunkLibrary::load(&root).expect("load library")
    }

    #[test]
    fn synthesis_plan_maps_lod_templates_and_tiles_full_grid() {
        let library = synthesis_library("terrain_chunk_synthesis_plan");
        let plan = terrain_chunk_synthesis_plan(
            &library,
            128,
            TerrainChunkSynthesisWorldRect::new(0.0, 0.0, 1024.0, 512.0).expect("world"),
        )
        .expect("plan");
        assert_eq!(plan.tile_count(), 32);
        assert_eq!(
            terrain_chunk_synthesis_expected_tile_count(plan.world_rect(), 128).expect("count"),
            32
        );
        assert_eq!(plan.cb1_scale(), 2.0);
        let mappings = plan.lod_mappings();
        assert_eq!(mappings.len(), 3);
        assert_eq!(mappings[0].tile_size, 128);
        assert_eq!(mappings[1].tile_size, 256);
        assert_eq!(mappings[2].tile_size, 512);
    }

    #[test]
    fn synthesis_plan_rejects_missing_lod() {
        let library = synthesis_library("terrain_chunk_synthesis_missing_lod");
        let err = terrain_chunk_synthesis_plan(
            &library,
            64,
            TerrainChunkSynthesisWorldRect::new(0.0, 0.0, 1024.0, 512.0).expect("world"),
        )
        .expect_err("missing LOD must fail");
        assert!(
            err.to_string().contains("no captured template for 64"),
            "{err}"
        );
    }
}
