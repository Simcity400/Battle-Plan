use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TerrainChunkBounds {
    pub(crate) min_x: f32,
    pub(crate) min_y: f32,
    pub(crate) min_z: f32,
    pub(crate) max_x: f32,
    pub(crate) max_y: f32,
    pub(crate) max_z: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TerrainChunkIdentity {
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    #[serde(default)]
    pub(crate) per_instance_count: Option<u32>,
    #[serde(default)]
    pub(crate) instance_count: Option<u32>,
    #[serde(default)]
    pub(crate) start_instance_location: Option<u32>,
    pub(crate) pixel_shader_fnv1a64: String,
    pub(crate) vertex_shader_fnv1a64: String,
    #[serde(default)]
    pub(crate) vertex_payloads: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) index_payload: Option<String>,
    #[serde(default)]
    pub(crate) per_chunk_constant_buffers: Vec<TerrainChunkConstantBufferIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerrainChunkConstantBufferIdentity {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    #[serde(default)]
    pub(crate) declared_name: Option<String>,
    pub(crate) fnv1a64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainChunkRecord {
    pub(crate) chunk_id: String,
    pub(crate) identity: TerrainChunkIdentity,
    pub(crate) bounds: TerrainChunkBounds,
    pub(crate) draw: MapFrameDraw,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TerrainChunkCoverage {
    pub(crate) world_rect: TerrainChunkWorldRect,
    pub(crate) covered_area: f64,
    pub(crate) world_area: f64,
    pub(crate) fraction: f64,
    #[serde(default)]
    pub(crate) holes: Vec<TerrainChunkHole>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TerrainChunkWorldRect {
    pub(crate) min_x: f32,
    pub(crate) min_z: f32,
    pub(crate) max_x: f32,
    pub(crate) max_z: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerrainChunkHole {
    pub(crate) min_x: f32,
    pub(crate) min_z: f32,
    pub(crate) max_x: f32,
    pub(crate) max_z: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainChunkLibraryManifest {
    pub(crate) schema: String,
    pub(crate) provenance: String,
    pub(crate) capture_moment: String,
    pub(crate) session_manifest: String,
    #[serde(default)]
    pub(crate) shader_cache_constants: Option<MapFrameShaderCacheConstants>,
    #[serde(default)]
    pub(crate) chunks: Vec<TerrainChunkRecord>,
    #[serde(default)]
    pub(crate) vanilla_sources: BTreeMap<String, MapFrameFileEvidence>,
    #[serde(default)]
    pub(crate) runtime_files: BTreeMap<String, MapFrameFileEvidence>,
    pub(crate) coverage: TerrainChunkCoverage,
    #[serde(default)]
    pub(crate) blockers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct RuntimeTextureCaptureSidecar {
    schema: String,
    file: String,
    width: u32,
    height: u32,
    byte_len: usize,
    fnv1a64: String,
    #[serde(default)]
    source_dxgi_format: Option<u32>,
    #[serde(default)]
    superseded_hash: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NativeD3d11TerrainChunkLibrary {
    pub(crate) manifest_path: PathBuf,
    pub(crate) bundle_dir: PathBuf,
    pub(crate) manifest: TerrainChunkLibraryManifest,
}

impl NativeD3d11TerrainChunkLibrary {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let manifest_path = terrain_chunk_library_manifest_path(path.as_ref());
        let bundle_dir = manifest_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "production terrain chunk library blocked: {} has no parent directory",
                    manifest_path.display()
                )
            })?
            .to_path_buf();
        let mut manifest: TerrainChunkLibraryManifest =
            serde_json::from_slice(&fs::read(&manifest_path)?)?;
        reconcile_terrain_chunk_runtime_texture_sidecars(&mut manifest, &bundle_dir)?;
        validate_terrain_chunk_library_manifest(&manifest, &bundle_dir)?;
        Ok(Self {
            manifest_path,
            bundle_dir,
            manifest,
        })
    }

    /// Load only the manifest's chunk-draw records and coverage, WITHOUT the
    /// full runtime-texture/shader sidecar reconciliation the render path needs.
    /// The mesh-law corpus gate only touches the captured VB/IB/CB buffer files
    /// and the shipped heightmap, so it does not require the texture sidecars —
    /// this lets it run against lightweight quick-capture harness runs that omit
    /// them. The manifest schema is still checked.
    pub fn load_records_only(path: impl AsRef<Path>) -> Result<Self> {
        let manifest_path = terrain_chunk_library_manifest_path(path.as_ref());
        let bundle_dir = manifest_path
            .parent()
            .ok_or_else(|| {
                format!(
                    "production terrain chunk library blocked: {} has no parent directory",
                    manifest_path.display()
                )
            })?
            .to_path_buf();
        let manifest: TerrainChunkLibraryManifest =
            serde_json::from_slice(&fs::read(&manifest_path)?)?;
        if manifest.schema != TERRAIN_CHUNK_LIBRARY_SCHEMA {
            return Err(format!(
                "production terrain chunk library blocked: manifest schema {:?} != {:?}",
                manifest.schema, TERRAIN_CHUNK_LIBRARY_SCHEMA
            )
            .into());
        }
        Ok(Self {
            manifest_path,
            bundle_dir,
            manifest,
        })
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn bundle_dir(&self) -> &Path {
        &self.bundle_dir
    }

    pub fn chunk_count(&self) -> usize {
        self.manifest.chunks.len()
    }

    /// The captured chunk-draw records, in manifest order. Used by the mesh-law
    /// corpus validator to regenerate and byte-compare every captured draw.
    pub(crate) fn chunk_records(&self) -> &[TerrainChunkRecord] {
        &self.manifest.chunks
    }

    pub fn coverage_fraction(&self) -> f64 {
        self.manifest.coverage.fraction
    }

    pub fn holes(&self) -> &[TerrainChunkHole] {
        &self.manifest.coverage.holes
    }

    pub fn world_rect(&self) -> TerrainChunkSynthesisWorldRect {
        TerrainChunkSynthesisWorldRect::from(self.manifest.coverage.world_rect)
    }

    pub fn production_camera_from_capture(&self) -> Result<ProductionMapCamera> {
        let record = self.manifest.chunks.first().ok_or(
            "production terrain chunk library blocked: library has no camera source records",
        )?;
        production_map_camera_from_scene_cb0(decode_map_frame_terrain_scene_cb0(
            &record.draw,
            self.bundle_dir(),
        )?)
    }

    pub fn camera_defines(&self) -> Result<Hoi4MapCameraDefines> {
        let evidence = self
            .manifest
            .vanilla_sources
            .get(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE)
            .ok_or_else(|| {
                format!(
                    "production terrain chunk library blocked: chunk library is missing vanilla graphics defines evidence for {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE}"
                )
            })?;
        let bytes = read_verified_map_frame_file(
            self.bundle_dir(),
            &evidence.file,
            evidence.byte_len,
            &evidence.fnv1a64,
            "graphics defines",
        )?;
        hoi4_map_camera_defines_from_source(&latin1_bytes_to_string(&bytes))
    }

    pub fn visible_chunk_ids(&self, camera: ProductionMapCamera) -> Vec<String> {
        self.manifest
            .chunks
            .iter()
            .filter(|chunk| terrain_chunk_bounds_intersects_camera(chunk.bounds, camera))
            .map(|chunk| chunk.chunk_id.clone())
            .collect()
    }

    pub(crate) fn frame_for_camera(
        &self,
        camera: ProductionMapCamera,
    ) -> Result<MapFrameRuntimeBundle> {
        let draws = self
            .manifest
            .chunks
            .iter()
            .filter(|chunk| terrain_chunk_bounds_intersects_camera(chunk.bounds, camera))
            .map(|chunk| chunk.draw.clone())
            .collect::<Vec<_>>();
        terrain_chunk_frame_from_draws(&self.manifest, draws)
    }

    pub(crate) fn target_evidence_frame(&self) -> Result<MapFrameRuntimeBundle> {
        let draw = self
            .manifest
            .chunks
            .first()
            .ok_or(
                "production terrain chunk library blocked: library has no target evidence records",
            )?
            .draw
            .clone();
        terrain_chunk_frame_from_draws(&self.manifest, vec![draw])
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn frame_for_chunk_ids(
        &self,
        chunk_ids: &BTreeSet<String>,
    ) -> Result<MapFrameRuntimeBundle> {
        let draws = self
            .manifest
            .chunks
            .iter()
            .filter(|chunk| chunk_ids.contains(&chunk.chunk_id))
            .map(|chunk| chunk.draw.clone())
            .collect::<Vec<_>>();
        terrain_chunk_frame_from_draws(&self.manifest, draws)
    }
}

pub(crate) fn terrain_chunk_library_manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(TERRAIN_CHUNK_LIBRARY_FILE)
    } else {
        path.to_path_buf()
    }
}

fn reconcile_terrain_chunk_runtime_texture_sidecars(
    manifest: &mut TerrainChunkLibraryManifest,
    bundle_dir: &Path,
) -> Result<()> {
    for entry in fs::read_dir(bundle_dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with("_capture.json") {
            continue;
        }
        let sidecar: RuntimeTextureCaptureSidecar = match serde_json::from_slice(&fs::read(&path)?)
        {
            Ok(sidecar) => sidecar,
            Err(_) => continue,
        };
        if sidecar.schema != "battle-plan-runtime-texture-capture-v1" {
            continue;
        }
        let Some(superseded_hash) = sidecar.superseded_hash.as_deref() else {
            continue;
        };
        let referenced_count =
            terrain_chunk_sidecar_reference_count(manifest, &sidecar.file, superseded_hash);
        if referenced_count == 0 {
            continue;
        }
        validate_terrain_chunk_runtime_texture_sidecar(&sidecar, bundle_dir, superseded_hash)?;
        for chunk in &mut manifest.chunks {
            for resource in &mut chunk.draw.shader_resources {
                if resource.file.as_deref() == Some(sidecar.file.as_str())
                    && resource.fnv1a64.as_deref() == Some(superseded_hash)
                {
                    validate_terrain_chunk_sidecar_resource(resource, &sidecar)?;
                    resource.fnv1a64 = Some(sidecar.fnv1a64.clone());
                }
            }
        }
        for evidence in manifest.runtime_files.values_mut() {
            if evidence.file == sidecar.file && evidence.fnv1a64 == superseded_hash {
                if evidence.byte_len != sidecar.byte_len {
                    return Err(format!(
                        "production terrain chunk library blocked: runtime texture sidecar {} byte_len {} does not match runtime file evidence {}",
                        path.display(),
                        sidecar.byte_len,
                        evidence.byte_len
                    )
                    .into());
                }
                evidence.fnv1a64 = sidecar.fnv1a64.clone();
            }
        }
    }
    Ok(())
}

fn terrain_chunk_sidecar_reference_count(
    manifest: &TerrainChunkLibraryManifest,
    file: &str,
    superseded_hash: &str,
) -> usize {
    let resource_count = manifest
        .chunks
        .iter()
        .flat_map(|chunk| chunk.draw.shader_resources.iter())
        .filter(|resource| {
            resource.file.as_deref() == Some(file)
                && resource.fnv1a64.as_deref() == Some(superseded_hash)
        })
        .count();
    let runtime_file_count = manifest
        .runtime_files
        .values()
        .filter(|evidence| evidence.file == file && evidence.fnv1a64 == superseded_hash)
        .count();
    resource_count + runtime_file_count
}

fn validate_terrain_chunk_runtime_texture_sidecar(
    sidecar: &RuntimeTextureCaptureSidecar,
    bundle_dir: &Path,
    superseded_hash: &str,
) -> Result<()> {
    if sidecar.file.trim().is_empty() || sidecar.fnv1a64.trim().is_empty() {
        return Err(
            "production terrain chunk library blocked: runtime texture sidecar has empty file or hash"
                .into(),
        );
    }
    if sidecar.fnv1a64 == superseded_hash {
        return Err(format!(
            "production terrain chunk library blocked: runtime texture sidecar {} does not replace superseded hash {}",
            sidecar.file, superseded_hash
        )
        .into());
    }
    let path = bundle_dir.join(&sidecar.file);
    let metadata = fs::metadata(&path)?;
    if metadata.len() != sidecar.byte_len as u64 {
        return Err(format!(
            "production terrain chunk library blocked: runtime texture sidecar {} byte_len {} does not match file byte_len {}",
            sidecar.file,
            sidecar.byte_len,
            metadata.len()
        )
        .into());
    }
    let actual_hash = fnv1a64_file(&path)?;
    if actual_hash != sidecar.fnv1a64 {
        return Err(format!(
            "production terrain chunk library blocked: runtime texture sidecar {} hash changed from {} to {}",
            sidecar.file, sidecar.fnv1a64, actual_hash
        )
        .into());
    }
    Ok(())
}

fn validate_terrain_chunk_sidecar_resource(
    resource: &MapFrameTextureBinding,
    sidecar: &RuntimeTextureCaptureSidecar,
) -> Result<()> {
    if resource.byte_len != sidecar.byte_len
        || resource.width != sidecar.width
        || resource.height != sidecar.height
        || sidecar
            .source_dxgi_format
            .is_some_and(|format| format != resource.dxgi_format)
    {
        return Err(format!(
            "production terrain chunk library blocked: runtime texture sidecar {} does not match resource {:?} slot {} descriptor",
            sidecar.file, resource.declared_name, resource.slot
        )
        .into());
    }
    Ok(())
}

pub(crate) fn validate_terrain_chunk_library_manifest(
    manifest: &TerrainChunkLibraryManifest,
    bundle_dir: &Path,
) -> Result<()> {
    if manifest.schema != TERRAIN_CHUNK_LIBRARY_SCHEMA {
        return Err(format!(
            "production terrain chunk library blocked: unsupported schema {}",
            manifest.schema
        )
        .into());
    }
    if manifest.capture_moment != TERRAIN_CHUNK_LIBRARY_MOMENT {
        return Err(format!(
            "production terrain chunk library blocked: capture_moment is {}, expected {}",
            manifest.capture_moment, TERRAIN_CHUNK_LIBRARY_MOMENT
        )
        .into());
    }
    if manifest.chunks.is_empty() {
        return Err("production terrain chunk library blocked: manifest contains no chunks".into());
    }
    if !manifest.blockers.is_empty() {
        return Err(format!(
            "production terrain chunk library blocked: {}",
            manifest.blockers.join("; ")
        )
        .into());
    }
    let mut ids = BTreeSet::new();
    let frame = terrain_chunk_frame_from_draws(
        manifest,
        manifest
            .chunks
            .iter()
            .map(|chunk| chunk.draw.clone())
            .collect(),
    )?;
    for chunk in &manifest.chunks {
        if chunk.chunk_id.trim().is_empty() || !ids.insert(chunk.chunk_id.clone()) {
            return Err(format!(
                "production terrain chunk library blocked: duplicate or empty chunk id {:?}",
                chunk.chunk_id
            )
            .into());
        }
        validate_terrain_chunk_bounds(chunk.bounds)?;
        validate_map_frame_draw(&chunk.draw, &frame, bundle_dir, TERRAIN_MAP_FRAME_SPEC)?;
    }
    validate_terrain_chunk_draw_payloads(&frame, bundle_dir)?;
    validate_terrain_chunk_coverage(&manifest.coverage)?;
    Ok(())
}

pub(crate) fn terrain_chunk_frame_from_draws(
    manifest: &TerrainChunkLibraryManifest,
    draws: Vec<MapFrameDraw>,
) -> Result<MapFrameRuntimeBundle> {
    let active_draw = draws
        .last()
        .cloned()
        .ok_or("production terrain chunk library blocked: camera cull selected no chunks")?;
    Ok(MapFrameRuntimeBundle {
        schema: MAP_FRAME_BUNDLE_SCHEMA.to_string(),
        provenance: manifest.provenance.clone(),
        capture_moment: TERRAIN_CAPTURE_MOMENT.to_string(),
        session_manifest: manifest.session_manifest.clone(),
        shader_cache_constants: manifest.shader_cache_constants.clone(),
        active_draw,
        draws,
        post_pass_output_merger: None,
        vanilla_sources: manifest.vanilla_sources.clone(),
        runtime_files: manifest.runtime_files.clone(),
        blockers: Vec::new(),
        synthetic_payloads: BTreeMap::new(),
    })
}

fn validate_terrain_chunk_draw_payloads(
    frame: &MapFrameRuntimeBundle,
    bundle_dir: &Path,
) -> Result<()> {
    let mut blockers = Vec::new();
    for draw in &frame.draws {
        for vertex_buffer in draw.vertex_buffers.values() {
            if let Err(error) = validate_map_frame_buffer_file(
                bundle_dir,
                vertex_buffer,
                "terrain chunk vertex buffer",
            ) {
                blockers.push(error.to_string());
            }
        }
        if let Some(index_buffer) = &draw.index_buffer {
            if let Err(error) = validate_map_frame_index_buffer(bundle_dir, index_buffer) {
                blockers.push(error.to_string());
            }
        }
        for constant_buffer in &draw.constant_buffers {
            if let Err(error) = validate_map_frame_constant_buffer(bundle_dir, constant_buffer) {
                blockers.push(error.to_string());
            }
        }
    }
    let replay_blockers = native_d3d11_replay_payload_blockers(frame, false)
        .into_iter()
        .filter(|blocker| !blocker.contains("post-pass output-merger"))
        .filter(|blocker| !blocker.contains("captured ClearRenderTargetView provenance"))
        .filter(|blocker| !blocker.contains("captured ClearDepthStencilView provenance"))
        .collect::<Vec<_>>();
    blockers.extend(replay_blockers);
    if blockers.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "production terrain chunk library blocked: {}",
            blockers.join("; ")
        )
        .into())
    }
}

#[cfg(test)]
pub(crate) fn terrain_chunk_identity_from_draw(draw: &MapFrameDraw) -> TerrainChunkIdentity {
    let vertex_payloads = draw
        .vertex_buffers
        .iter()
        .filter_map(|(slot, binding)| {
            binding
                .fnv1a64
                .as_ref()
                .map(|hash| (slot.clone(), hash.clone()))
        })
        .collect();
    let per_chunk_constant_buffers = draw
        .constant_buffers
        .iter()
        .filter(|binding| binding.declared_name.as_deref() != Some(TERRAIN_SCENE_CB0_SYMBOL))
        .filter_map(|binding| {
            binding
                .fnv1a64
                .as_ref()
                .map(|hash| TerrainChunkConstantBufferIdentity {
                    stage: binding.stage.clone(),
                    slot: binding.slot,
                    declared_name: binding.declared_name.clone(),
                    fnv1a64: hash.clone(),
                })
        })
        .collect();
    TerrainChunkIdentity {
        draw_call: draw.draw_call.clone(),
        draw_count: draw.draw_count,
        draw_start: draw.draw_start,
        base_vertex_location: draw.base_vertex_location,
        per_instance_count: draw.per_instance_count,
        instance_count: draw.instance_count,
        start_instance_location: draw.start_instance_location,
        pixel_shader_fnv1a64: draw.shader.fnv1a64.clone(),
        vertex_shader_fnv1a64: draw
            .vertex_shader
            .as_ref()
            .map(|shader| shader.fnv1a64.clone())
            .unwrap_or_default(),
        vertex_payloads,
        index_payload: draw
            .index_buffer
            .as_ref()
            .and_then(|binding| binding.fnv1a64.clone()),
        per_chunk_constant_buffers,
    }
}

#[cfg(test)]
pub(crate) fn terrain_chunk_bounds_from_position_payload(
    position_bytes: &[u8],
    stride: usize,
    offset: usize,
    cb1_quad_offset_scale: Option<[f32; 4]>,
) -> Result<TerrainChunkBounds> {
    if stride < 8
        || offset
            .checked_add(8)
            .is_none_or(|end| end > position_bytes.len())
    {
        return Err(
            "production terrain chunk library blocked: POSITION0 payload stride/offset is invalid"
                .into(),
        );
    }
    let transform = cb1_quad_offset_scale.unwrap_or([0.0, 0.0, 1.0, 0.0]);
    let scale = transform[2];
    if !scale.is_finite() || scale == 0.0 || transform.iter().any(|value| !value.is_finite()) {
        return Err(
            "production terrain chunk library blocked: CB1 QuadOffset_Scale_IsDetail is invalid"
                .into(),
        );
    }
    let mut bounds = None;
    let mut cursor = offset;
    while cursor + 8 <= position_bytes.len() {
        let x = f32::from(read_i16_le(position_bytes, cursor)?) * scale + transform[0];
        let z = f32::from(read_i16_le(position_bytes, cursor + 2)?) * scale + transform[1];
        let y = 0.0;
        let candidate = TerrainChunkBounds {
            min_x: x,
            min_y: y,
            min_z: z,
            max_x: x,
            max_y: y,
            max_z: z,
        };
        bounds = Some(match bounds {
            Some(existing) => merge_chunk_bounds(existing, candidate),
            None => candidate,
        });
        cursor = cursor.saturating_add(stride);
    }
    bounds.ok_or_else(|| {
        "production terrain chunk library blocked: POSITION0 payload contains no vertices".into()
    })
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn terrain_chunk_bounds_from_draw(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
) -> Result<TerrainChunkBounds> {
    let position_element =
        required_map_frame_input_element(draw, "POSITION", 0, DXGI_FORMAT_R16G16B16A16_SINT)?;
    let position_binding = required_map_frame_vertex_buffer(draw, position_element.input_slot)?;
    let position_bytes =
        read_map_frame_buffer_bytes(bundle_dir, position_binding, "vertex buffer")?;
    let offset = usize::try_from(position_element.aligned_byte_offset)?
        .checked_add(usize::try_from(position_binding.offset.unwrap_or(0))?)
        .ok_or("production terrain chunk library blocked: POSITION0 byte offset overflow")?;
    let stride = usize::try_from(position_binding.stride.unwrap_or(0))?;
    let cb1 = terrain_chunk_cb1_quad_offset_scale(draw, bundle_dir).ok();
    terrain_chunk_bounds_from_position_payload(&position_bytes, stride, offset, cb1)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn terrain_chunk_cb1_quad_offset_scale(
    draw: &MapFrameDraw,
    bundle_dir: &Path,
) -> Result<[f32; 4]> {
    let binding = draw
        .constant_buffers
        .iter()
        .find(|binding| binding.stage == "VS" && binding.slot == TERRAIN_DRAW_CB1_BIND_POINT)
        .ok_or("production terrain chunk library blocked: draw has no VS terrain draw CB1")?;
    let bytes = read_map_frame_buffer_bytes(bundle_dir, binding, "constant buffer")?;
    if bytes.len() < 80 {
        return Err("production terrain chunk library blocked: terrain draw CB1 is too short for QuadOffset_Scale_IsDetail".into());
    }
    Ok([
        read_f32_le(&bytes, 64)?,
        read_f32_le(&bytes, 68)?,
        read_f32_le(&bytes, 72)?,
        read_f32_le(&bytes, 76)?,
    ])
}

#[cfg(test)]
pub(crate) fn coverage_for_chunk_bounds(
    bounds: &[TerrainChunkBounds],
    world_rect: TerrainChunkWorldRect,
) -> TerrainChunkCoverage {
    let world_area = f64::from((world_rect.max_x - world_rect.min_x).max(0.0))
        * f64::from((world_rect.max_z - world_rect.min_z).max(0.0));
    let clipped = bounds
        .iter()
        .filter_map(|bounds| clip_bounds_to_world(*bounds, world_rect))
        .collect::<Vec<_>>();
    let covered_area = union_rect_area(&clipped);
    let fraction = if world_area > 0.0 {
        (covered_area / world_area).clamp(0.0, 1.0)
    } else {
        0.0
    };
    TerrainChunkCoverage {
        world_rect,
        covered_area,
        world_area,
        fraction,
        holes: coverage_holes(&clipped, world_rect),
    }
}

fn validate_terrain_chunk_coverage(coverage: &TerrainChunkCoverage) -> Result<()> {
    if !coverage.fraction.is_finite()
        || coverage.fraction < 0.0
        || coverage.fraction > 1.0
        || coverage.world_area <= 0.0
        || coverage.covered_area < 0.0
    {
        return Err("production terrain chunk library blocked: coverage stats are invalid".into());
    }
    Ok(())
}

fn validate_terrain_chunk_bounds(bounds: TerrainChunkBounds) -> Result<()> {
    let values = [
        bounds.min_x,
        bounds.min_y,
        bounds.min_z,
        bounds.max_x,
        bounds.max_y,
        bounds.max_z,
    ];
    if values.iter().any(|value| !value.is_finite())
        || bounds.min_x > bounds.max_x
        || bounds.min_y > bounds.max_y
        || bounds.min_z > bounds.max_z
    {
        return Err("production terrain chunk library blocked: chunk bounds are invalid".into());
    }
    Ok(())
}

pub(crate) fn terrain_chunk_bounds_intersects_camera(
    bounds: TerrainChunkBounds,
    camera: ProductionMapCamera,
) -> bool {
    if validate_terrain_chunk_bounds(bounds).is_err() || camera.validate().is_err() {
        return false;
    }
    let corners = [
        [bounds.min_x, bounds.min_y, bounds.min_z],
        [bounds.min_x, bounds.min_y, bounds.max_z],
        [bounds.min_x, bounds.max_y, bounds.min_z],
        [bounds.min_x, bounds.max_y, bounds.max_z],
        [bounds.max_x, bounds.min_y, bounds.min_z],
        [bounds.max_x, bounds.min_y, bounds.max_z],
        [bounds.max_x, bounds.max_y, bounds.min_z],
        [bounds.max_x, bounds.max_y, bounds.max_z],
    ];
    let clips = corners
        .iter()
        .map(|corner| {
            mul_float4x4_float4(
                camera.view_projection_matrix,
                Vec4 {
                    x: corner[0],
                    y: corner[1],
                    z: corner[2],
                    w: 1.0,
                },
            )
        })
        .collect::<Vec<_>>();
    let outside_left = clips.iter().all(|clip| clip.x < -clip.w);
    let outside_right = clips.iter().all(|clip| clip.x > clip.w);
    let outside_bottom = clips.iter().all(|clip| clip.y < -clip.w);
    let outside_top = clips.iter().all(|clip| clip.y > clip.w);
    let outside_near = clips.iter().all(|clip| clip.z < 0.0);
    let outside_far = clips.iter().all(|clip| clip.z > clip.w);
    !(outside_left || outside_right || outside_bottom || outside_top || outside_near || outside_far)
}

#[cfg(test)]
fn clip_bounds_to_world(
    bounds: TerrainChunkBounds,
    world_rect: TerrainChunkWorldRect,
) -> Option<TerrainChunkWorldRect> {
    let min_x = bounds.min_x.max(world_rect.min_x);
    let max_x = bounds.max_x.min(world_rect.max_x);
    let min_z = bounds.min_z.max(world_rect.min_z);
    let max_z = bounds.max_z.min(world_rect.max_z);
    (min_x < max_x && min_z < max_z).then_some(TerrainChunkWorldRect {
        min_x,
        min_z,
        max_x,
        max_z,
    })
}

#[cfg(test)]
fn coverage_holes(
    covered: &[TerrainChunkWorldRect],
    world_rect: TerrainChunkWorldRect,
) -> Vec<TerrainChunkHole> {
    if covered.is_empty() {
        return vec![TerrainChunkHole {
            min_x: world_rect.min_x,
            min_z: world_rect.min_z,
            max_x: world_rect.max_x,
            max_z: world_rect.max_z,
        }];
    }
    let mut xs = vec![world_rect.min_x, world_rect.max_x];
    let mut zs = vec![world_rect.min_z, world_rect.max_z];
    for rect in covered {
        xs.push(rect.min_x);
        xs.push(rect.max_x);
        zs.push(rect.min_z);
        zs.push(rect.max_z);
    }
    sort_dedup_f32(&mut xs);
    sort_dedup_f32(&mut zs);
    let mut holes = Vec::new();
    for x_pair in xs.windows(2) {
        for z_pair in zs.windows(2) {
            let min_x = x_pair[0];
            let max_x = x_pair[1];
            let min_z = z_pair[0];
            let max_z = z_pair[1];
            if min_x >= max_x || min_z >= max_z {
                continue;
            }
            let covered_cell = covered.iter().any(|rect| {
                rect.min_x <= min_x
                    && rect.max_x >= max_x
                    && rect.min_z <= min_z
                    && rect.max_z >= max_z
            });
            if !covered_cell {
                holes.push(TerrainChunkHole {
                    min_x,
                    min_z,
                    max_x,
                    max_z,
                });
            }
        }
    }
    holes
}

#[cfg(test)]
fn union_rect_area(rects: &[TerrainChunkWorldRect]) -> f64 {
    if rects.is_empty() {
        return 0.0;
    }
    let mut xs = rects
        .iter()
        .flat_map(|rect| [rect.min_x, rect.max_x])
        .collect::<Vec<_>>();
    sort_dedup_f32(&mut xs);
    let mut area = 0.0f64;
    for x_pair in xs.windows(2) {
        let x0 = x_pair[0];
        let x1 = x_pair[1];
        if x1 <= x0 {
            continue;
        }
        let mut intervals = rects
            .iter()
            .filter(|rect| rect.min_x <= x0 && rect.max_x >= x1)
            .map(|rect| (rect.min_z, rect.max_z))
            .collect::<Vec<_>>();
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut covered_z = 0.0f64;
        let mut current: Option<(f32, f32)> = None;
        for (start, end) in intervals {
            if end <= start {
                continue;
            }
            match current {
                None => current = Some((start, end)),
                Some((cur_start, cur_end)) if start <= cur_end => {
                    current = Some((cur_start, cur_end.max(end)));
                }
                Some((cur_start, cur_end)) => {
                    covered_z += f64::from(cur_end - cur_start);
                    current = Some((start, end));
                }
            }
        }
        if let Some((cur_start, cur_end)) = current {
            covered_z += f64::from(cur_end - cur_start);
        }
        area += f64::from(x1 - x0) * covered_z;
    }
    area
}

#[cfg(test)]
fn merge_chunk_bounds(a: TerrainChunkBounds, b: TerrainChunkBounds) -> TerrainChunkBounds {
    TerrainChunkBounds {
        min_x: a.min_x.min(b.min_x),
        min_y: a.min_y.min(b.min_y),
        min_z: a.min_z.min(b.min_z),
        max_x: a.max_x.max(b.max_x),
        max_y: a.max_y.max(b.max_y),
        max_z: a.max_z.max(b.max_z),
    }
}

#[cfg(test)]
fn sort_dedup_f32(values: &mut Vec<f32>) {
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup_by(|a, b| (*a - *b).abs() <= f32::EPSILON);
}

#[cfg(test)]
fn read_i16_le(bytes: &[u8], offset: usize) -> Result<i16> {
    let raw = bytes.get(offset..offset + 2).ok_or_else(|| {
        format!("production terrain chunk library blocked: missing i16 bytes at {offset}")
    })?;
    Ok(i16::from_le_bytes([raw[0], raw[1]]))
}

#[cfg(test)]
#[allow(dead_code)]
fn read_f32_le(bytes: &[u8], offset: usize) -> Result<f32> {
    let raw = bytes.get(offset..offset + 4).ok_or_else(|| {
        format!("production terrain chunk library blocked: missing f32 bytes at {offset}")
    })?;
    Ok(f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::fs;

    fn identity_camera() -> ProductionMapCamera {
        ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, 0.0, 0.0],
        )
        .expect("identity camera")
    }

    fn synthetic_position_payload(points: &[(i16, i16)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (x, z) in points {
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&z.to_le_bytes());
            out.extend_from_slice(&0i16.to_le_bytes());
            out.extend_from_slice(&0i16.to_le_bytes());
        }
        out
    }

    #[test]
    fn chunk_bounds_apply_cb1_quad_offset_scale() {
        let payload = synthetic_position_payload(&[(0, 0), (64, 64), (16, 32)]);
        let bounds = terrain_chunk_bounds_from_position_payload(
            &payload,
            8,
            0,
            Some([2048.0, 1024.0, 8.0, 0.0]),
        )
        .expect("bounds");
        assert_eq!(bounds.min_x, 2048.0);
        assert_eq!(bounds.max_x, 2560.0);
        assert_eq!(bounds.min_z, 1024.0);
        assert_eq!(bounds.max_z, 1536.0);
    }

    #[test]
    fn coverage_fraction_reports_holes() {
        let world = TerrainChunkWorldRect {
            min_x: 0.0,
            min_z: 0.0,
            max_x: 4.0,
            max_z: 4.0,
        };
        let coverage = coverage_for_chunk_bounds(
            &[
                TerrainChunkBounds {
                    min_x: 0.0,
                    min_y: 0.0,
                    min_z: 0.0,
                    max_x: 2.0,
                    max_y: 0.0,
                    max_z: 4.0,
                },
                TerrainChunkBounds {
                    min_x: 2.0,
                    min_y: 0.0,
                    min_z: 0.0,
                    max_x: 4.0,
                    max_y: 0.0,
                    max_z: 2.0,
                },
            ],
            world,
        );
        assert_eq!(coverage.covered_area, 12.0);
        assert_eq!(coverage.world_area, 16.0);
        assert_eq!(coverage.fraction, 0.75);
        assert_eq!(
            coverage.holes,
            vec![TerrainChunkHole {
                min_x: 2.0,
                min_z: 2.0,
                max_x: 4.0,
                max_z: 4.0,
            }]
        );
    }

    #[test]
    fn frustum_cull_selection_is_deterministic() {
        let camera = identity_camera();
        let visible = TerrainChunkBounds {
            min_x: -0.5,
            min_y: -0.5,
            min_z: 0.0,
            max_x: 0.5,
            max_y: 0.5,
            max_z: 0.5,
        };
        let outside = TerrainChunkBounds {
            min_x: 2.0,
            min_y: -0.5,
            min_z: 0.0,
            max_x: 3.0,
            max_y: 0.5,
            max_z: 0.5,
        };
        assert!(terrain_chunk_bounds_intersects_camera(visible, camera));
        assert!(!terrain_chunk_bounds_intersects_camera(outside, camera));
    }

    #[test]
    fn chunk_identity_includes_per_chunk_cb_but_not_scene_cb0() {
        let root = temp_test_dir("chunk_identity");
        fs::create_dir_all(&root).expect("create root");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.draws[0].constant_buffers[0].fnv1a64 = Some("scene_a".to_string());
            frame.draws[1].constant_buffers[0].fnv1a64 = Some("scene_b".to_string());
            frame.draws[0].constant_buffers[1].fnv1a64 = Some("chunk_a".to_string());
            frame.draws[1].constant_buffers[1].fnv1a64 = Some("chunk_b".to_string());
            frame.active_draw = frame.draws[1].clone();
            frame
        });
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(path).expect("read bundle")).expect("frame");
        let first = terrain_chunk_identity_from_draw(&frame.draws[0]);
        let second = terrain_chunk_identity_from_draw(&frame.draws[1]);
        assert_ne!(first, second);
        assert!(!first
            .per_chunk_constant_buffers
            .iter()
            .any(|buffer| buffer.declared_name.as_deref() == Some(TERRAIN_SCENE_CB0_SYMBOL)));
    }

    #[test]
    fn terrain_chunk_library_manifest_round_trips() {
        let root = temp_test_dir("chunk_library_round_trip");
        fs::create_dir_all(&root).expect("create root");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                let mut state = valid_map_frame_draw_state();
                state.output_merger = Some(valid_output_merger_state());
                draw.draw_state = Some(state);
            }
            frame.active_draw = frame.draws[1].clone();
            frame
        });
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(path).expect("read bundle")).expect("frame");
        let bounds = TerrainChunkBounds {
            min_x: 0.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1.0,
            max_y: 0.0,
            max_z: 1.0,
        };
        let manifest = TerrainChunkLibraryManifest {
            schema: TERRAIN_CHUNK_LIBRARY_SCHEMA.to_string(),
            provenance: "unit_test".to_string(),
            capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT.to_string(),
            session_manifest: MAP_CAPTURE_SESSION_FILE.to_string(),
            shader_cache_constants: frame.shader_cache_constants.clone(),
            chunks: vec![TerrainChunkRecord {
                chunk_id: "chunk_a".to_string(),
                identity: terrain_chunk_identity_from_draw(&frame.draws[0]),
                bounds,
                draw: frame.draws[0].clone(),
            }],
            vanilla_sources: frame.vanilla_sources.clone(),
            runtime_files: frame.runtime_files.clone(),
            coverage: coverage_for_chunk_bounds(
                &[bounds],
                TerrainChunkWorldRect {
                    min_x: 0.0,
                    min_z: 0.0,
                    max_x: 1.0,
                    max_z: 1.0,
                },
            ),
            blockers: Vec::new(),
        };
        fs::write(
            root.join(TERRAIN_CHUNK_LIBRARY_FILE),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let library = NativeD3d11TerrainChunkLibrary::load(&root).expect("load chunk library");
        assert_eq!(library.chunk_count(), 1);
        assert_eq!(library.coverage_fraction(), 1.0);
        let graph = ProductionMapPassGraph::from_terrain_chunk_library(&library, identity_camera())
            .expect("chunk library establishes production graph");
        let terrain = graph
            .pass(ProductionMapPassKind::Terrain)
            .expect("terrain pass");
        assert_eq!(terrain.status, ProductionMapPassStatus::Ready);
        assert_eq!(terrain.capture_moment, TERRAIN_CHUNK_LIBRARY_MOMENT);
    }

    #[test]
    fn incomplete_chunk_library_coverage_blocks_production_graph() {
        let root = temp_test_dir("incomplete_chunk_library_coverage_blocks_graph");
        fs::create_dir_all(&root).expect("create root");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            for draw in &mut frame.draws {
                let mut state = valid_map_frame_draw_state();
                state.output_merger = Some(valid_output_merger_state());
                draw.draw_state = Some(state);
            }
            frame.active_draw = frame.draws[0].clone();
            frame
        });
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(path).expect("read bundle")).expect("frame");
        let bounds = TerrainChunkBounds {
            min_x: 0.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1.0,
            max_y: 0.0,
            max_z: 1.0,
        };
        let manifest = TerrainChunkLibraryManifest {
            schema: TERRAIN_CHUNK_LIBRARY_SCHEMA.to_string(),
            provenance: "unit_test".to_string(),
            capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT.to_string(),
            session_manifest: MAP_CAPTURE_SESSION_FILE.to_string(),
            shader_cache_constants: frame.shader_cache_constants.clone(),
            chunks: vec![TerrainChunkRecord {
                chunk_id: "chunk_a".to_string(),
                identity: terrain_chunk_identity_from_draw(&frame.draws[0]),
                bounds,
                draw: frame.draws[0].clone(),
            }],
            vanilla_sources: frame.vanilla_sources.clone(),
            runtime_files: frame.runtime_files.clone(),
            coverage: coverage_for_chunk_bounds(
                &[bounds],
                TerrainChunkWorldRect {
                    min_x: 0.0,
                    min_z: 0.0,
                    max_x: 2.0,
                    max_z: 2.0,
                },
            ),
            blockers: Vec::new(),
        };
        fs::write(
            root.join(TERRAIN_CHUNK_LIBRARY_FILE),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let library = NativeD3d11TerrainChunkLibrary::load(&root).expect("load chunk library");
        assert!(library.coverage_fraction() < 1.0);
        let result =
            ProductionMapPassGraph::from_terrain_chunk_library(&library, identity_camera());
        assert!(result.is_err(), "partial coverage must fail closed");
        let err = result.err().unwrap();
        assert!(
            err.to_string()
                .contains("terrain chunk library coverage is 0.250000"),
            "{err}"
        );
    }

    #[test]
    fn chunk_library_graph_promotes_full_map_dependent_layers() {
        let terrain_root = temp_test_dir("chunk_library_graph_full_map_layers_terrain");
        let water_root = temp_test_dir("chunk_library_graph_full_map_layers_water");
        let river_root = temp_test_dir("chunk_library_graph_full_map_layers_rivers");
        let tree_root = temp_test_dir("chunk_library_graph_full_map_layers_trees");
        for root in [&terrain_root, &water_root, &river_root, &tree_root] {
            fs::create_dir_all(root).expect("create root");
        }
        let path = write_valid_map_frame_bundle(&terrain_root, |mut frame| {
            for draw in &mut frame.draws {
                let mut state = valid_map_frame_draw_state();
                state.output_merger = Some(valid_output_merger_state());
                draw.draw_state = Some(state);
            }
            frame.active_draw = frame.draws[0].clone();
            frame
        });
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(path).expect("read bundle")).expect("frame");
        let bounds = TerrainChunkBounds {
            min_x: 0.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1.0,
            max_y: 0.0,
            max_z: 1.0,
        };
        let manifest = TerrainChunkLibraryManifest {
            schema: TERRAIN_CHUNK_LIBRARY_SCHEMA.to_string(),
            provenance: "unit_test".to_string(),
            capture_moment: TERRAIN_CHUNK_LIBRARY_MOMENT.to_string(),
            session_manifest: MAP_CAPTURE_SESSION_FILE.to_string(),
            shader_cache_constants: frame.shader_cache_constants.clone(),
            chunks: vec![TerrainChunkRecord {
                chunk_id: "chunk_a".to_string(),
                identity: terrain_chunk_identity_from_draw(&frame.draws[0]),
                bounds,
                draw: frame.draws[0].clone(),
            }],
            vanilla_sources: frame.vanilla_sources.clone(),
            runtime_files: frame.runtime_files.clone(),
            coverage: coverage_for_chunk_bounds(
                &[bounds],
                TerrainChunkWorldRect {
                    min_x: 0.0,
                    min_z: 0.0,
                    max_x: 1.0,
                    max_z: 1.0,
                },
            ),
            blockers: Vec::new(),
        };
        fs::write(
            terrain_root.join(TERRAIN_CHUNK_LIBRARY_FILE),
            serde_json::to_vec_pretty(&manifest).expect("manifest json"),
        )
        .expect("write manifest");
        let library = NativeD3d11TerrainChunkLibrary::load(&terrain_root).expect("load library");
        let water_path = write_valid_water_map_frame_bundle(&water_root, |mut frame| {
            add_valid_water_output_merger_state(&water_root, &mut frame);
            frame
        });
        let rivers_path = write_valid_map_layer_frame_bundle(
            &river_root,
            RIVERS_MAP_FRAME_SPEC,
            &RIVER_FRAME_SHADER_SOURCES,
            &RIVER_FRAME_REQUIRED_SHADER_RESOURCES,
            "river_pixel_shader_5555555555555555.bin",
            "map_vertex_shader_5555555555555555.bin",
            |frame| frame,
        );
        let trees_path = write_valid_map_layer_frame_bundle(
            &tree_root,
            TREES_MAP_FRAME_SPEC,
            &TREE_FRAME_SHADER_SOURCES,
            &TREE_FRAME_REQUIRED_SHADER_RESOURCES,
            "tree_pixel_shader_7777777777777777.bin",
            "map_vertex_shader_7777777777777777.bin",
            |frame| frame,
        );
        let water_bundle =
            NativeD3d11TerrainBundle::load_water(&water_path).expect("load water bundle");
        let rivers_bundle =
            NativeD3d11TerrainBundle::load_rivers(&rivers_path).expect("load rivers bundle");
        let trees_bundle =
            NativeD3d11TerrainBundle::load_trees(&trees_path).expect("load trees bundle");

        let cull_empty_camera = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 4.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.0, 0.0, 0.0],
        )
        .expect("cull-empty camera");
        assert!(
            library.visible_chunk_ids(cull_empty_camera).is_empty(),
            "test camera should prove graph setup does not depend on visible captured chunks"
        );

        let graph = ProductionMapPassGraph::from_terrain_chunk_library(&library, cull_empty_camera)
            .expect("chunk library establishes graph")
            .with_water_capture(&water_bundle)
            .expect("water remains available after terrain synthesis")
            .with_rivers_capture(&rivers_bundle)
            .expect("rivers remain available after terrain synthesis")
            .with_trees_capture(&trees_bundle)
            .expect("trees remain available after terrain synthesis");

        for kind in [
            ProductionMapPassKind::Terrain,
            ProductionMapPassKind::Water,
            ProductionMapPassKind::Rivers,
            ProductionMapPassKind::Trees,
        ] {
            let pass = graph.pass(kind).expect("pass");
            assert_eq!(pass.status, ProductionMapPassStatus::Ready);
        }
        assert_eq!(
            graph
                .pass(ProductionMapPassKind::Terrain)
                .expect("terrain pass")
                .capture_moment,
            TERRAIN_CHUNK_LIBRARY_MOMENT
        );
    }
}
