use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::constants::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_TEXTURECUBE,
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_B8G8R8X8_UNORM,
    DXGI_FORMAT_BC1_UNORM, DXGI_FORMAT_BC1_UNORM_SRGB, DXGI_FORMAT_BC3_UNORM,
    DXGI_FORMAT_BC3_UNORM_SRGB, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT,
    DXGI_FORMAT_R8G8B8A8_UNORM,
};
#[cfg(windows)]
use winapi::um::d3d11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};
#[cfg(windows)]
use winapi::um::d3dcommon::*;
#[cfg(windows)]
use winapi::um::unknwnbase::IUnknown;

/// WATER3 — the world-synthesis (`--synthesize-terrain-chunks`) production
/// path's `WaterRefraction` (slot 5) source law, as a pure function so it is
/// unit-testable without a GPU and stays in lockstep with the captured-terrain
/// path's non-reference selection (see
/// `prepare_production_map_with_layers_for_reference`).
///
/// * `water_full == false` → the old calibrated approximation snapshot
///   (`AppOwnedTerrainColorSnapshot`), now blocked for non-reference product
///   water rendering when a water pass is present.
/// * `water_full == true` → the mechanism-correct WATER2 seafloor render
///   (`UnderwaterPass`, `pdxmap.shader` `Effect underwater` over the
///   synthesized tiles). The underwater pass is therefore selected **only**
///   under `--water-full`; see documents/WATER_LAW.md §2a.
#[cfg(windows)]
pub(crate) fn world_synthesis_water_refraction_source(
    water_full: bool,
) -> NativeD3d11WaterRefractionSource {
    if water_full {
        NativeD3d11WaterRefractionSource::UnderwaterPass
    } else {
        NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot
    }
}

/// WATER3 — the world-synthesis path's `ReflectionCubeMap` (slot 8) source law
/// (WATER1), a separate axis from refraction. `water_full` uploads the real
/// shipped cube (byte-identical to `map/terrain/reflection.dds`); flag-OFF
/// keeps the mean-color flatten for byte-identity.
#[cfg(windows)]
pub(crate) fn world_synthesis_reflection_cube_source(
    water_full: bool,
) -> NativeD3d11ReflectionCubeSource {
    if water_full {
        NativeD3d11ReflectionCubeSource::ShippedFileCube
    } else {
        NativeD3d11ReflectionCubeSource::MeanColorFlatten
    }
}

#[cfg(windows)]
pub(crate) fn product_water_source_policy(
    reference_replay: bool,
    water_full: bool,
    has_water_bundle: bool,
) -> Result<(
    NativeD3d11WaterRefractionSource,
    NativeD3d11ReflectionCubeSource,
)> {
    let water_refraction_source = if reference_replay {
        NativeD3d11WaterRefractionSource::CapturedBundleTexture
    } else if water_full {
        NativeD3d11WaterRefractionSource::UnderwaterPass
    } else if has_water_bundle {
        return Err(
            "production water replay blocked: non-reference water rendering requires --water-full and a resolver-backed underwater pass; the old AppOwnedTerrainColorSnapshot approximation is not engine-derived"
                .into(),
        );
    } else {
        NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot
    };
    let reflection_cube_source = if reference_replay || water_full {
        NativeD3d11ReflectionCubeSource::ShippedFileCube
    } else {
        NativeD3d11ReflectionCubeSource::MeanColorFlatten
    };
    Ok((water_refraction_source, reflection_cube_source))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeD3d11ReplaySummary {
    pub draw_count: usize,
    pub vertex_buffer_count: usize,
    pub index_buffer_count: usize,
    pub constant_buffer_count: usize,
    pub shader_resource_count: usize,
    pub sampler_count: usize,
    pub target_width: u32,
    pub target_height: u32,
    pub target_format: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeD3d11ReplayReadback {
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub row_pitch: u32,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeD3d11ReferenceComparison {
    pub byte_len: usize,
    pub expected_fnv1a64: String,
    pub actual_fnv1a64: String,
}

/// WATER2 validation render result: the reconstructed `WaterRefraction` buffer
/// (the underwater seafloor pass) plus the compiled underwater PS bytecode for
/// provenance. `bytes` are the read-back `B8G8R8A8_UNORM` texels
/// (`row_pitch`-padded, D3D11 staging layout).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnderwaterValidationRender {
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub row_pitch: u32,
    pub bytes: Vec<u8>,
    pub ps_bytecode: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeD3d11ReplayResult {
    pub summary: NativeD3d11ReplaySummary,
    pub readback: Option<NativeD3d11ReplayReadback>,
    pub reference_comparison: Option<NativeD3d11ReferenceComparison>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainD3d11AcceptanceReport {
    pub schema: String,
    pub status: String,
    pub capture_moment: String,
    pub bundle_file: String,
    pub map_frame_bundle_fnv1a64: String,
    pub map_capture_session_fnv1a64: String,
    pub repeat: usize,
    pub compare_each_draw: bool,
    pub draw_count: usize,
    pub vertex_buffer_count: usize,
    pub index_buffer_count: usize,
    pub constant_buffer_count: usize,
    pub shader_resource_count: usize,
    pub sampler_count: usize,
    pub target_width: u32,
    pub target_height: u32,
    pub target_format: u32,
    pub final_reference_byte_len: usize,
    pub final_expected_fnv1a64: String,
    pub final_actual_fnv1a64: String,
}

pub struct NativeD3d11TerrainRenderer {
    #[cfg(windows)]
    pub(crate) device: NativeD3d11Device,
    #[cfg(windows)]
    pub(crate) terrain_color_snapshot_pipeline: NativeD3d11TerrainColorSnapshotPipeline,
    #[cfg(not(windows))]
    pub(crate) _unavailable: (),
    /// `--water-full` mode (WATER1). Default `false` => flag-OFF, byte-identical
    /// to prior behaviour. When `true`, the world-synthesis water pass uploads
    /// the REAL reflection cube (byte-identical to the shipped, mod-resolvable
    /// map/terrain/reflection.dds; see documents/WATER_LAW.md) instead of the
    /// 1x1x6 mean-color flatten. WATER1 fixed reflection; WATER2 (below) makes
    /// refraction mechanism-correct too, so under `--water-full` the water pass
    /// now binds a real seafloor render as `WaterRefraction` instead of the
    /// approximation.
    pub(crate) water_full: bool,
    /// WATER2: resolver used to reconstruct the `Effect underwater` pixel
    /// shader (from the shipped, mod-resolvable `gfx/FX/pdxmap.shader`) that
    /// fills `WaterRefraction` with the real seafloor pass. Required when
    /// `water_full` is set on the world-synthesis path; when absent the
    /// renderer has no game-file access and the underwater pass fails closed
    /// rather than silently falling back to the approximation. Ignored when
    /// `water_full` is off (byte-identical to prior behaviour).
    #[cfg(windows)]
    pub(crate) underwater_resolver: Option<GameFileResolver>,
    /// BORD2 `--borders-from-files` mode. Default `false` => flag-OFF,
    /// byte-identical to prior behaviour (no border pass, no shader compile, no
    /// buffer allocation). When `true`, the world-synthesis production-map path
    /// renders the synthesized thin-border mesh (all classes) over the terrain
    /// target with the shipped `gfx/FX/border.shader` compiled byte-exact,
    /// per-class `BorderDiffuse` textures, and camera-height fade per
    /// [`crate::border_transparency`]. See documents/BORDER_LAW.md.
    pub(crate) borders_from_files: bool,
    /// BORD2: resolver used to synthesize the border mesh and resolve the border
    /// shader + `map/terrain/border_*.dds` textures (mod-aware). Required when
    /// `borders_from_files` is set; when absent the border pass fails closed
    /// rather than rendering nothing silently. Ignored when
    /// `borders_from_files` is off (byte-identical to prior behaviour).
    #[cfg(windows)]
    pub(crate) border_resolver: Option<GameFileResolver>,
    /// CHUNK1 `--chunks-from-files` mode. Default `false` => flag-OFF,
    /// byte-identical to prior behaviour (chunk VB/IB/CB1 uploaded from the
    /// captured template files on disk). When `true`, the terrain-chunk
    /// synthesis frame UPLOADS GENERATED mesh buffers (slot0 grid, index
    /// tessellation, per-LOD slot1 height pool from `map/heightmap.bmp`, CB1)
    /// injected as synthetic payloads keyed by the recorded file names, so the
    /// GPU vertex/index/constant buffers are materialized from files+law, not
    /// the captured buffer bytes. The generated bytes are byte-identical to the
    /// captured law (proven by the corpus gate + the synthetic-payload
    /// length+hash guard, which fails closed on any drift).
    ///
    /// KNOWN RESIDUAL (honest scope): the plan/library-load stage still READS
    /// the captured chunk buffer files for scene-CB0/camera and the per-LOD
    /// `cb1_scale`/detail-flag metadata (`terrain_chunk_synthesis_plan`,
    /// `production_camera_from_capture`), so this mode reduces the mesh-BYTE
    /// dependency to metadata-reading, it does not yet fully eliminate the
    /// captured template. Making the plan derive scale/camera from files is the
    /// documented follow-up. See [`crate::terrain_mesh_law`].
    pub(crate) chunks_from_files: bool,
    /// CHUNK1: resolver used to load `map/heightmap.bmp` (mod-aware) for the
    /// generated slot1 height pool. Required when `chunks_from_files` is set;
    /// when absent the synthesis fails closed. Ignored when off (byte-identical
    /// to prior behaviour).
    #[cfg(windows)]
    pub(crate) chunks_from_files_resolver: Option<GameFileResolver>,
    /// WATER1/REFLECT: resolver used to decode + repack the mod-aware
    /// `map/terrain/reflection.dds` into the captured `ReflectionCubeMap`
    /// (slot 8) mapped-subresource layout at replay time. When present, the
    /// reflection cube is produced by
    /// [`crate::reflection_cube_law::decode_reflection_cube_payload`] and
    /// BYTE-VERIFIED equal to the captured payload (fail closed on mismatch),
    /// so pixels are provably unchanged for vanilla and mod-correct otherwise.
    /// When absent, the captured `.d3d11` payload is uploaded verbatim
    /// (byte-identical to prior behaviour). Provenance-only: the descriptor,
    /// SRV, and DXGI 91 (`B8G8R8A8_UNORM_SRGB`) view stay bound from the
    /// capture; only the byte SOURCE flips. See documents/WATER_LAW.md.
    #[cfg(windows)]
    pub(crate) reflection_resolver: Option<GameFileResolver>,
}

pub struct NativeD3d11PreparedTerrainBundle {
    pub(crate) draw_count: usize,
    pub(crate) target_width: u32,
    pub(crate) target_height: u32,
    pub(crate) target_format: u32,
    #[cfg(windows)]
    pub(crate) native: NativeD3d11PreparedMapFrame,
    #[cfg(not(windows))]
    pub(crate) _unavailable: (),
}

pub struct NativeD3d11PreparedProductionMap {
    pub(crate) graph: ProductionMapPassGraph,
    pub(crate) terrain: NativeD3d11PreparedTerrainBundle,
    #[cfg(windows)]
    pub(crate) water: Option<NativeD3d11PreparedMapPass>,
    #[cfg(windows)]
    pub(crate) rivers: Option<NativeD3d11PreparedMapPass>,
    #[cfg(windows)]
    pub(crate) borders: Option<NativeD3d11PreparedMapPass>,
    #[cfg(windows)]
    pub(crate) trees: Option<NativeD3d11PreparedMapPass>,
    /// BORD2 `--borders-from-files`: the synthesized thin-border pass. `None`
    /// when the flag is off (byte-identical) or on the reference-replay path.
    /// Renders after water/rivers, before trees (BORDER_LAW.md draw order).
    #[cfg(windows)]
    pub(crate) borders_files: Option<NativeD3d11BorderFilesPass>,
}

pub struct NativeD3d11PreparedSynthesizedProductionMap {
    pub(crate) graph: ProductionMapPassGraph,
    pub(crate) terrain: NativeD3d11PreparedTerrainChunkSynthesis,
    #[cfg(windows)]
    pub(crate) water: Option<NativeD3d11PreparedMapPass>,
    #[cfg(windows)]
    pub(crate) rivers: Option<NativeD3d11PreparedMapPass>,
    #[cfg(windows)]
    pub(crate) borders: Option<NativeD3d11PreparedMapPass>,
    #[cfg(windows)]
    pub(crate) trees: Option<NativeD3d11PreparedMapPass>,
    /// BORD2 `--borders-from-files` (see the captured-path struct above).
    #[cfg(windows)]
    pub(crate) borders_files: Option<NativeD3d11BorderFilesPass>,
}

pub struct NativeD3d11PreparedTerrainChunkSynthesis {
    pub(crate) plan: TerrainChunkSynthesisPlan,
    pub(crate) draw_count: usize,
    pub(crate) target_width: u32,
    pub(crate) target_height: u32,
    pub(crate) target_format: u32,
    #[cfg(windows)]
    pub(crate) native: NativeD3d11PreparedTerrainChunkSynthesisFrame,
    #[cfg(not(windows))]
    pub(crate) _unavailable: (),
}

impl NativeD3d11PreparedTerrainBundle {
    pub fn draw_count(&self) -> usize {
        self.draw_count
    }

    pub fn target_width(&self) -> u32 {
        self.target_width
    }

    pub fn target_height(&self) -> u32 {
        self.target_height
    }

    pub fn target_format(&self) -> u32 {
        self.target_format
    }
}

impl NativeD3d11PreparedProductionMap {
    pub fn graph(&self) -> &ProductionMapPassGraph {
        &self.graph
    }

    pub fn terrain(&self) -> &NativeD3d11PreparedTerrainBundle {
        &self.terrain
    }

    pub fn has_water_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.water.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn has_rivers_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.rivers.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn has_borders_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.borders.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// BORD2: stats for the prepared `--borders-from-files` overlay (VRAM +
    /// class ranges + prepare time), `None` when the flag is off.
    pub fn borders_files_stats(&self) -> Option<&BorderFilesPassStats> {
        #[cfg(windows)]
        {
            self.borders_files.as_ref().map(|pass| pass.stats())
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    pub fn has_trees_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.trees.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn ready_dependent_pass_labels(&self) -> Vec<&'static str> {
        self.graph
            .passes
            .iter()
            .filter(|pass| {
                pass.kind != ProductionMapPassKind::Terrain
                    && pass.status == ProductionMapPassStatus::Ready
            })
            .map(|pass| pass.kind.pass_label())
            .collect()
    }

    pub fn target_width(&self) -> u32 {
        self.terrain.target_width
    }

    pub fn target_height(&self) -> u32 {
        self.terrain.target_height
    }

    pub fn target_format(&self) -> u32 {
        self.terrain.target_format
    }

    #[cfg(windows)]
    pub fn output_texture_raw(&self) -> *mut ID3D11Texture2D {
        self.terrain.native.render_target.texture.as_raw()
    }

    /// SEASON1 (validation): replace the SnowMudTexture SRV in the terrain
    /// pass and every dependent pass (water ice, tree snow, border snow all
    /// sample it). Name-matched so a session without the input fails loudly.
    pub unsafe fn replace_snow_mud_srv(
        &mut self,
        snow_mud: *mut ID3D11ShaderResourceView,
    ) -> Result<usize> {
        let mut count = replace_named_ps_srv_in_pass(
            &mut self.terrain.native.pass,
            "SnowMudTexture",
            snow_mud,
        )?;
        for pass in [
            self.water.as_mut(),
            self.rivers.as_mut(),
            self.borders.as_mut(),
            self.trees.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            count += replace_named_ps_srv_in_pass(pass, "SnowMudTexture", snow_mud)?;
        }
        Ok(count)
    }

    #[cfg(windows)]
    pub(crate) fn dependent_passes_in_order(
        &self,
    ) -> Vec<(ProductionMapPassKind, &NativeD3d11PreparedMapPass)> {
        let mut passes = Vec::new();
        if let Some(water) = self.water.as_ref() {
            passes.push((ProductionMapPassKind::Water, water));
        }
        if let Some(rivers) = self.rivers.as_ref() {
            passes.push((ProductionMapPassKind::Rivers, rivers));
        }
        if let Some(borders) = self.borders.as_ref() {
            passes.push((ProductionMapPassKind::Borders, borders));
        }
        if let Some(trees) = self.trees.as_ref() {
            passes.push((ProductionMapPassKind::Trees, trees));
        }
        passes
    }
}

impl NativeD3d11PreparedSynthesizedProductionMap {
    pub fn graph(&self) -> &ProductionMapPassGraph {
        &self.graph
    }

    pub fn terrain(&self) -> &NativeD3d11PreparedTerrainChunkSynthesis {
        &self.terrain
    }

    pub fn has_water_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.water.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn has_rivers_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.rivers.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn has_trees_pass(&self) -> bool {
        #[cfg(windows)]
        {
            self.trees.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// BORD2: stats for the prepared `--borders-from-files` overlay, `None`
    /// when the flag is off.
    pub fn borders_files_stats(&self) -> Option<&BorderFilesPassStats> {
        #[cfg(windows)]
        {
            self.borders_files.as_ref().map(|pass| pass.stats())
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    pub fn target_width(&self) -> u32 {
        self.terrain.target_width
    }

    pub fn target_height(&self) -> u32 {
        self.terrain.target_height
    }

    pub fn target_format(&self) -> u32 {
        self.terrain.target_format
    }

    #[cfg(windows)]
    pub fn output_texture_raw(&self) -> *mut ID3D11Texture2D {
        self.terrain.native.render_target.texture.as_raw()
    }

    /// SEASON1 (validation): SnowMudTexture substitution across the
    /// synthesized terrain template draw + dependent passes.
    pub unsafe fn replace_snow_mud_srv(
        &mut self,
        snow_mud: *mut ID3D11ShaderResourceView,
    ) -> Result<usize> {
        let mut count = self.terrain.replace_snow_mud_srv(snow_mud)?;
        for pass in [
            self.water.as_mut(),
            self.rivers.as_mut(),
            self.borders.as_mut(),
            self.trees.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            count += replace_named_ps_srv_in_pass(pass, "SnowMudTexture", snow_mud)?;
        }
        Ok(count)
    }

    #[cfg(windows)]
    pub(crate) fn dependent_passes_in_order(
        &self,
    ) -> Vec<(ProductionMapPassKind, &NativeD3d11PreparedMapPass)> {
        let mut passes = Vec::new();
        if let Some(water) = self.water.as_ref() {
            passes.push((ProductionMapPassKind::Water, water));
        }
        if let Some(rivers) = self.rivers.as_ref() {
            passes.push((ProductionMapPassKind::Rivers, rivers));
        }
        if let Some(borders) = self.borders.as_ref() {
            passes.push((ProductionMapPassKind::Borders, borders));
        }
        if let Some(trees) = self.trees.as_ref() {
            passes.push((ProductionMapPassKind::Trees, trees));
        }
        passes
    }
}

impl NativeD3d11PreparedTerrainChunkSynthesis {
    pub fn draw_count(&self) -> usize {
        self.draw_count
    }

    pub fn target_width(&self) -> u32 {
        self.target_width
    }

    pub fn target_height(&self) -> u32 {
        self.target_height
    }

    pub fn target_format(&self) -> u32 {
        self.target_format
    }

    pub fn tile_count(&self) -> usize {
        self.plan.tile_count()
    }

    pub fn target_tile_size(&self) -> u32 {
        self.plan.target_tile_size()
    }

    pub fn lod_mappings(&self) -> &[TerrainChunkSynthesisLodMapping] {
        self.plan.lod_mappings()
    }

    #[cfg(windows)]
    pub fn output_texture_raw(&self) -> *mut ID3D11Texture2D {
        self.native.render_target.texture.as_raw()
    }

    /// SEASON1 (validation): SnowMudTexture (terrain sampler slot 7 per
    /// `pdxmap.shader:77`; name-verified) substitution on the template draw.
    pub unsafe fn replace_snow_mud_srv(
        &mut self,
        snow_mud: *mut ID3D11ShaderResourceView,
    ) -> Result<usize> {
        replace_terrain_ps_srv_slot_in_draw(
            &mut self.native.template_draw,
            7,
            snow_mud,
            "SnowMudTexture",
        )
    }
}

/// Replace every PS-stage SRV whose captured `declared_name` matches, across
/// all draws of a dependent pass. Zero replacements is fine (the pass may not
/// sample that input).
#[cfg(windows)]
unsafe fn replace_named_ps_srv_in_pass(
    pass: &mut NativeD3d11PreparedMapPass,
    declared_name: &str,
    view: *mut ID3D11ShaderResourceView,
) -> Result<usize> {
    if view.is_null() {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {declared_name} replacement SRV is null"
        )
        .into());
    }
    let mut replaced = 0usize;
    for draw in &mut pass.draws {
        for resource in &mut draw.shader_resources {
            if resource.stage == "PS" && resource.declared_name.as_deref() == Some(declared_name) {
                resource.source = NativeD3d11ShaderResourceSource::CapturedBundleTexture;
                resource.view = Some(borrowed_srv_ptr(view, declared_name)?);
                replaced += 1;
            }
        }
    }
    Ok(replaced)
}

#[cfg(windows)]
unsafe fn replace_terrain_ps_srv_slot_in_draw(
    draw: &mut NativeD3d11PreparedDraw,
    slot: u32,
    view: *mut ID3D11ShaderResourceView,
    label: &str,
) -> Result<usize> {
    if view.is_null() {
        return Err(format!(
            "production native D3D11 terrain replay blocked: {label} replacement SRV is null"
        )
        .into());
    }
    let mut replaced = 0usize;
    for resource in &mut draw.shader_resources {
        if resource.stage == "PS" && resource.slot == slot {
            resource.source = NativeD3d11ShaderResourceSource::CapturedBundleTexture;
            resource.view = Some(borrowed_srv_ptr(view, label)?);
            replaced += 1;
        }
    }
    Ok(replaced)
}

#[cfg(windows)]
unsafe fn borrowed_srv_ptr(
    view: *mut ID3D11ShaderResourceView,
    label: &str,
) -> Result<D3dPtr<ID3D11ShaderResourceView>> {
    (*(view as *mut IUnknown)).AddRef();
    D3dPtr::from_raw(view, label)
}

#[cfg(windows)]
impl NativeD3d11TerrainRenderer {
    pub fn new() -> Result<Self> {
        unsafe {
            let device = native_d3d11_create_device()?;
            let terrain_color_snapshot_pipeline =
                native_d3d11_create_terrain_color_snapshot_pipeline(&device)?;
            Ok(Self {
                device,
                terrain_color_snapshot_pipeline,
                water_full: false,
                underwater_resolver: None,
                borders_from_files: false,
                border_resolver: None,
                chunks_from_files: false,
                chunks_from_files_resolver: None,
                reflection_resolver: None,
            })
        }
    }

    pub unsafe fn from_borrowed_raw(
        device: *mut ID3D11Device,
        context: *mut ID3D11DeviceContext,
    ) -> Result<Self> {
        if device.is_null() || context.is_null() {
            return Err(
                "production native D3D11 terrain replay blocked: borrowed D3D11 device/context is null"
                    .into(),
            );
        }
        (*(device as *mut IUnknown)).AddRef();
        (*(context as *mut IUnknown)).AddRef();
        let device = NativeD3d11Device {
            device: D3dPtr::from_raw(device, "borrowed D3D11 device")?,
            context: D3dPtr::from_raw(context, "borrowed D3D11 immediate context")?,
        };
        let terrain_color_snapshot_pipeline =
            native_d3d11_create_terrain_color_snapshot_pipeline(&device)?;
        Ok(Self {
            device,
            terrain_color_snapshot_pipeline,
            water_full: false,
            underwater_resolver: None,
            borders_from_files: false,
            border_resolver: None,
            chunks_from_files: false,
            chunks_from_files_resolver: None,
            reflection_resolver: None,
        })
    }

    /// Provide the game-file resolver used to reconstruct the underwater
    /// seafloor shader for the WATER2 refraction pass. Required alongside
    /// `--water-full` on the world-synthesis path; if `--water-full` is set
    /// without a resolver, the underwater pass fails closed. Builder-style.
    #[cfg(windows)]
    pub fn with_underwater_resolver(mut self, resolver: GameFileResolver) -> Self {
        self.underwater_resolver = Some(resolver);
        self
    }

    /// Provide the game-file resolver used to decode + repack the mod-aware
    /// `map/terrain/reflection.dds` into the captured `ReflectionCubeMap`
    /// (slot 8) mapped-subresource layout (WATER1/REFLECT). When present, the
    /// reflection cube for that slot is resolver-decoded and BYTE-VERIFIED equal
    /// to the captured payload (fail closed on mismatch); when absent, the
    /// captured `.d3d11` payload is uploaded verbatim (byte-identical to prior
    /// behaviour). Provenance-only — the descriptor/SRV/format 91 are untouched.
    /// Builder-style.
    #[cfg(windows)]
    pub fn with_reflection_resolver(mut self, resolver: GameFileResolver) -> Self {
        self.reflection_resolver = Some(resolver);
        self
    }

    /// Enable/disable `--water-full` mode (WATER1). Default is `false`.
    ///
    /// When enabled, the world-synthesis water pass uploads the real reflection
    /// cube instead of the mean-color flatten. Callers that leave this OFF keep
    /// byte-identical output. Intended to be set once, right after construction
    /// (e.g. from the replay_app CLI `--water-full` flag).
    pub fn set_water_full(&mut self, water_full: bool) {
        self.water_full = water_full;
    }

    /// Builder-style variant of [`set_water_full`](Self::set_water_full).
    pub fn with_water_full(mut self, water_full: bool) -> Self {
        self.water_full = water_full;
        self
    }

    /// Provide the game-file resolver used to synthesize the border mesh and
    /// resolve the border shader + `map/terrain/border_*.dds` textures (BORD2).
    /// Required alongside `--borders-from-files`; if the flag is set without a
    /// resolver, the border pass fails closed. Builder-style.
    #[cfg(windows)]
    pub fn with_border_resolver(mut self, resolver: GameFileResolver) -> Self {
        self.border_resolver = Some(resolver);
        self
    }

    /// Enable/disable `--borders-from-files` mode (BORD2). Default is `false`.
    ///
    /// When enabled, the world-synthesis production-map path renders the
    /// synthesized thin-border mesh over the terrain target. Callers that leave
    /// this OFF keep byte-identical output. Intended to be set once, right after
    /// construction (e.g. from the replay_app CLI `--borders-from-files` flag).
    pub fn set_borders_from_files(&mut self, borders_from_files: bool) {
        self.borders_from_files = borders_from_files;
    }

    /// Builder-style variant of
    /// [`set_borders_from_files`](Self::set_borders_from_files).
    pub fn with_borders_from_files(mut self, borders_from_files: bool) -> Self {
        self.borders_from_files = borders_from_files;
        self
    }

    /// Provide the game-file resolver used to load `map/heightmap.bmp` for the
    /// generated terrain-chunk mesh (CHUNK1 `--chunks-from-files`). Required
    /// alongside the flag; if the flag is set without a resolver, the synthesis
    /// fails closed. Builder-style.
    #[cfg(windows)]
    pub fn with_chunks_from_files_resolver(mut self, resolver: GameFileResolver) -> Self {
        self.chunks_from_files_resolver = Some(resolver);
        self
    }

    /// Enable/disable `--chunks-from-files` mode (CHUNK1). Default is `false`.
    ///
    /// When enabled, the terrain-chunk synthesis frame binds generated mesh
    /// buffers (no captured chunk template file needed). Callers that leave this
    /// OFF keep byte-identical output. Intended to be set once, right after
    /// construction (e.g. from the replay_app CLI `--chunks-from-files` flag).
    pub fn set_chunks_from_files(&mut self, chunks_from_files: bool) {
        self.chunks_from_files = chunks_from_files;
    }

    /// Builder-style variant of
    /// [`set_chunks_from_files`](Self::set_chunks_from_files).
    pub fn with_chunks_from_files(mut self, chunks_from_files: bool) -> Self {
        self.chunks_from_files = chunks_from_files;
        self
    }

    pub fn prepare(
        &self,
        bundle: &NativeD3d11TerrainBundle,
    ) -> Result<NativeD3d11PreparedTerrainBundle> {
        let native = unsafe {
            native_d3d11_prepare_map_frame(&self.device, &bundle.frame, &bundle.bundle_dir, None)?
        };
        let summary = native.pass.summary.clone();
        Ok(NativeD3d11PreparedTerrainBundle {
            draw_count: summary.draw_count,
            target_width: summary.target_width,
            target_height: summary.target_height,
            target_format: summary.target_format,
            native,
        })
    }

    pub fn prepare_production_terrain(
        &self,
        bundle: &NativeD3d11TerrainBundle,
        graph: &ProductionMapPassGraph,
    ) -> Result<NativeD3d11PreparedTerrainBundle> {
        graph.validate_terrain_bundle(bundle)?;
        let native = unsafe {
            native_d3d11_prepare_map_frame(
                &self.device,
                &bundle.frame,
                &bundle.bundle_dir,
                Some(graph.camera),
            )?
        };
        let summary = native.pass.summary.clone();
        Ok(NativeD3d11PreparedTerrainBundle {
            draw_count: summary.draw_count,
            target_width: summary.target_width,
            target_height: summary.target_height,
            target_format: summary.target_format,
            native,
        })
    }

    pub fn prepare_production_map(
        &self,
        bundle: &NativeD3d11TerrainBundle,
        camera: Option<ProductionMapCamera>,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        let camera = match camera {
            Some(camera) => camera,
            None => bundle.production_camera_from_capture()?,
        };
        let graph = ProductionMapPassGraph::from_terrain_capture(bundle, camera)?;
        let terrain = self.prepare_production_terrain(bundle, &graph)?;
        Ok(NativeD3d11PreparedProductionMap {
            graph,
            terrain,
            water: None,
            rivers: None,
            borders: None,
            trees: None,
            // Plain/reference prepare: the synthesized border pass rides only
            // the with_layers production path (reference replays must stay
            // byte-exact against the capture).
            borders_files: None,
        })
    }

    pub fn prepare_terrain_chunk_library(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedTerrainBundle> {
        let frame = library.frame_for_camera(camera)?;
        let native = unsafe {
            native_d3d11_prepare_terrain_chunk_library_frame(
                &self.device,
                &frame,
                &library.bundle_dir,
                camera,
            )?
        };
        let summary = native.pass.summary.clone();
        Ok(NativeD3d11PreparedTerrainBundle {
            draw_count: summary.draw_count,
            target_width: summary.target_width,
            target_height: summary.target_height,
            target_format: summary.target_format,
            native,
        })
    }

    pub fn render_terrain_chunk_library(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
        options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        let prepared = self.prepare_terrain_chunk_library(library, camera)?;
        let options = NativeD3d11RenderOptions {
            output: options.output.clone(),
            readback: options.readback,
            compare_each_draw: false,
            compare_final_reference: false,
            parity_report: None,
            cull_world_rect: None,
            clear_targets: options.clear_targets,
            tree_season_cb: None,
            map_clock_time: None,
        };
        self.render_prepared(&prepared, &options)
    }

    pub fn prepare_terrain_chunk_synthesis(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedTerrainChunkSynthesis> {
        self.prepare_terrain_chunk_synthesis_with_detail_flags(
            library,
            camera,
            target_tile_size,
            world_rect,
            false,
            scene_cb0,
        )
    }

    fn prepare_terrain_chunk_synthesis_validation(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
    ) -> Result<NativeD3d11PreparedTerrainChunkSynthesis> {
        // Byte-exact record validation must replay each record's own captured
        // frame state verbatim, so no canonical scene CB0 is substituted here.
        self.prepare_terrain_chunk_synthesis_with_detail_flags(
            library,
            camera,
            target_tile_size,
            world_rect,
            true,
            None,
        )
    }

    fn prepare_terrain_chunk_synthesis_with_detail_flags(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        use_captured_detail_flags: bool,
        scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedTerrainChunkSynthesis> {
        let plan = if use_captured_detail_flags {
            terrain_chunk_synthesis_validation_plan(library, target_tile_size, world_rect)?
        } else {
            terrain_chunk_synthesis_plan(library, target_tile_size, world_rect)?
        };
        // CHUNK1: under --chunks-from-files, regenerate the chunk mesh buffers
        // (slot0 grid, index tessellation, per-LOD slot1 height pool, CB1) from
        // the shipped heightmap + vertex law and bind them as synthetic
        // payloads, so the template draw's VB/IB/CB1 come from files not the
        // captured chunk buffers. Flag-OFF leaves the map empty => the frame
        // reads the captured template bytes byte-identically to before.
        let synthetic_payloads = self.terrain_chunk_synthetic_payloads(library, world_rect)?;
        let native = unsafe {
            native_d3d11_prepare_terrain_chunk_synthesis_frame(
                &self.device,
                &plan,
                library.bundle_dir(),
                camera,
                scene_cb0,
                &synthetic_payloads,
            )?
        };
        let target_width = native.render_target.desc.width;
        let target_height = native.render_target.desc.height;
        let target_format = native.render_target.desc.format;
        Ok(NativeD3d11PreparedTerrainChunkSynthesis {
            draw_count: plan.tile_count(),
            target_width,
            target_height,
            target_format,
            plan,
            native,
        })
    }

    /// CHUNK1: build the generated chunk-mesh synthetic payloads when
    /// `--chunks-from-files` is active, or an empty map (flag-OFF, byte-
    /// identical) otherwise. Fails closed if the flag is set without a resolver.
    fn terrain_chunk_synthetic_payloads(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        world_rect: TerrainChunkSynthesisWorldRect,
    ) -> Result<BTreeMap<String, MapFrameSyntheticPayload>> {
        if !self.chunks_from_files {
            return Ok(BTreeMap::new());
        }
        let resolver = self.chunks_from_files_resolver.as_ref().ok_or(
            "--chunks-from-files blocked: pass --game <the game dir> so map/heightmap.bmp resolves",
        )?;
        let heightmap_path =
            resolver.resolve_required(TERRAIN_CHUNK_HEIGHTMAP_SOURCE, "heightmap.bmp")?;
        let heightmap = load_bmp_indexed(&heightmap_path)?;
        let world_width = (world_rect.max_x - world_rect.min_x).round();
        let world_height = (world_rect.max_z - world_rect.min_z).round();
        if world_width <= 0.0 || world_height <= 0.0 {
            return Err("--chunks-from-files blocked: world rect has non-positive extent".into());
        }
        build_terrain_chunk_synthetic_payloads(
            library,
            &heightmap,
            world_width as u32,
            world_height as u32,
        )
    }

    pub fn render_prepared_terrain_chunk_synthesis(
        &self,
        prepared: &NativeD3d11PreparedTerrainChunkSynthesis,
        options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        native_d3d11_render_prepared_terrain_chunk_synthesis(
            &self.device,
            &prepared.native,
            &prepared.plan,
            options.clear_targets,
            options.output.as_deref(),
            options.readback,
            options.cull_world_rect,
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_layers(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        self.prepare_terrain_chunk_synthesis_with_generated_layers(
            library,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            target_tile_size,
            world_rect,
            None,
            scene_cb0,
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_generated_layers(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        layer_synthesis: Option<&MapLayerSynthesisInputs>,
        scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        self.prepare_terrain_chunk_synthesis_with_generated_layers_and_tree_season_patch(
            library,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            target_tile_size,
            world_rect,
            layer_synthesis,
            scene_cb0,
            false,
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_generated_layers_and_tree_season_patch(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        layer_synthesis: Option<&MapLayerSynthesisInputs>,
        scene_cb0: Option<&[u8]>,
        patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        self.prepare_terrain_chunk_synthesis_with_generated_layers_tree_files_and_tree_season_patch(
            library,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            target_tile_size,
            world_rect,
            layer_synthesis,
            None,
            scene_cb0,
            patch_tree_season_cb1,
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_generated_layers_tree_files_and_tree_season_patch(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        layer_synthesis: Option<&MapLayerSynthesisInputs>,
        tree_file_layer: Option<&TreeFileDerivedLayer>,
        scene_cb0: Option<&[u8]>,
        patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        let mut graph = ProductionMapPassGraph::from_terrain_chunk_library(library, camera)?;
        if let Some(water_bundle) = water_bundle {
            graph = graph.with_water_capture(water_bundle)?;
        }
        if let Some(rivers_bundle) = rivers_bundle {
            graph = graph.with_rivers_capture(rivers_bundle)?;
        }
        if let Some(borders_bundle) = borders_bundle {
            graph = graph.with_borders_capture(borders_bundle)?;
        }
        if let Some(trees_bundle) = trees_bundle {
            graph = graph.with_trees_capture(trees_bundle)?;
        }
        let terrain = self.prepare_terrain_chunk_synthesis(
            library,
            camera,
            target_tile_size,
            world_rect,
            scene_cb0,
        )?;
        // WATER3: refraction/reflection source policy for the chunk-synthesis
        // production path. With --water-full the water and rivers passes bind
        // the REAL seafloor render (WATER2 underwater pass, rendered below over
        // the synthesized tiles) as WaterRefraction and the real reflection
        // cube. Without --water-full, product replay is blocked before water is
        // rendered when a water bundle is present. The selection is the pure
        // `world_synthesis_water_refraction_source` /
        // `world_synthesis_reflection_cube_source` law so it is unit-testable
        // without a GPU (see the tests) and stays in lockstep with the
        // captured-terrain path's non-reference selection.
        let water_refraction_source = world_synthesis_water_refraction_source(self.water_full);
        // Reflection-cube policy (WATER1), a separate axis from refraction:
        // honour self.water_full for the water/rivers/trees passes. Trees have
        // no ReflectionCubeMap slot, so this is inert there, but keep it
        // consistent.
        let reflection_cube_source = world_synthesis_reflection_cube_source(self.water_full);
        let water = self.prepare_optional_production_map_pass_with_inputs(
            water_bundle,
            Some(graph.camera),
            false,
            water_refraction_source,
            reflection_cube_source,
            false,
        )?;
        let mut rivers = self.prepare_optional_production_map_pass_with_inputs(
            rivers_bundle,
            Some(graph.camera),
            false,
            water_refraction_source,
            reflection_cube_source,
            false,
        )?;
        let borders = self.prepare_optional_production_map_pass_with_inputs(
            borders_bundle,
            Some(graph.camera),
            false,
            water_refraction_source,
            reflection_cube_source,
            false,
        )?;
        let mut trees = self.prepare_optional_production_map_pass_with_inputs(
            trees_bundle,
            Some(graph.camera),
            false,
            NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot,
            reflection_cube_source,
            patch_tree_season_cb1,
        )?;
        if let Some(inputs) = layer_synthesis {
            if let (Some(bundle), Some(pass)) = (rivers_bundle, rivers.as_mut()) {
                unsafe {
                    native_d3d11_patch_generated_river_pass(&self.device, bundle, pass, inputs)?;
                }
            }
            if tree_file_layer.is_none() {
                if let (Some(bundle), Some(pass)) = (trees_bundle, trees.as_mut()) {
                    unsafe {
                        native_d3d11_patch_generated_tree_pass(&self.device, bundle, pass, inputs)?;
                    }
                }
            }
        }
        if let Some(tree_file_layer) = tree_file_layer {
            if let (Some(bundle), Some(pass)) = (trees_bundle, trees.as_mut()) {
                unsafe {
                    native_d3d11_patch_tree_pass_from_files(
                        &self.device,
                        bundle,
                        pass,
                        tree_file_layer,
                    )?;
                }
            }
        }
        // BORD2 (--borders-from-files): prepare the synthesized border pass
        // once, fail-closed without a resolver. Flag OFF: no work, no change.
        let borders_files = match self.border_files_resolver_for_prepare()? {
            Some(resolver) => Some(native_d3d11_prepare_border_files_pass(
                &self.device,
                resolver,
            )?),
            None => None,
        };
        Ok(NativeD3d11PreparedSynthesizedProductionMap {
            graph,
            terrain,
            water,
            rivers,
            borders,
            trees,
            borders_files,
        })
    }

    pub fn render_prepared_synthesized_production_map(
        &self,
        prepared: &NativeD3d11PreparedSynthesizedProductionMap,
        options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        let dependent_passes = prepared.dependent_passes_in_order();
        // BORD2 (--borders-from-files): same overlay position as the
        // captured-terrain path — after water/rivers, before trees (diag23b
        // pass order; BORDER_LAW.md).
        let border_insert_index = prepared.borders_files.as_ref().map(|_| {
            dependent_passes
                .iter()
                .position(|(kind, _)| *kind == ProductionMapPassKind::Trees)
                .unwrap_or(dependent_passes.len())
        });
        // Overlay-after-final-output combinations fail closed (see the
        // captured-path hook for the rationale). This also covers the
        // no-dependent-passes case, where the terrain render itself would take
        // the output before the overlay.
        if border_insert_index == Some(dependent_passes.len())
            && (options.output.is_some() || options.readback || options.compare_final_reference)
        {
            return Err(
                "--borders-from-files border pass blocked: this synthesized frame has no \
                 trees pass, so the border overlay renders after the final pass, but the \
                 render options request output/readback/comparison that would be captured \
                 before the overlay; drop the direct output options (the replay surface \
                 reads the composited texture after the render)"
                    .into(),
            );
        }
        let terrain_options = NativeD3d11RenderOptions {
            output: dependent_passes
                .is_empty()
                .then(|| options.output.clone())
                .flatten(),
            readback: dependent_passes.is_empty() && options.readback,
            compare_each_draw: false,
            compare_final_reference: false,
            parity_report: options
                .parity_report
                .as_ref()
                .map(|path| path.join("terrain_synthesis")),
            cull_world_rect: options.cull_world_rect,
            clear_targets: options.clear_targets,
            tree_season_cb: None,
            map_clock_time: options.map_clock_time,
        };
        // WATER3 (--water-full): render the REAL seafloor pass over the
        // SYNTHESIZED terrain and bind it as WaterRefraction — the same
        // mechanism-correct pass the captured-terrain path uses
        // (render_prepared_production_map), but issued through the tiled
        // synthesis draw stream. Re-render the synthesis template draw with
        // PixelShaderUnderwater into an offscreen B8G8R8A8_UNORM target (its
        // own depth), iterating the WHOLE tile plan. Fail-closed if no resolver
        // was threaded in (no game-file access to reconstruct pdxmap.shader
        // Effect underwater). Without --water-full, product replay is blocked
        // before water rendering when a water bundle is present.
        //
        // ORDER (CB1 safety): this runs BEFORE the terrain color synthesis
        // render below. `native_d3d11_render_underwater_synthesis` rewrites the
        // shared `template_draw` CB1 (QuadOffset/detail-edge) per tile over the
        // whole plan; the terrain color render then runs LAST and leaves CB1 at
        // its own last visible tile, exactly as it does flag-OFF. So the border
        // overlay (which borrows the template draw's CB1) sees the identical
        // terminal CB1 state whether or not --water-full is set — no new CB1
        // corruption is introduced. The underwater pass writes only its own
        // offscreen target/depth, so running it first does not disturb the
        // terrain HDR target.
        // Only render the seafloor when a dependent pass will consume it (the
        // water/rivers pass); a frame with no dependent passes has no
        // WaterRefraction slot to bind, so skip the work entirely.
        let underwater_pass = if self.water_full && !dependent_passes.is_empty() {
            let resolver = self.underwater_resolver.as_ref().ok_or(
                "--water-full underwater pass blocked: no game-file resolver was provided to \
                 the renderer (needed to reconstruct pdxmap.shader Effect underwater); call \
                 with_underwater_resolver",
            )?;
            let synthesis = &prepared.terrain.native;
            let pass = native_d3d11_prepare_underwater_pass(
                &self.device,
                resolver,
                &synthesis.target_evidence,
                &synthesis.depth_evidence,
                std::slice::from_ref(&synthesis.template_draw),
            )?;
            native_d3d11_render_underwater_synthesis(
                &self.device,
                &pass,
                synthesis,
                &prepared.terrain.plan,
            )?;
            // DEBUG (env-gated, ocean-defect diagnosis): read back the product
            // path's WaterRefraction buffer (the underwater seafloor render) so
            // its pattern amplitude can be measured offline against the
            // engine's captured refraction buffer.
            if let Some(path) = std::env::var_os("REPLAY_DEBUG_DUMP_UNDERWATER") {
                let readback = unsafe {
                    crate::d3d11_device::native_d3d11_read_render_target(
                        &self.device,
                        &self.device.context,
                        &pass.render_target,
                    )?
                };
                std::fs::write(&path, &readback.bytes)?;
                let ps_path = std::path::PathBuf::from(&path).with_extension("ps.bin");
                std::fs::write(&ps_path, pass.ps_bytecode())?;
                eprintln!(
                    "UNDERWATER-DEBUG dumped refraction buffer {}x{} fmt {} -> {} (+ compiled PS {} bytes -> {})",
                    pass.render_target.desc.width,
                    pass.render_target.desc.height,
                    pass.render_target.desc.format,
                    std::path::Path::new(&path).display(),
                    pass.ps_bytecode().len(),
                    ps_path.display()
                );
            }
            Some(pass)
        } else {
            None
        };
        let terrain_result =
            self.render_prepared_terrain_chunk_synthesis(prepared.terrain(), &terrain_options)?;
        if dependent_passes.is_empty() {
            self.render_border_files_overlay_synthesized(prepared, options)?;
            return Ok(terrain_result);
        }
        let terrain_color_refraction_desc = dependent_passes
            .iter()
            .find_map(|(_, pass)| pass.app_owned_terrain_color_srv_desc.as_ref())
            .cloned();
        let terrain_color_snapshot = terrain_color_refraction_desc
            .as_ref()
            .map(|desc| unsafe {
                native_d3d11_create_terrain_color_snapshot_srv(
                    &self.device,
                    &self.terrain_color_snapshot_pipeline,
                    &prepared.terrain.native.render_target,
                    desc,
                )
            })
            .transpose()?;
        let app_owned_resources = NativeD3d11AppOwnedShaderResources {
            terrain_color_snapshot: terrain_color_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.view),
            // WATER3: under --water-full the WaterRefraction slot is filled by
            // the real seafloor render over the synthesized tiles; flag-OFF
            // keeps the approximation refraction.
            underwater_refraction: underwater_pass
                .as_ref()
                .map(|pass| pass.shader_resource_view()),
        };
        // DEBUG (env-gated, ocean-defect diagnosis): clear the composite target
        // to magenta right before the dependent passes so the dump shows the
        // water/rivers contribution in isolation — where the water pass draws,
        // with what color, and where the (magenta) background leaks through its
        // blend alpha. Diagnosis-only; never set in gates.
        if std::env::var_os("REPLAY_DEBUG_CLEAR_BEFORE_WATER").is_some() {
            unsafe {
                (*self.device.context.as_raw()).ClearRenderTargetView(
                    prepared.terrain.native.render_target.view.as_raw(),
                    &[1.0, 0.0, 1.0, 1.0],
                );
            }
        }
        let mut result = None;
        let last_index = dependent_passes.len() - 1;
        for (index, (kind, pass)) in dependent_passes.iter().enumerate() {
            if border_insert_index == Some(index) {
                // BORD2: overlay draws here — after water/rivers, before trees.
                self.render_border_files_overlay_synthesized(prepared, options)?;
            }
            let final_pass = index == last_index;
            let pass_parity_report = options
                .parity_report
                .as_ref()
                .map(|path| path.join(kind.pass_label()));
            result = Some(native_d3d11_render_prepared_map_pass(
                &self.device,
                pass,
                &prepared.terrain.native.render_target,
                &prepared.terrain.native.depth_target,
                Some(&app_owned_resources),
                options.tree_season_cb,
                options.map_clock_time,
                false,
                final_pass.then_some(options.output.as_deref()).flatten(),
                final_pass && options.readback,
                options.compare_each_draw,
                final_pass && options.compare_final_reference,
                pass_parity_report.as_deref(),
            )?);
        }
        if border_insert_index == Some(dependent_passes.len()) {
            // No trees pass: the overlay draws last (output options that would
            // have missed it were failed closed above).
            self.render_border_files_overlay_synthesized(prepared, options)?;
        }
        result.ok_or_else(|| "production native D3D11 synthesized map graph did not execute".into())
    }

    /// BORD2: render the synthesized border overlay over a terrain-chunk
    /// synthesis frame. Borrows the synthesis template draw for the live scene
    /// CB0/CB1 (camera-updated by the synthesis scene updater) and viewport.
    #[cfg(windows)]
    fn render_border_files_overlay_synthesized(
        &self,
        prepared: &NativeD3d11PreparedSynthesizedProductionMap,
        options: &NativeD3d11RenderOptions,
    ) -> Result<()> {
        let Some(border_pass) = prepared.borders_files.as_ref() else {
            return Ok(());
        };
        native_d3d11_render_border_files_pass(
            &self.device,
            border_pass,
            &prepared.terrain.native.render_target,
            &prepared.terrain.native.depth_target,
            &prepared.terrain.native.template_draw,
            prepared.graph.camera.position[1],
            options.map_clock_time.unwrap_or(0.0),
        )
    }

    pub fn update_prepared_terrain_chunk_synthesis_camera(
        &self,
        prepared: &mut NativeD3d11PreparedTerrainChunkSynthesis,
        camera: ProductionMapCamera,
    ) -> Result<()> {
        self.update_prepared_terrain_chunk_synthesis_scene(prepared, camera, None)
    }

    pub fn update_prepared_terrain_chunk_synthesis_scene(
        &self,
        prepared: &mut NativeD3d11PreparedTerrainChunkSynthesis,
        camera: ProductionMapCamera,
        day_night_hour: Option<f32>,
    ) -> Result<()> {
        self.update_prepared_terrain_chunk_synthesis_scene_with_light_rig(
            prepared,
            camera,
            day_night_hour,
            None,
            None,
        )
    }

    /// The virtual light rig `.x` longitudes are driven from the day-cycle law
    /// at `day_night_hour` (see `shader_meta::write_virtual_light_rig_fields`).
    /// `day_night_hour = None` keeps the captured rig verbatim (reference /
    /// byte-exact validation renders). The rig no longer follows the camera.
    /// `political = Some` writes the political-mapmode GB override/cutoffs and
    /// the shadow-fade height law (COL3, see `shader_meta::PoliticalSceneCb0`);
    /// `None` keeps the captured GB/shadow bytes verbatim.
    pub fn update_prepared_terrain_chunk_synthesis_scene_with_light_rig(
        &self,
        prepared: &mut NativeD3d11PreparedTerrainChunkSynthesis,
        camera: ProductionMapCamera,
        day_night_hour: Option<f32>,
        map_clock_time: Option<f32>,
        political: Option<PoliticalSceneCb0>,
    ) -> Result<()> {
        camera.validate()?;
        unsafe {
            native_d3d11_update_prepared_draw_scene(
                &self.device.context,
                &prepared.native.template_draw,
                camera,
                day_night_hour,
                map_clock_time,
                political,
                "terrain chunk synthesis",
            )?;
        }
        Ok(())
    }

    pub fn update_prepared_synthesized_production_map_scene(
        &self,
        prepared: &mut NativeD3d11PreparedSynthesizedProductionMap,
        camera: ProductionMapCamera,
        day_night_hour: Option<f32>,
        map_clock_time: Option<f32>,
        political: Option<PoliticalSceneCb0>,
    ) -> Result<()> {
        self.update_prepared_terrain_chunk_synthesis_scene_with_light_rig(
            &mut prepared.terrain,
            camera,
            day_night_hour,
            map_clock_time,
            political,
        )?;
        unsafe {
            if let Some(water) = prepared.water.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    water,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "water",
                )?;
            }
            if let Some(rivers) = prepared.rivers.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    rivers,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "rivers",
                )?;
            }
            if let Some(borders) = prepared.borders.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    borders,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "borders",
                )?;
            }
            if let Some(trees) = prepared.trees.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    trees,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "trees",
                )?;
            }
        }
        prepared.graph.camera = camera;
        Ok(())
    }

    pub fn render_terrain_chunk_synthesis(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        camera: ProductionMapCamera,
        target_tile_size: u32,
        world_rect: TerrainChunkSynthesisWorldRect,
        options: &NativeD3d11RenderOptions,
        scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11ReplayResult> {
        let prepared = self.prepare_terrain_chunk_synthesis(
            library,
            camera,
            target_tile_size,
            world_rect,
            scene_cb0,
        )?;
        self.render_prepared_terrain_chunk_synthesis(&prepared, options)
    }

    pub fn prepare_production_map_with_water(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
        water_bundle: &NativeD3d11TerrainBundle,
        camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        self.prepare_production_map_with_layers(
            terrain_bundle,
            Some(water_bundle),
            None,
            None,
            None,
            camera,
        )
    }

    pub fn prepare_production_map_with_layers(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        self.prepare_production_map_with_layers_for_reference(
            terrain_bundle,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            false,
            false,
        )
    }

    pub fn prepare_production_map_with_layers_and_tree_season_patch(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        self.prepare_production_map_with_layers_for_reference(
            terrain_bundle,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            false,
            true,
        )
    }

    pub fn prepare_production_map_with_layers_tree_files_and_tree_season_patch(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
        tree_file_layer: Option<&TreeFileDerivedLayer>,
        patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        let mut prepared = self.prepare_production_map_with_layers_for_reference(
            terrain_bundle,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            false,
            patch_tree_season_cb1,
        )?;
        if let Some(tree_file_layer) = tree_file_layer {
            if let (Some(bundle), Some(pass)) = (trees_bundle, prepared.trees.as_mut()) {
                unsafe {
                    native_d3d11_patch_tree_pass_from_files(
                        &self.device,
                        bundle,
                        pass,
                        tree_file_layer,
                    )?;
                }
            }
        }
        Ok(prepared)
    }

    pub fn prepare_reference_production_map_with_layers(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        self.prepare_production_map_with_layers_for_reference(
            terrain_bundle,
            water_bundle,
            rivers_bundle,
            borders_bundle,
            trees_bundle,
            camera,
            true,
            false,
        )
    }

    fn prepare_production_map_with_layers_for_reference(
        &self,
        terrain_bundle: &NativeD3d11TerrainBundle,
        water_bundle: Option<&NativeD3d11TerrainBundle>,
        rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        borders_bundle: Option<&NativeD3d11TerrainBundle>,
        trees_bundle: Option<&NativeD3d11TerrainBundle>,
        camera: ProductionMapCamera,
        reference_replay: bool,
        patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        let mut graph = ProductionMapPassGraph::from_terrain_capture(terrain_bundle, camera)?;
        if let Some(water_bundle) = water_bundle {
            graph = graph.with_water_capture(water_bundle)?;
        }
        if let Some(rivers_bundle) = rivers_bundle {
            graph = graph.with_rivers_capture(rivers_bundle)?;
        }
        if let Some(borders_bundle) = borders_bundle {
            graph = graph.with_borders_capture(borders_bundle)?;
        }
        if let Some(trees_bundle) = trees_bundle {
            graph = graph.with_trees_capture(trees_bundle)?;
        }
        let terrain = self.prepare_production_terrain(terrain_bundle, &graph)?;
        let pass_camera = (!reference_replay).then_some(graph.camera);
        let restore_pre_draw_outputs = reference_replay;
        // Refraction (WaterRefraction slot 5) source policy:
        // * reference replay binds the captured buffer (byte-exact parity);
        // * world synthesis + --water-full renders the REAL seafloor pass
        //   (WATER2 underwater), mechanism-correct;
        // * world synthesis without --water-full is blocked when a water pass is
        //   present because the old app-owned approximation is not engine data.
        let (water_refraction_source, reflection_cube_source) =
            product_water_source_policy(reference_replay, self.water_full, water_bundle.is_some())?;
        // Reflection-cube policy (WATER1, separate axis from refraction). The
        // reference path already binds the real captured cube. The world-
        // synthesis path may not use the old mean-color flatten when a water
        // pass is present; --water-full (self.water_full) uploads the real cube
        // (byte-identical to the shipped map/terrain/reflection.dds).
        let water = self.prepare_optional_production_map_pass_with_inputs(
            water_bundle,
            pass_camera,
            restore_pre_draw_outputs,
            water_refraction_source,
            reflection_cube_source,
            false,
        )?;
        let rivers = self.prepare_optional_production_map_pass_with_inputs(
            rivers_bundle,
            pass_camera,
            restore_pre_draw_outputs,
            water_refraction_source,
            reflection_cube_source,
            false,
        )?;
        let borders = self.prepare_optional_production_map_pass_with_inputs(
            borders_bundle,
            pass_camera,
            restore_pre_draw_outputs,
            water_refraction_source,
            reflection_cube_source,
            false,
        )?;
        let trees = self.prepare_optional_production_map_pass_with_inputs(
            trees_bundle,
            pass_camera,
            restore_pre_draw_outputs,
            water_refraction_source,
            reflection_cube_source,
            patch_tree_season_cb1,
        )?;
        // BORD2 (--borders-from-files): prepare the synthesized border pass
        // once per prepared map, fail-closed without a resolver. Never on the
        // reference-replay path (byte-exact captured comparator). Flag OFF:
        // no shader compile, no allocation — byte-identical.
        let borders_files = if reference_replay {
            // Reference replays are the byte-exact captured comparator; the
            // synthesized overlay never rides them.
            None
        } else {
            match self.border_files_resolver_for_prepare()? {
                Some(resolver) => Some(native_d3d11_prepare_border_files_pass(
                    &self.device,
                    resolver,
                )?),
                None => None,
            }
        };
        Ok(NativeD3d11PreparedProductionMap {
            graph,
            terrain,
            water,
            rivers,
            borders,
            trees,
            borders_files,
        })
    }

    pub(crate) fn prepare_optional_production_map_pass_with_inputs(
        &self,
        bundle: Option<&NativeD3d11TerrainBundle>,
        camera: Option<ProductionMapCamera>,
        restore_pre_draw_outputs: bool,
        water_refraction_source: NativeD3d11WaterRefractionSource,
        reflection_cube_source: NativeD3d11ReflectionCubeSource,
        patch_tree_season_cb1: bool,
    ) -> Result<Option<NativeD3d11PreparedMapPass>> {
        bundle
            .map(|bundle| unsafe {
                native_d3d11_prepare_map_pass(
                    &self.device,
                    &bundle.frame,
                    &bundle.bundle_dir,
                    camera,
                    restore_pre_draw_outputs,
                    water_refraction_source,
                    reflection_cube_source,
                    self.reflection_resolver.as_ref(),
                    patch_tree_season_cb1,
                )
            })
            .transpose()
    }

    pub fn render(
        &self,
        bundle: &NativeD3d11TerrainBundle,
        options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        let prepared = self.prepare_production_map(bundle, None)?;
        self.render_prepared_production_map(&prepared, options)
    }

    pub fn render_prepared(
        &self,
        prepared: &NativeD3d11PreparedTerrainBundle,
        options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        native_d3d11_render_prepared_map_frame(
            &self.device,
            &prepared.native,
            options.clear_targets,
            options.output.as_deref(),
            options.readback,
            options.compare_each_draw,
            options.compare_final_reference,
            options.parity_report.as_deref(),
        )
    }

    pub fn render_prepared_production_map(
        &self,
        prepared: &NativeD3d11PreparedProductionMap,
        options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        let dependent_passes = prepared.dependent_passes_in_order();
        // BORD2 (--borders-from-files): the synthesized border overlay draws
        // after water/rivers and BEFORE trees, matching the one captured pass
        // sequence (diag23b: terrain, water, rivers, borders, trees; see
        // BORDER_LAW.md). `border_insert_index` is the dependent-pass index the
        // overlay renders in front of (`len()` = after every captured pass).
        let border_insert_index = prepared.borders_files.as_ref().map(|_| {
            dependent_passes
                .iter()
                .position(|(kind, _)| *kind == ProductionMapPassKind::Trees)
                .unwrap_or(dependent_passes.len())
        });
        // When the overlay renders after the LAST captured pass, that pass has
        // already taken the requested output/readback/compare, which would
        // therefore miss the overlay. Fail closed instead of returning an
        // image without the borders (no current caller hits this: the replay
        // surface passes output: None and reads the composited texture after
        // render() returns).
        if border_insert_index == Some(dependent_passes.len())
            && (options.output.is_some() || options.readback || options.compare_final_reference)
        {
            return Err(
                "--borders-from-files border pass blocked: this frame has no trees pass, so \
                 the border overlay renders after the final captured pass, but the render \
                 options request output/readback/comparison that would be captured before \
                 the overlay; drop the direct output options (the replay surface reads the \
                 composited texture after the render)"
                    .into(),
            );
        }
        if dependent_passes.is_empty() {
            if prepared.borders_files.is_some() {
                let result = self.render_prepared(prepared.terrain(), options)?;
                self.render_border_files_overlay_captured(prepared, options)?;
                return Ok(result);
            }
            return self.render_prepared(prepared.terrain(), options);
        }
        let terrain_options = NativeD3d11RenderOptions {
            output: None,
            readback: false,
            compare_each_draw: options.compare_each_draw,
            compare_final_reference: false,
            cull_world_rect: None,
            clear_targets: options.clear_targets,
            parity_report: options
                .parity_report
                .as_ref()
                .map(|path| path.join("terrain")),
            tree_season_cb: None,
            map_clock_time: options.map_clock_time,
        };
        self.render_prepared(prepared.terrain(), &terrain_options)?;
        let terrain_color_refraction_desc = dependent_passes
            .iter()
            .find_map(|(_, pass)| pass.app_owned_terrain_color_srv_desc.as_ref())
            .cloned();
        let terrain_color_snapshot = terrain_color_refraction_desc
            .as_ref()
            .map(|desc| unsafe {
                native_d3d11_create_terrain_color_snapshot_srv(
                    &self.device,
                    &self.terrain_color_snapshot_pipeline,
                    &prepared.terrain.native.render_target,
                    desc,
                )
            })
            .transpose()?;
        // WATER2 (--water-full): render the REAL seafloor pass. Re-render the
        // captured terrain draws with PixelShaderUnderwater into an offscreen
        // B8G8R8A8_UNORM target and bind it as WaterRefraction. Fail-closed if
        // no resolver was threaded in (no game-file access to reconstruct the
        // shader). Without --water-full, product replay is blocked before water
        // rendering when a water bundle is present.
        let underwater_pass = if self.water_full {
            let resolver = self.underwater_resolver.as_ref().ok_or(
                "--water-full underwater pass blocked: no game-file resolver was provided to \
                 the renderer (needed to reconstruct pdxmap.shader Effect underwater); call \
                 with_underwater_resolver",
            )?;
            let pass = native_d3d11_prepare_underwater_pass(
                &self.device,
                resolver,
                &prepared.terrain.native.pass.target_evidence,
                &prepared.terrain.native.pass.depth_evidence,
                &prepared.terrain.native.pass.draws,
            )?;
            native_d3d11_render_underwater_pass(
                &self.device,
                &pass,
                &prepared.terrain.native.pass.draws,
            )?;
            Some(pass)
        } else {
            None
        };
        let app_owned_resources = NativeD3d11AppOwnedShaderResources {
            terrain_color_snapshot: terrain_color_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.view),
            underwater_refraction: underwater_pass
                .as_ref()
                .map(|pass| pass.shader_resource_view()),
        };
        let mut result = None;
        let last_index = dependent_passes.len() - 1;
        for (index, (kind, pass)) in dependent_passes.iter().enumerate() {
            if border_insert_index == Some(index) {
                // BORD2: overlay draws here — after water/rivers, before trees.
                self.render_border_files_overlay_captured(prepared, options)?;
            }
            let final_pass = index == last_index;
            let pass_parity_report = options
                .parity_report
                .as_ref()
                .map(|path| path.join(kind.pass_label()));
            result = Some(native_d3d11_render_prepared_map_pass(
                &self.device,
                pass,
                &prepared.terrain.native.render_target,
                &prepared.terrain.native.depth_target,
                Some(&app_owned_resources),
                options.tree_season_cb,
                options.map_clock_time,
                false,
                final_pass.then_some(options.output.as_deref()).flatten(),
                final_pass && options.readback,
                options.compare_each_draw,
                final_pass && options.compare_final_reference,
                pass_parity_report.as_deref(),
            )?);
        }
        if border_insert_index == Some(dependent_passes.len()) {
            // No trees pass in this frame: the overlay draws last (the output
            // options that would have missed it were failed closed above).
            self.render_border_files_overlay_captured(prepared, options)?;
        }
        result.ok_or_else(|| "production native D3D11 map graph did not execute".into())
    }

    /// BORD2: the one fail-closed gate for `--borders-from-files` at prepare
    /// time. `Ok(None)` = flag off, no work (byte-identical path);
    /// `Ok(Some(resolver))` = flag on with a resolver; `Err` = flag on without
    /// a resolver (never a silent no-op).
    #[cfg(windows)]
    pub(crate) fn border_files_resolver_for_prepare(&self) -> Result<Option<&GameFileResolver>> {
        if !self.borders_from_files {
            return Ok(None);
        }
        match self.border_resolver.as_ref() {
            Some(resolver) => Ok(Some(resolver)),
            None => Err(
                "--borders-from-files border pass blocked: no game-file resolver was provided                  to the renderer (needed to synthesize the border mesh and resolve                  border.shader / border_*.dds); call with_border_resolver"
                    .into(),
            ),
        }
    }

    /// BORD2: render the synthesized border overlay over the captured-terrain
    /// production frame. Borrows the frame's first terrain draw for the live
    /// scene CB0/CB1 (camera-patched, including wrap-copy shifts) and the
    /// viewport; the fade uses the CURRENT `graph.camera` world-Y eye height
    /// (kept current by `update_prepared_production_map_scene`).
    #[cfg(windows)]
    fn render_border_files_overlay_captured(
        &self,
        prepared: &NativeD3d11PreparedProductionMap,
        options: &NativeD3d11RenderOptions,
    ) -> Result<()> {
        let Some(border_pass) = prepared.borders_files.as_ref() else {
            return Ok(());
        };
        let scene_draw = prepared.terrain.native.pass.draws.first().ok_or(
            "--borders-from-files border pass blocked: the terrain pass has no draws to \
             borrow the scene CB0/viewport from",
        )?;
        native_d3d11_render_border_files_pass(
            &self.device,
            border_pass,
            &prepared.terrain.native.render_target,
            &prepared.terrain.native.depth_target,
            scene_draw,
            prepared.graph.camera.position[1],
            options.map_clock_time.unwrap_or(0.0),
        )
    }

    /// WATER2 validation: render ONLY the underwater seafloor pass for a
    /// captured terrain frame, at the captured camera (reference mode — the
    /// draws carry the engine's exact CB0/CB1), and return the resulting
    /// `B8G8R8A8_UNORM` readback. This is the mechanism-correct `WaterRefraction`
    /// reconstruction; the caller compares it byte-for-byte to the captured
    /// buffer. `resolver` reconstructs `pdxmap.shader` `Effect underwater`.
    ///
    /// The pass is fully self-contained (clears its own RT to transparent black
    /// and depth to 1.0, then re-renders the terrain geometry with
    /// `PixelShaderUnderwater`), so no prior terrain color render is needed.
    #[cfg(windows)]
    pub fn render_underwater_validation(
        &self,
        bundle: &NativeD3d11TerrainBundle,
        resolver: &GameFileResolver,
    ) -> Result<UnderwaterValidationRender> {
        // Reference mode (app_camera None): the captured draws replay with the
        // engine's exact captured constant buffers → the pixels match what the
        // engine wrote to WaterRefraction.
        let prepared = self.prepare(bundle)?;
        let pass = &prepared.native.pass;
        let underwater = native_d3d11_prepare_underwater_pass(
            &self.device,
            resolver,
            &pass.target_evidence,
            &pass.depth_evidence,
            &pass.draws,
        )?;
        native_d3d11_render_underwater_pass(&self.device, &underwater, &pass.draws)?;
        let readback = unsafe {
            native_d3d11_read_render_target(
                &self.device,
                &self.device.context,
                &underwater.render_target,
            )?
        };
        Ok(UnderwaterValidationRender {
            width: underwater.width(),
            height: underwater.height(),
            dxgi_format: readback.dxgi_format,
            row_pitch: readback.row_pitch,
            bytes: readback.bytes,
            ps_bytecode: underwater.ps_bytecode().to_vec(),
        })
    }

    /// WATER2 validation via a terrain-chunk-library session. Unlike a raw
    /// terrain frame (whose seafloor textures are descriptor-only), the chunk
    /// library carries the resolved terrain textures AND the captured
    /// ShadowMap, so the underwater pass has ALL of its inputs. Renders the
    /// underwater seafloor for the library's chunks at the captured camera and
    /// returns the readback for comparison against that session's captured
    /// `WaterRefraction`.
    ///
    /// NOTE (oracle test): the chunk-library geometry is the SYNTHESIZED
    /// terrain (culled/LOD-selected chunk draws), which is not guaranteed to be
    /// the exact geometry the engine rasterized for its underwater pass. This
    /// is a strong reconstruction/oracle test, not a definitive byte-exact
    /// engine proof — see documents/WATER_LAW.md.
    #[cfg(windows)]
    pub fn render_underwater_validation_from_chunk_library(
        &self,
        library: &NativeD3d11TerrainChunkLibrary,
        target_tile_size: u32,
        scene_cb0: Option<&[u8]>,
        resolver: &GameFileResolver,
    ) -> Result<UnderwaterValidationRender> {
        // Use the terrain-chunk SYNTHESIS plan (tiled template draw) — the
        // proven-renderable path for a chunk library — with an optional
        // canonical scene CB0 (the captured camera/sun for byte-alignment with
        // the session's WaterRefraction). frame_for_camera's frustum cull is
        // avoided here; the synthesis plan tiles the whole covered world rect.
        let camera = library.production_camera_from_capture()?;
        let world_rect = library.world_rect();
        let prepared = self.prepare_terrain_chunk_synthesis(
            library,
            camera,
            target_tile_size,
            world_rect,
            scene_cb0,
        )?;
        let synthesis = &prepared.native;
        let underwater = native_d3d11_prepare_underwater_pass(
            &self.device,
            resolver,
            &synthesis.target_evidence,
            &synthesis.depth_evidence,
            std::slice::from_ref(&synthesis.template_draw),
        )?;
        native_d3d11_render_underwater_synthesis(
            &self.device,
            &underwater,
            synthesis,
            &prepared.plan,
        )?;
        let readback = unsafe {
            native_d3d11_read_render_target(
                &self.device,
                &self.device.context,
                &underwater.render_target,
            )?
        };
        Ok(UnderwaterValidationRender {
            width: underwater.width(),
            height: underwater.height(),
            dxgi_format: readback.dxgi_format,
            row_pitch: readback.row_pitch,
            bytes: readback.bytes,
            ps_bytecode: underwater.ps_bytecode().to_vec(),
        })
    }

    pub fn update_prepared_production_map_camera(
        &self,
        prepared: &mut NativeD3d11PreparedProductionMap,
        camera: ProductionMapCamera,
    ) -> Result<()> {
        self.update_prepared_production_map_scene(prepared, camera, None, None, None)
    }

    pub fn update_prepared_production_map_scene(
        &self,
        prepared: &mut NativeD3d11PreparedProductionMap,
        camera: ProductionMapCamera,
        day_night_hour: Option<f32>,
        map_clock_time: Option<f32>,
        political: Option<PoliticalSceneCb0>,
    ) -> Result<()> {
        camera.validate()?;
        unsafe {
            native_d3d11_update_prepared_map_pass_scene(
                &self.device.context,
                &prepared.terrain.native.pass,
                camera,
                day_night_hour,
                map_clock_time,
                political,
                "terrain",
            )?;
            if let Some(water) = prepared.water.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    water,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "water",
                )?;
            }
            if let Some(rivers) = prepared.rivers.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    rivers,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "rivers",
                )?;
            }
            if let Some(borders) = prepared.borders.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    borders,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "borders",
                )?;
            }
            if let Some(trees) = prepared.trees.as_ref() {
                native_d3d11_update_prepared_map_pass_scene(
                    &self.device.context,
                    trees,
                    camera,
                    day_night_hour,
                    map_clock_time,
                    political,
                    "trees",
                )?;
            }
        }
        prepared.graph.camera = camera;
        Ok(())
    }
}

#[cfg(windows)]
unsafe fn native_d3d11_patch_generated_river_pass(
    device: &NativeD3d11Device,
    bundle: &NativeD3d11TerrainBundle,
    pass: &mut NativeD3d11PreparedMapPass,
    _inputs: &MapLayerSynthesisInputs,
) -> Result<()> {
    // Measured capture separation: <= 3.3 units within a river, >= 12.1
    // units between rivers.
    const RIVER_WORLD_SEGMENT_BREAK_DISTANCE: f32 = 8.0;

    let draw = pass
        .draws
        .first_mut()
        .ok_or_else(|| "generated river synthesis blocked: captured river pass has no draw")?;
    let source_draw = bundle
        .frame
        .draws
        .iter()
        .find(|source| source.draw_index == draw.draw_index)
        .or_else(|| bundle.frame.draws.first())
        .ok_or_else(|| "generated river synthesis blocked: captured river bundle has no draw")?;
    let vertex_shader = source_draw
        .vertex_shader
        .as_ref()
        .ok_or_else(|| "generated river synthesis blocked: river draw has no vertex shader")?;
    let vs_bytes = native_d3d11_read_shader_bytecode(bundle.bundle_dir(), vertex_shader)?;
    let vertex_binding = source_draw.vertex_buffers.get("0").ok_or_else(|| {
        "generated river synthesis blocked: captured river draw has no IA vertex buffer slot 0"
    })?;
    let vertex_stride = vertex_binding.stride.ok_or_else(|| {
        "generated river synthesis blocked: captured river vertex buffer slot 0 has no stride"
    })?;
    if !draw.vertex_buffers.iter().any(|buffer| buffer.slot == 0) {
        return Err(
            "generated river synthesis blocked: prepared river pass has no IA vertex buffer slot 0"
                .into(),
        );
    }
    let vertex_bytes =
        native_d3d11_read_buffer_file(bundle.bundle_dir(), vertex_binding, "river vertex buffer")?;
    let indices = synthesize_river_world_indices(
        &vertex_bytes,
        vertex_stride as usize,
        RIVER_WORLD_SEGMENT_BREAK_DISTANCE,
    )?;
    draw.input_layout = native_d3d11_create_input_layout_from_elements(
        &device.device,
        &native_d3d11_river_input_layout_elements(),
        &vs_bytes,
    )?;
    draw.index_buffer = Some(native_d3d11_create_generated_index_buffer_u32(
        &device.device,
        &indices,
        "generated world river index buffer",
    )?);
    draw.draw_call = "DrawIndexed".to_string();
    draw.draw_count = indices.len().try_into().map_err(|_| {
        format!(
            "generated river synthesis blocked: {} indices exceed D3D11 u32 draw count",
            indices.len()
        )
    })?;
    draw.draw_start = 0;
    draw.base_vertex_location = 0;
    draw.per_instance_count = None;
    draw.instance_count = None;
    draw.start_instance_location = None;
    draw.parity_desc.draw_call = draw.draw_call.clone();
    draw.parity_desc.draw_count = draw.draw_count;
    draw.parity_desc.draw_start = draw.draw_start;
    draw.parity_desc.base_vertex_location = draw.base_vertex_location;
    draw.parity_desc.per_instance_count = draw.per_instance_count;
    draw.parity_desc.instance_count = draw.instance_count;
    draw.parity_desc.start_instance_location = draw.start_instance_location;
    draw.parity_desc.input_layout.elements = native_d3d11_river_input_layout_elements();
    native_d3d11_refresh_map_pass_summary(pass);
    Ok(())
}

#[cfg(windows)]
unsafe fn native_d3d11_patch_generated_tree_pass(
    device: &NativeD3d11Device,
    bundle: &NativeD3d11TerrainBundle,
    pass: &mut NativeD3d11PreparedMapPass,
    inputs: &MapLayerSynthesisInputs,
) -> Result<()> {
    let synthesis = synthesize_tree_instances(inputs)?;
    let draw = pass
        .draws
        .first_mut()
        .ok_or_else(|| "generated tree synthesis blocked: captured tree pass has no draw")?;
    let source_draw = bundle
        .frame
        .draws
        .iter()
        .find(|source| source.draw_index == draw.draw_index)
        .or_else(|| bundle.frame.draws.first())
        .ok_or_else(|| "generated tree synthesis blocked: captured tree bundle has no draw")?;
    let vertex_shader = source_draw
        .vertex_shader
        .as_ref()
        .ok_or_else(|| "generated tree synthesis blocked: tree draw has no vertex shader")?;
    let vs_bytes = native_d3d11_read_shader_bytecode(bundle.bundle_dir(), vertex_shader)?;
    draw.input_layout = native_d3d11_create_input_layout_from_elements(
        &device.device,
        &native_d3d11_tree_input_layout_elements(),
        &vs_bytes,
    )?;
    draw.vertex_buffers.retain(|buffer| buffer.slot != 1);
    draw.vertex_buffers
        .push(native_d3d11_create_generated_vertex_buffer(
            &device.device,
            1,
            TREE_INSTANCE_STRIDE_BYTES,
            &synthesis.instances,
            "generated world tree instance buffer",
        )?);
    draw.draw_call = "DrawIndexedInstanced".to_string();
    let per_instance_count = draw.per_instance_count.unwrap_or(draw.draw_count);
    let instance_count = synthesis.stats.instance_count.try_into().map_err(|_| {
        format!(
            "generated tree synthesis blocked: {} instances exceed D3D11 u32 draw count",
            synthesis.stats.instance_count
        )
    })?;
    draw.per_instance_count = Some(per_instance_count);
    draw.instance_count = Some(instance_count);
    draw.start_instance_location = Some(0);
    draw.draw_count = per_instance_count.checked_mul(instance_count).ok_or_else(|| {
        format!(
            "generated tree synthesis blocked: {per_instance_count} indices x {instance_count} instances exceeds u32"
        )
    })?;
    draw.draw_start = 0;
    draw.base_vertex_location = 0;
    draw.parity_desc.draw_call = draw.draw_call.clone();
    draw.parity_desc.draw_count = draw.draw_count;
    draw.parity_desc.draw_start = draw.draw_start;
    draw.parity_desc.base_vertex_location = draw.base_vertex_location;
    draw.parity_desc.per_instance_count = draw.per_instance_count;
    draw.parity_desc.instance_count = draw.instance_count;
    draw.parity_desc.start_instance_location = draw.start_instance_location;
    draw.parity_desc.input_layout.elements = native_d3d11_tree_input_layout_elements();
    native_d3d11_refresh_map_pass_summary(pass);
    Ok(())
}

#[cfg(windows)]
unsafe fn native_d3d11_patch_tree_pass_from_files(
    device: &NativeD3d11Device,
    bundle: &NativeD3d11TerrainBundle,
    pass: &mut NativeD3d11PreparedMapPass,
    layer: &TreeFileDerivedLayer,
) -> Result<()> {
    let draw = pass
        .draws
        .first_mut()
        .ok_or_else(|| "file-derived tree layer blocked: captured tree pass has no draw")?;
    let source_draw = bundle
        .frame
        .draws
        .iter()
        .find(|source| source.draw_index == draw.draw_index)
        .or_else(|| bundle.frame.draws.first())
        .ok_or_else(|| "file-derived tree layer blocked: captured tree bundle has no draw")?;
    let vertex_shader = source_draw
        .vertex_shader
        .as_ref()
        .ok_or_else(|| "file-derived tree layer blocked: tree draw has no vertex shader")?;
    let vs_bytes = native_d3d11_read_shader_bytecode(bundle.bundle_dir(), vertex_shader)?;
    draw.input_layout = native_d3d11_create_input_layout_from_elements(
        &device.device,
        &native_d3d11_tree_input_layout_elements(),
        &vs_bytes,
    )?;
    draw.vertex_buffers
        .retain(|buffer| buffer.slot != 0 && buffer.slot != 1);
    draw.vertex_buffers
        .push(native_d3d11_create_generated_vertex_buffer(
            &device.device,
            0,
            layer.mesh.vertex_stride_bytes,
            &layer.mesh.vertices,
            "file-derived tree mesh vertex buffer",
        )?);
    draw.vertex_buffers
        .push(native_d3d11_create_generated_vertex_buffer(
            &device.device,
            1,
            TREE_INSTANCE_STRIDE_BYTES,
            &layer.instances.instances,
            "file-derived tree instance buffer",
        )?);
    draw.vertex_buffers.sort_by_key(|buffer| buffer.slot);
    draw.index_buffer = Some(native_d3d11_create_generated_index_buffer_u32(
        &device.device,
        &layer.mesh.indices,
        "file-derived tree mesh index buffer",
    )?);
    replace_tree_texture_resource(device, draw, "TintMap", &layer.tint)?;
    replace_tree_texture_resource(device, draw, "SeasonMap", &layer.season)?;
    draw.draw_call = "DrawIndexedInstanced".to_string();
    let per_instance_count = u32::try_from(layer.mesh.indices.len()).map_err(|_| {
        format!(
            "file-derived tree layer blocked: {} indices exceed D3D11 u32 draw count",
            layer.mesh.indices.len()
        )
    })?;
    let instance_count = u32::try_from(layer.instances.stats.instance_count).map_err(|_| {
        format!(
            "file-derived tree layer blocked: {} instances exceed D3D11 u32 draw count",
            layer.instances.stats.instance_count
        )
    })?;
    draw.per_instance_count = Some(per_instance_count);
    draw.instance_count = Some(instance_count);
    draw.start_instance_location = Some(0);
    draw.draw_count = per_instance_count.checked_mul(instance_count).ok_or_else(|| {
        format!(
            "file-derived tree layer blocked: {per_instance_count} indices x {instance_count} instances exceeds u32"
        )
    })?;
    draw.draw_start = 0;
    draw.base_vertex_location = 0;
    draw.parity_desc.draw_call = draw.draw_call.clone();
    draw.parity_desc.draw_count = draw.draw_count;
    draw.parity_desc.draw_start = draw.draw_start;
    draw.parity_desc.base_vertex_location = draw.base_vertex_location;
    draw.parity_desc.per_instance_count = draw.per_instance_count;
    draw.parity_desc.instance_count = draw.instance_count;
    draw.parity_desc.start_instance_location = draw.start_instance_location;
    draw.parity_desc.input_layout.elements = native_d3d11_tree_input_layout_elements();
    native_d3d11_refresh_map_pass_summary(pass);
    Ok(())
}

#[cfg(windows)]
unsafe fn replace_tree_texture_resource(
    device: &NativeD3d11Device,
    draw: &mut NativeD3d11PreparedDraw,
    declared_name: &str,
    texture: &TreeFileTexture,
) -> Result<()> {
    let resource_index = draw
        .shader_resources
        .iter()
        .position(|resource| {
            resource.stage == "PS" && resource.declared_name.as_deref() == Some(declared_name)
        })
        .ok_or_else(|| {
            format!(
                "file-derived tree layer blocked: tree draw has no PS shader resource {declared_name}"
            )
        })?;
    let (native_texture, view, texture_desc, view_desc) = native_d3d11_create_b8g8r8x8_texture_srv(
        &device.device,
        texture.width,
        texture.height,
        &texture.bgra8,
        &format!("file-derived tree {declared_name}"),
    )?;
    draw._textures.push(native_texture);
    draw.shader_resources[resource_index].source =
        NativeD3d11ShaderResourceSource::CapturedBundleTexture;
    draw.shader_resources[resource_index].view = Some(view);
    if let Some(report) = draw.parity_desc.shader_resources.iter_mut().find(|report| {
        report.stage == "PS" && report.declared_name.as_deref() == Some(declared_name)
    }) {
        report.is_null = false;
        report.file = Some(texture.source.clone());
        report.fnv1a64 = Some(fnv1a64_bytes(&texture.bgra8));
        report.replay_texture_desc = texture_desc;
        report.replay_view_desc = Some(view_desc);
    }
    Ok(())
}

#[cfg(windows)]
fn native_d3d11_refresh_map_pass_summary(pass: &mut NativeD3d11PreparedMapPass) {
    pass.summary.draw_count = pass.draws.len();
    pass.summary.vertex_buffer_count = pass
        .draws
        .iter()
        .map(|draw| draw.vertex_buffers.len())
        .sum();
    pass.summary.index_buffer_count = pass
        .draws
        .iter()
        .filter(|draw| draw.index_buffer.is_some())
        .count();
    pass.summary.constant_buffer_count = pass
        .draws
        .iter()
        .map(|draw| draw.constant_buffers.len())
        .sum();
    pass.summary.shader_resource_count = pass
        .draws
        .iter()
        .map(|draw| draw.shader_resources.len())
        .sum();
    pass.summary.sampler_count = pass.draws.iter().map(|draw| draw.samplers.len()).sum();
}

#[cfg(not(windows))]
impl NativeD3d11TerrainRenderer {
    pub fn new() -> Result<Self> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    /// See the Windows implementation. No-op here (no renderer instance can
    /// exist off Windows); kept for cross-platform API parity with `--water-full`.
    pub fn set_water_full(&mut self, _water_full: bool) {}

    /// See the Windows implementation. Returns `self` unchanged off Windows.
    pub fn with_water_full(self, _water_full: bool) -> Self {
        self
    }

    /// See the Windows implementation. No-op here; kept for cross-platform API
    /// parity with `--borders-from-files`.
    pub fn set_borders_from_files(&mut self, _borders_from_files: bool) {}

    /// See the Windows implementation. Returns `self` unchanged off Windows.
    pub fn with_borders_from_files(self, _borders_from_files: bool) -> Self {
        self
    }

    /// See the Windows implementation. No-op here; kept for cross-platform API
    /// parity with `--chunks-from-files`.
    pub fn set_chunks_from_files(&mut self, _chunks_from_files: bool) {}

    /// See the Windows implementation. Returns `self` unchanged off Windows.
    pub fn with_chunks_from_files(self, _chunks_from_files: bool) -> Self {
        self
    }

    pub fn render(
        &self,
        _bundle: &NativeD3d11TerrainBundle,
        _options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare(
        &self,
        _bundle: &NativeD3d11TerrainBundle,
    ) -> Result<NativeD3d11PreparedTerrainBundle> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn render_prepared(
        &self,
        _prepared: &NativeD3d11PreparedTerrainBundle,
        _options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_production_terrain(
        &self,
        _bundle: &NativeD3d11TerrainBundle,
        _graph: &ProductionMapPassGraph,
    ) -> Result<NativeD3d11PreparedTerrainBundle> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_production_map(
        &self,
        _bundle: &NativeD3d11TerrainBundle,
        _camera: Option<ProductionMapCamera>,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_terrain_chunk_library(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedTerrainBundle> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn render_terrain_chunk_library(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _camera: ProductionMapCamera,
        _options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_terrain_chunk_synthesis(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
        _scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedTerrainChunkSynthesis> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    fn prepare_terrain_chunk_synthesis_validation(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
    ) -> Result<NativeD3d11PreparedTerrainChunkSynthesis> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn render_prepared_terrain_chunk_synthesis(
        &self,
        _prepared: &NativeD3d11PreparedTerrainChunkSynthesis,
        _options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn update_prepared_terrain_chunk_synthesis_camera(
        &self,
        _prepared: &mut NativeD3d11PreparedTerrainChunkSynthesis,
        _camera: ProductionMapCamera,
    ) -> Result<()> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn render_terrain_chunk_synthesis(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
        _options: &NativeD3d11RenderOptions,
        _scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11ReplayResult> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_layers(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
        _scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_generated_layers(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
        _layer_synthesis: Option<&MapLayerSynthesisInputs>,
        _scene_cb0: Option<&[u8]>,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_generated_layers_and_tree_season_patch(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
        _layer_synthesis: Option<&MapLayerSynthesisInputs>,
        _scene_cb0: Option<&[u8]>,
        _patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_terrain_chunk_synthesis_with_generated_layers_tree_files_and_tree_season_patch(
        &self,
        _library: &NativeD3d11TerrainChunkLibrary,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
        _target_tile_size: u32,
        _world_rect: TerrainChunkSynthesisWorldRect,
        _layer_synthesis: Option<&MapLayerSynthesisInputs>,
        _tree_file_layer: Option<&TreeFileDerivedLayer>,
        _scene_cb0: Option<&[u8]>,
        _patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedSynthesizedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_production_map_with_water(
        &self,
        _terrain_bundle: &NativeD3d11TerrainBundle,
        _water_bundle: &NativeD3d11TerrainBundle,
        _camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_production_map_with_layers_and_tree_season_patch(
        &self,
        _terrain_bundle: &NativeD3d11TerrainBundle,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_production_map_with_layers_tree_files_and_tree_season_patch(
        &self,
        _terrain_bundle: &NativeD3d11TerrainBundle,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
        _tree_file_layer: Option<&TreeFileDerivedLayer>,
        _patch_tree_season_cb1: bool,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_production_map_with_layers(
        &self,
        _terrain_bundle: &NativeD3d11TerrainBundle,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn prepare_reference_production_map_with_layers(
        &self,
        _terrain_bundle: &NativeD3d11TerrainBundle,
        _water_bundle: Option<&NativeD3d11TerrainBundle>,
        _rivers_bundle: Option<&NativeD3d11TerrainBundle>,
        _borders_bundle: Option<&NativeD3d11TerrainBundle>,
        _trees_bundle: Option<&NativeD3d11TerrainBundle>,
        _camera: ProductionMapCamera,
    ) -> Result<NativeD3d11PreparedProductionMap> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn render_prepared_production_map(
        &self,
        _prepared: &NativeD3d11PreparedProductionMap,
        _options: &NativeD3d11RenderOptions,
    ) -> Result<NativeD3d11ReplayResult> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn update_prepared_production_map_camera(
        &self,
        _prepared: &mut NativeD3d11PreparedProductionMap,
        _camera: ProductionMapCamera,
    ) -> Result<()> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }

    pub fn update_prepared_production_map_scene(
        &self,
        _prepared: &mut NativeD3d11PreparedProductionMap,
        _camera: ProductionMapCamera,
        _day_night_hour: Option<f32>,
        _map_clock_time: Option<f32>,
        _political: Option<PoliticalSceneCb0>,
    ) -> Result<()> {
        Err(
            "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
                .into(),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeD3d11RenderOptions {
    pub output: Option<PathBuf>,
    pub readback: bool,
    pub compare_each_draw: bool,
    pub compare_final_reference: bool,
    pub parity_report: Option<PathBuf>,
    /// Clear the render/depth targets before the pass. Defaults to `true`
    /// for existing single-copy/reference behavior; product wrap rendering
    /// disables it after the first copy so copies accumulate.
    pub clear_targets: bool,
    /// Optional world-space ground rect `[min_x, min_z, max_x, max_z]`
    /// restricting which synthesized terrain tiles are drawn (the camera's
    /// ground footprint, padded). `None` draws everything - the byte-exact
    /// validation and QA paths never set this. Tiles are tested with
    /// east-west wrap.
    pub cull_world_rect: Option<[f32; 4]>,
    /// Optional tree-pass CB1 override `(vSeasonLerp, raw vSeasonColumn)`.
    /// `None` preserves captured tree CB1 bytes. `Some` is the opt-in
    /// date-hypothesis path; the shader, not this option, converts the raw
    /// 0..7 column to `column / 8 + 1 / 16` season-map UVs.
    pub tree_season_cb: Option<(f32, f32)>,
    /// Optional interactive wall-clock seconds for live map animation.
    /// `None` preserves captured time bytes and is the byte-exact
    /// dump/reference default.
    pub map_clock_time: Option<f32>,
}

impl Default for NativeD3d11RenderOptions {
    fn default() -> Self {
        Self {
            output: None,
            readback: false,
            compare_each_draw: false,
            compare_final_reference: true,
            parity_report: None,
            clear_targets: true,
            cull_world_rect: None,
            tree_season_cb: None,
            map_clock_time: None,
        }
    }
}

impl NativeD3d11RenderOptions {
    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn with_readback(mut self) -> Self {
        self.readback = true;
        self
    }

    pub fn with_per_draw_comparison(mut self) -> Self {
        self.compare_each_draw = true;
        self
    }

    pub fn without_final_reference_comparison(mut self) -> Self {
        self.compare_final_reference = false;
        self
    }

    pub fn with_parity_report(mut self, report_dir: impl Into<PathBuf>) -> Self {
        self.parity_report = Some(report_dir.into());
        self
    }

    pub fn with_tree_season_cb(mut self, tree_season_cb: (f32, f32)) -> Self {
        self.tree_season_cb = Some(tree_season_cb);
        self
    }

    pub fn with_map_clock_time(mut self, map_clock_time: f32) -> Self {
        self.map_clock_time = Some(map_clock_time);
        self
    }
}

#[derive(Clone, Debug)]
pub struct NativeD3d11TerrainBundle {
    pub(crate) map_frame_bundle: PathBuf,
    pub(crate) bundle_dir: PathBuf,
    pub(crate) frame: MapFrameRuntimeBundle,
}

impl NativeD3d11TerrainBundle {
    pub fn load(map_frame_bundle: impl AsRef<Path>) -> Result<Self> {
        Self::load_for(map_frame_bundle, TERRAIN_MAP_FRAME_SPEC)
    }

    pub fn load_with_resolver(
        map_frame_bundle: impl AsRef<Path>,
        resolver: &GameFileResolver,
    ) -> Result<Self> {
        Self::load_for_with_resolver(map_frame_bundle, TERRAIN_MAP_FRAME_SPEC, Some(resolver))
    }

    pub fn load_water(map_frame_bundle: impl AsRef<Path>) -> Result<Self> {
        Self::load_for(map_frame_bundle, WATER_MAP_FRAME_SPEC)
    }

    pub fn load_rivers(map_frame_bundle: impl AsRef<Path>) -> Result<Self> {
        Self::load_for(map_frame_bundle, RIVERS_MAP_FRAME_SPEC)
    }

    pub fn load_borders(map_frame_bundle: impl AsRef<Path>) -> Result<Self> {
        Self::load_for(map_frame_bundle, BORDERS_MAP_FRAME_SPEC)
    }

    pub fn load_trees(map_frame_bundle: impl AsRef<Path>) -> Result<Self> {
        Self::load_for(map_frame_bundle, TREES_MAP_FRAME_SPEC)
    }

    pub(crate) fn load_for(
        map_frame_bundle: impl AsRef<Path>,
        spec: MapFrameBundleValidationSpec,
    ) -> Result<Self> {
        Self::load_for_with_resolver(map_frame_bundle, spec, None)
    }

    pub(crate) fn load_for_with_resolver(
        map_frame_bundle: impl AsRef<Path>,
        spec: MapFrameBundleValidationSpec,
        resolver: Option<&GameFileResolver>,
    ) -> Result<Self> {
        let map_frame_bundle = map_frame_bundle.as_ref().to_path_buf();
        let frame =
            validate_map_frame_runtime_bundle_for_with_resolver(&map_frame_bundle, spec, resolver)?;
        validate_native_d3d11_replay_payloads(&frame, false)?;
        let bundle_dir = map_frame_bundle_dir_for(&map_frame_bundle, spec)?;
        Ok(Self {
            map_frame_bundle,
            bundle_dir,
            frame,
        })
    }

    pub fn map_frame_bundle(&self) -> &Path {
        &self.map_frame_bundle
    }

    pub fn bundle_dir(&self) -> &Path {
        &self.bundle_dir
    }

    pub fn draw_count(&self) -> usize {
        self.frame.draws.len()
    }

    pub fn active_draw_index(&self) -> u32 {
        self.frame.active_draw.draw_index
    }

    pub fn production_camera_from_capture(&self) -> Result<ProductionMapCamera> {
        production_map_camera_from_terrain_bundle(self)
    }

    /// The frame's scene constant buffer (slot-0 `dx11_cb0`) bytes. This is the
    /// canonical per-frame scene state (sun direction, time-of-day, snow/mud
    /// fade, fog, camera) that every synthesized pass must share; terrain chunk
    /// templates were captured across different frames of the session and carry
    /// their own divergent scene state otherwise.
    pub fn scene_cb0_bytes(&self) -> Result<Vec<u8>> {
        let draw = self
            .frame
            .draws
            .first()
            .ok_or("production map frame scene CB0 blocked: map frame bundle has no draws")?;
        let binding = draw
            .constant_buffers
            .iter()
            .find(|binding| binding.slot == 0 && binding.stage == "VS")
            .or_else(|| {
                draw.constant_buffers
                    .iter()
                    .find(|binding| binding.slot == 0 && binding.stage == "PS")
            })
            .ok_or(
                "production map frame scene CB0 blocked: draw 0 has no slot-0 constant buffer",
            )?;
        read_map_frame_buffer_bytes(&self.bundle_dir, binding, "scene constant buffer")
    }

    pub fn render(&self, options: &NativeD3d11RenderOptions) -> Result<NativeD3d11ReplayResult> {
        NativeD3d11TerrainRenderer::new()?.render(self, options)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeD3d11ReplayRequest {
    pub map_frame_bundle: PathBuf,
    pub output: Option<PathBuf>,
    pub readback: bool,
    pub compare_each_draw: bool,
    pub production_camera: Option<ProductionMapCamera>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionMapD3d11ReplayRequest {
    pub terrain_map_frame_bundle: PathBuf,
    pub water_map_frame_bundle: Option<PathBuf>,
    pub rivers_map_frame_bundle: Option<PathBuf>,
    pub borders_map_frame_bundle: Option<PathBuf>,
    pub trees_map_frame_bundle: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub readback: bool,
    pub compare_each_draw: bool,
    pub production_camera: ProductionMapCamera,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainD3d11AcceptanceRequest {
    pub map_frame_bundle: PathBuf,
    pub output: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub repeat: usize,
}

impl NativeD3d11ReplayRequest {
    pub fn new(map_frame_bundle: impl Into<PathBuf>) -> Self {
        Self {
            map_frame_bundle: map_frame_bundle.into(),
            output: None,
            readback: false,
            compare_each_draw: false,
            production_camera: None,
        }
    }

    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn with_readback(mut self) -> Self {
        self.readback = true;
        self
    }

    pub fn with_per_draw_comparison(mut self) -> Self {
        self.compare_each_draw = true;
        self
    }

    pub fn with_production_camera(mut self, camera: ProductionMapCamera) -> Self {
        self.production_camera = Some(camera);
        self
    }
}

impl ProductionMapD3d11ReplayRequest {
    pub fn new(
        terrain_map_frame_bundle: impl Into<PathBuf>,
        production_camera: ProductionMapCamera,
    ) -> Self {
        Self {
            terrain_map_frame_bundle: terrain_map_frame_bundle.into(),
            water_map_frame_bundle: None,
            rivers_map_frame_bundle: None,
            borders_map_frame_bundle: None,
            trees_map_frame_bundle: None,
            output: None,
            readback: false,
            compare_each_draw: false,
            production_camera,
        }
    }

    pub fn with_water_map_frame_bundle(
        mut self,
        water_map_frame_bundle: impl Into<PathBuf>,
    ) -> Self {
        self.water_map_frame_bundle = Some(water_map_frame_bundle.into());
        self
    }

    pub fn with_rivers_map_frame_bundle(
        mut self,
        rivers_map_frame_bundle: impl Into<PathBuf>,
    ) -> Self {
        self.rivers_map_frame_bundle = Some(rivers_map_frame_bundle.into());
        self
    }

    pub fn with_borders_map_frame_bundle(
        mut self,
        borders_map_frame_bundle: impl Into<PathBuf>,
    ) -> Self {
        self.borders_map_frame_bundle = Some(borders_map_frame_bundle.into());
        self
    }

    pub fn with_trees_map_frame_bundle(
        mut self,
        trees_map_frame_bundle: impl Into<PathBuf>,
    ) -> Self {
        self.trees_map_frame_bundle = Some(trees_map_frame_bundle.into());
        self
    }

    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn with_readback(mut self) -> Self {
        self.readback = true;
        self
    }

    pub fn with_per_draw_comparison(mut self) -> Self {
        self.compare_each_draw = true;
        self
    }

    pub fn with_production_camera(mut self, camera: ProductionMapCamera) -> Self {
        self.production_camera = camera;
        self
    }
}

impl TerrainD3d11AcceptanceRequest {
    pub fn new(map_frame_bundle: impl Into<PathBuf>) -> Self {
        Self {
            map_frame_bundle: map_frame_bundle.into(),
            output: None,
            report: None,
            repeat: 1,
        }
    }

    pub fn with_output(mut self, output: impl Into<PathBuf>) -> Self {
        self.output = Some(output.into());
        self
    }

    pub fn with_report(mut self, report: impl Into<PathBuf>) -> Self {
        self.report = Some(report.into());
        self
    }

    pub fn with_repeat(mut self, repeat: usize) -> Self {
        self.repeat = repeat;
        self
    }
}

pub fn validate_terrain_d3d11_bundle(map_frame_bundle: &Path) -> Result<()> {
    NativeD3d11TerrainBundle::load(map_frame_bundle).map(|_| ())
}

pub fn validate_terrain_d3d11_bundle_with_resolver(
    map_frame_bundle: &Path,
    resolver: &GameFileResolver,
) -> Result<()> {
    NativeD3d11TerrainBundle::load_with_resolver(map_frame_bundle, resolver).map(|_| ())
}

pub fn render_terrain_d3d11(request: &NativeD3d11ReplayRequest) -> Result<NativeD3d11ReplayResult> {
    let bundle = NativeD3d11TerrainBundle::load(&request.map_frame_bundle)?;
    render_terrain_d3d11_loaded(request, &bundle)
}

pub fn render_terrain_d3d11_with_resolver(
    request: &NativeD3d11ReplayRequest,
    resolver: &GameFileResolver,
) -> Result<NativeD3d11ReplayResult> {
    let bundle = NativeD3d11TerrainBundle::load_with_resolver(&request.map_frame_bundle, resolver)?;
    render_terrain_d3d11_loaded(request, &bundle)
}

fn render_terrain_d3d11_loaded(
    request: &NativeD3d11ReplayRequest,
    bundle: &NativeD3d11TerrainBundle,
) -> Result<NativeD3d11ReplayResult> {
    let options = NativeD3d11RenderOptions {
        output: request.output.clone(),
        readback: request.readback,
        compare_each_draw: request.compare_each_draw,
        ..NativeD3d11RenderOptions::default()
    };
    let renderer = NativeD3d11TerrainRenderer::new()?;
    let prepared = renderer.prepare_production_map(bundle, request.production_camera)?;
    renderer.render_prepared_production_map(&prepared, &options)
}

pub fn render_production_map_d3d11(
    request: &ProductionMapD3d11ReplayRequest,
) -> Result<NativeD3d11ReplayResult> {
    let terrain_bundle = NativeD3d11TerrainBundle::load(&request.terrain_map_frame_bundle)?;
    render_production_map_d3d11_loaded(request, &terrain_bundle)
}

pub fn render_production_map_d3d11_with_resolver(
    request: &ProductionMapD3d11ReplayRequest,
    resolver: &GameFileResolver,
) -> Result<NativeD3d11ReplayResult> {
    let terrain_bundle =
        NativeD3d11TerrainBundle::load_with_resolver(&request.terrain_map_frame_bundle, resolver)?;
    render_production_map_d3d11_loaded(request, &terrain_bundle)
}

fn render_production_map_d3d11_loaded(
    request: &ProductionMapD3d11ReplayRequest,
    terrain_bundle: &NativeD3d11TerrainBundle,
) -> Result<NativeD3d11ReplayResult> {
    let options = NativeD3d11RenderOptions {
        output: request.output.clone(),
        readback: request.readback,
        compare_each_draw: request.compare_each_draw,
        ..NativeD3d11RenderOptions::default()
    };
    let renderer = NativeD3d11TerrainRenderer::new()?;
    let water_bundle = request
        .water_map_frame_bundle
        .as_ref()
        .map(NativeD3d11TerrainBundle::load_water)
        .transpose()?;
    let rivers_bundle = request
        .rivers_map_frame_bundle
        .as_ref()
        .map(NativeD3d11TerrainBundle::load_rivers)
        .transpose()?;
    let borders_bundle = request
        .borders_map_frame_bundle
        .as_ref()
        .map(NativeD3d11TerrainBundle::load_borders)
        .transpose()?;
    let trees_bundle = request
        .trees_map_frame_bundle
        .as_ref()
        .map(NativeD3d11TerrainBundle::load_trees)
        .transpose()?;
    let prepared = renderer.prepare_reference_production_map_with_layers(
        terrain_bundle,
        water_bundle.as_ref(),
        rivers_bundle.as_ref(),
        borders_bundle.as_ref(),
        trees_bundle.as_ref(),
        request.production_camera,
    )?;
    renderer.render_prepared_production_map(&prepared, &options)
}

pub fn render_terrain_d3d11_bundle(
    map_frame_bundle: &Path,
    output: Option<&Path>,
) -> Result<NativeD3d11ReplaySummary> {
    let request = NativeD3d11ReplayRequest {
        map_frame_bundle: map_frame_bundle.to_path_buf(),
        output: output.map(Path::to_path_buf),
        readback: false,
        compare_each_draw: false,
        production_camera: None,
    };
    Ok(render_terrain_d3d11(&request)?.summary)
}

pub fn validate_terrain_chunk_synthesis(
    terrain_chunk_library: &Path,
    report: Option<&Path>,
) -> Result<TerrainChunkSynthesisValidationReport> {
    let library = NativeD3d11TerrainChunkLibrary::load(terrain_chunk_library)?;
    let renderer = NativeD3d11TerrainRenderer::new()?;
    let samples = terrain_chunk_synthesis_record_samples(&library)?;
    let mut compared_records = Vec::new();
    for record in samples {
        let camera = production_map_camera_from_chunk_record(&library, record)?;
        let tile_size = terrain_chunk_world_tile_size(record)?;
        let captured = render_terrain_chunk_record_readback(&renderer, &library, record, camera)?;
        let world_rect = TerrainChunkSynthesisWorldRect::new(
            record.bounds.min_x,
            record.bounds.min_z,
            record.bounds.max_x,
            record.bounds.max_z,
        )?;
        let synthesized_prepared = renderer
            .prepare_terrain_chunk_synthesis_validation(&library, camera, tile_size, world_rect)?;
        let synthesized = renderer
            .render_prepared_terrain_chunk_synthesis(
                &synthesized_prepared,
                &NativeD3d11RenderOptions {
                    readback: true,
                    compare_final_reference: false,
                    ..NativeD3d11RenderOptions::default()
                },
            )?
            .readback
            .ok_or("terrain chunk synthesis validation expected synthesized readback")?;
        compared_records.push(terrain_chunk_synthesis_comparison(
            record,
            tile_size,
            terrain_chunk_draw_quad_offset_scale(&record.draw, library.bundle_dir())?,
            &captured,
            &synthesized,
        )?);
    }
    let status = if compared_records
        .iter()
        .all(|comparison| comparison.identical)
    {
        "identical"
    } else {
        "mismatch"
    };
    let report_value = TerrainChunkSynthesisValidationReport {
        schema: "battle-plan-terrain-chunk-synthesis-validation-v1".to_string(),
        status: status.to_string(),
        compared_records,
        lod_mappings: terrain_chunk_synthesis_lod_sources(&library)?
            .values()
            .map(TerrainChunkLodSource::mapping)
            .collect(),
    };
    if let Some(report) = report {
        write_json(report, &report_value)?;
    }
    Ok(report_value)
}

pub fn smoke_terrain_chunk_synthesis(
    terrain_chunk_library: &Path,
    target_tile_size: u32,
    output: Option<&Path>,
    cull_world_rect: Option<[f32; 4]>,
    scene_frame_bundle: Option<&Path>,
) -> Result<TerrainChunkSynthesisSmokeReport> {
    let library = NativeD3d11TerrainChunkLibrary::load(terrain_chunk_library)?;
    // With a scene frame of record, adopt its camera and scene CB0 wholesale so
    // the smoke output is directly comparable to that frame's replay. Without
    // one, fall back to the first record's captured frame state.
    let (camera, scene_cb0) = if let Some(path) = scene_frame_bundle {
        let scene_bundle = NativeD3d11TerrainBundle::load(path)?;
        (
            scene_bundle.production_camera_from_capture()?,
            Some(scene_bundle.scene_cb0_bytes()?),
        )
    } else {
        let first_record = library
            .manifest
            .chunks
            .first()
            .ok_or("production terrain chunk synthesis blocked: library has no records")?;
        (
            production_map_camera_from_chunk_record(&library, first_record)?,
            None,
        )
    };
    let renderer = NativeD3d11TerrainRenderer::new()?;
    let world_rect = library.world_rect();
    let expected_tile_count =
        terrain_chunk_synthesis_expected_tile_count(world_rect, target_tile_size)?;
    let prepared = renderer.prepare_terrain_chunk_synthesis(
        &library,
        camera,
        target_tile_size,
        world_rect,
        scene_cb0.as_deref(),
    )?;
    if prepared.tile_count() != expected_tile_count {
        return Err(format!(
            "production terrain chunk synthesis blocked: planned {} tiles, expected {}",
            prepared.tile_count(),
            expected_tile_count
        )
        .into());
    }
    let start = std::time::Instant::now();
    let result = renderer.render_prepared_terrain_chunk_synthesis(
        &prepared,
        &NativeD3d11RenderOptions {
            output: output.map(Path::to_path_buf),
            readback: false,
            compare_each_draw: false,
            compare_final_reference: false,
            parity_report: None,
            cull_world_rect,
            clear_targets: true,
            tree_season_cb: None,
            map_clock_time: None,
        },
    )?;
    let elapsed_ms = terrain_chunk_synthesis_elapsed_ms(start.elapsed());
    Ok(TerrainChunkSynthesisSmokeReport {
        schema: "battle-plan-terrain-chunk-synthesis-smoke-v1".to_string(),
        status: "ok".to_string(),
        target_tile_size,
        expected_tile_count,
        rendered_tile_count: result.summary.draw_count,
        elapsed_ms,
        target_width: result.summary.target_width,
        target_height: result.summary.target_height,
        target_format: result.summary.target_format,
        scene_frame_bundle: scene_frame_bundle.map(|path| path.display().to_string()),
        lod_mappings: prepared.lod_mappings().to_vec(),
    })
}

fn production_map_camera_from_chunk_record(
    library: &NativeD3d11TerrainChunkLibrary,
    record: &TerrainChunkRecord,
) -> Result<ProductionMapCamera> {
    production_map_camera_from_scene_cb0(decode_map_frame_terrain_scene_cb0(
        &record.draw,
        library.bundle_dir(),
    )?)
}

#[cfg(windows)]
fn render_terrain_chunk_record_readback(
    renderer: &NativeD3d11TerrainRenderer,
    library: &NativeD3d11TerrainChunkLibrary,
    record: &TerrainChunkRecord,
    camera: ProductionMapCamera,
) -> Result<NativeD3d11ReplayReadback> {
    let frame = terrain_chunk_frame_from_draws(&library.manifest, vec![record.draw.clone()])?;
    let prepared = unsafe {
        native_d3d11_prepare_terrain_chunk_library_frame(
            &renderer.device,
            &frame,
            library.bundle_dir(),
            camera,
        )?
    };
    native_d3d11_render_prepared_map_frame(
        &renderer.device,
        &prepared,
        true,
        None,
        true,
        false,
        false,
        None,
    )?
    .readback
    .ok_or_else(|| "terrain chunk synthesis validation expected captured readback".into())
}

#[cfg(not(windows))]
fn render_terrain_chunk_record_readback(
    _renderer: &NativeD3d11TerrainRenderer,
    _library: &NativeD3d11TerrainChunkLibrary,
    _record: &TerrainChunkRecord,
    _camera: ProductionMapCamera,
) -> Result<NativeD3d11ReplayReadback> {
    Err(
        "production native D3D11 terrain replay blocked: D3D11 replay is only available on Windows"
            .into(),
    )
}

fn terrain_chunk_synthesis_comparison(
    record: &TerrainChunkRecord,
    tile_size: u32,
    quad_offset_scale_is_detail: [f32; 4],
    captured: &NativeD3d11ReplayReadback,
    synthesized: &NativeD3d11ReplayReadback,
) -> Result<TerrainChunkSynthesisComparison> {
    if captured.width != synthesized.width
        || captured.height != synthesized.height
        || captured.dxgi_format != synthesized.dxgi_format
        || captured.bytes.len() != synthesized.bytes.len()
    {
        return Err(format!(
            "production terrain chunk synthesis validation blocked: readback shape mismatch for {} captured {}x{} format {} bytes {}, synthesized {}x{} format {} bytes {}",
            record.chunk_id,
            captured.width,
            captured.height,
            captured.dxgi_format,
            captured.bytes.len(),
            synthesized.width,
            synthesized.height,
            synthesized.dxgi_format,
            synthesized.bytes.len()
        )
        .into());
    }
    let first_mismatch_byte = captured
        .bytes
        .iter()
        .zip(&synthesized.bytes)
        .position(|(left, right)| left != right);
    let first_mismatch_texel = first_mismatch_byte.and_then(|byte| {
        let bytes_per_texel = usize::try_from(captured.row_pitch)
            .ok()
            .and_then(|row| {
                usize::try_from(captured.width)
                    .ok()
                    .map(|width| row / width)
            })
            .filter(|value| *value > 0)?;
        let texel = byte / bytes_per_texel;
        let width = usize::try_from(captured.width).ok()?;
        Some([(texel % width) as u32, (texel / width) as u32])
    });
    Ok(TerrainChunkSynthesisComparison {
        record_chunk_id: record.chunk_id.clone(),
        tile_size,
        cb1_scale: quad_offset_scale_is_detail[2],
        quad_offset_scale_is_detail,
        captured_fnv1a64: fnv1a64_bytes(&captured.bytes),
        synthesized_fnv1a64: fnv1a64_bytes(&synthesized.bytes),
        byte_len: captured.bytes.len(),
        identical: first_mismatch_byte.is_none(),
        first_mismatch_byte,
        first_mismatch_texel,
    })
}

pub fn accept_terrain_d3d11(
    request: &TerrainD3d11AcceptanceRequest,
) -> Result<TerrainD3d11AcceptanceReport> {
    accept_terrain_d3d11_impl(request, None)
}

pub fn accept_terrain_d3d11_with_resolver(
    request: &TerrainD3d11AcceptanceRequest,
    resolver: &GameFileResolver,
) -> Result<TerrainD3d11AcceptanceReport> {
    accept_terrain_d3d11_impl(request, Some(resolver))
}

fn accept_terrain_d3d11_impl(
    request: &TerrainD3d11AcceptanceRequest,
    resolver: Option<&GameFileResolver>,
) -> Result<TerrainD3d11AcceptanceReport> {
    if request.repeat == 0 {
        return Err("terrain D3D11 acceptance repeat count must be positive".into());
    }
    let bundle = match resolver {
        Some(resolver) => {
            NativeD3d11TerrainBundle::load_with_resolver(&request.map_frame_bundle, resolver)?
        }
        None => NativeD3d11TerrainBundle::load(&request.map_frame_bundle)?,
    };
    let renderer = NativeD3d11TerrainRenderer::new()?;
    let prepared = renderer.prepare_production_map(&bundle, None)?;
    let mut result = None;
    for iteration in 0..request.repeat {
        let final_iteration = iteration + 1 == request.repeat;
        let options = NativeD3d11RenderOptions {
            output: final_iteration.then(|| request.output.clone()).flatten(),
            readback: false,
            compare_each_draw: true,
            ..NativeD3d11RenderOptions::default()
        };
        result = Some(renderer.render_prepared_production_map(&prepared, &options)?);
    }
    let result = result.ok_or("terrain D3D11 acceptance did not execute")?;
    let report =
        terrain_d3d11_acceptance_report(&request.map_frame_bundle, request.repeat, &result)?;
    let report_path = match &request.report {
        Some(report) => report.clone(),
        None => {
            map_frame_bundle_dir(&request.map_frame_bundle)?.join("terrain_d3d11_acceptance.json")
        }
    };
    write_json(&report_path, &report)?;
    Ok(report)
}

pub(crate) fn terrain_d3d11_acceptance_report(
    map_frame_bundle: &Path,
    repeat: usize,
    result: &NativeD3d11ReplayResult,
) -> Result<TerrainD3d11AcceptanceReport> {
    let comparison = result
        .reference_comparison
        .as_ref()
        .ok_or("terrain D3D11 acceptance requires final the game reference comparison proof")?;
    let manifest_path = map_frame_bundle_manifest_path(map_frame_bundle);
    let bundle_dir = map_frame_bundle_dir(map_frame_bundle)?;
    let bundle_file = manifest_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("terrain D3D11 acceptance bundle path has no UTF-8 file name")?
        .to_string();
    let map_frame_bundle_fnv1a64 = fnv1a64_file(&manifest_path)?;
    let map_capture_session_fnv1a64 = fnv1a64_file(&bundle_dir.join(MAP_CAPTURE_SESSION_FILE))?;
    Ok(TerrainD3d11AcceptanceReport {
        schema: TERRAIN_D3D11_ACCEPTANCE_REPORT_SCHEMA.to_string(),
        status: "accepted".to_string(),
        capture_moment: TERRAIN_CAPTURE_MOMENT.to_string(),
        bundle_file,
        map_frame_bundle_fnv1a64,
        map_capture_session_fnv1a64,
        repeat,
        compare_each_draw: true,
        draw_count: result.summary.draw_count,
        vertex_buffer_count: result.summary.vertex_buffer_count,
        index_buffer_count: result.summary.index_buffer_count,
        constant_buffer_count: result.summary.constant_buffer_count,
        shader_resource_count: result.summary.shader_resource_count,
        sampler_count: result.summary.sampler_count,
        target_width: result.summary.target_width,
        target_height: result.summary.target_height,
        target_format: result.summary.target_format,
        final_reference_byte_len: comparison.byte_len,
        final_expected_fnv1a64: comparison.expected_fnv1a64.clone(),
        final_actual_fnv1a64: comparison.actual_fnv1a64.clone(),
    })
}

pub(crate) fn validate_native_d3d11_replay_payloads(
    frame: &MapFrameRuntimeBundle,
    require_pre_draw_outputs: bool,
) -> Result<()> {
    let blockers = native_d3d11_replay_payload_blockers(frame, require_pre_draw_outputs);
    if blockers.is_empty() {
        return Ok(());
    }
    Err(format!(
        "production native D3D11 terrain replay blocked: {}",
        blockers.join("; ")
    )
    .into())
}

pub(crate) fn native_d3d11_replay_payload_blockers(
    frame: &MapFrameRuntimeBundle,
    require_pre_draw_outputs: bool,
) -> Vec<String> {
    let mut blockers = Vec::new();
    for draw in &frame.draws {
        if draw.vertex_shader.is_none() {
            blockers.push(format!(
                "draw {} has no captured vertex shader bytecode/reflection",
                draw.draw_index
            ));
        }
        if draw.topology.is_none() {
            blockers.push(format!(
                "draw {} has no captured IA primitive topology",
                draw.draw_index
            ));
        }
        if draw.shader.bytecode_file.trim().is_empty() {
            blockers.push(format!(
                "draw {} has no captured pixel shader bytecode file",
                draw.draw_index
            ));
        }
        if draw.vertex_buffers.is_empty() {
            blockers.push(format!(
                "draw {} has no captured IA vertex-buffer bindings",
                draw.draw_index
            ));
        }
        for vertex_buffer in draw.vertex_buffers.values() {
            if vertex_buffer.file.is_none()
                || vertex_buffer.byte_len == 0
                || vertex_buffer.fnv1a64.is_none()
                || vertex_buffer.stride.unwrap_or(0) == 0
            {
                blockers.push(format!(
                    "draw {} IA vertex buffer slot {} is missing raw bytes, hash, or stride",
                    draw.draw_index, vertex_buffer.slot
                ));
            }
        }
        if draw.draw_call.contains("Indexed") && draw.index_buffer.is_none() {
            blockers.push(format!(
                "draw {} is indexed but has no captured index buffer",
                draw.draw_index
            ));
        }
        if draw.constant_buffers.is_empty() {
            blockers.push(format!(
                "draw {} has no captured constant-buffer bindings",
                draw.draw_index
            ));
        }
        for constant_buffer in &draw.constant_buffers {
            if constant_buffer.file.is_none()
                || constant_buffer.byte_len == 0
                || constant_buffer.fnv1a64.is_none()
            {
                blockers.push(format!(
                    "draw {} {} constant buffer slot {} is missing raw bytes or hash",
                    draw.draw_index, constant_buffer.stage, constant_buffer.slot
                ));
            }
            if constant_buffer.byte_len % 16 != 0 {
                blockers.push(format!(
                    "draw {} {} constant buffer slot {} byte_len {} is not 16-byte aligned",
                    draw.draw_index,
                    constant_buffer.stage,
                    constant_buffer.slot,
                    constant_buffer.byte_len
                ));
            }
        }
        blockers.extend(map_frame_reflected_constant_buffer_binding_blockers(
            &draw.shader,
            draw,
        ));
        if let Some(vertex_shader) = draw.vertex_shader.as_ref() {
            blockers.extend(map_frame_reflected_constant_buffer_binding_blockers(
                vertex_shader,
                draw,
            ));
        }
        if draw.samplers.is_empty() {
            blockers.push(format!(
                "draw {} has no captured sampler-state bindings",
                draw.draw_index
            ));
        }
        blockers.extend(map_frame_reflected_resource_binding_blockers(draw));
        blockers.extend(map_frame_reflected_sampler_binding_blockers(draw));
        if frame.capture_moment == WATER_CAPTURE_MOMENT {
            blockers.extend(pdxwater_sampler_binding_blockers(draw));
        }
        for resource in &draw.shader_resources {
            let name = resource
                .declared_name
                .as_deref()
                .unwrap_or("<unnamed shader resource>");
            if resource.is_null {
                continue;
            }
            if resource.bind_flags & D3D11_BIND_SHADER_RESOURCE == 0 {
                blockers.push(format!(
                    "draw {} shader resource {name} {} slot {} is missing D3D11_BIND_SHADER_RESOURCE",
                    draw.draw_index, resource.stage, resource.slot
                ));
            }
            if resource.view_desc.is_none() {
                blockers.push(format!(
                    "draw {} shader resource {name} {} slot {} is missing captured D3D11 shader-resource-view descriptor",
                    draw.draw_index, resource.stage, resource.slot
                ));
            }
            if frame.capture_moment == WATER_CAPTURE_MOMENT
                && name == WATER_REFRACTION_RESOURCE_SYMBOL
            {
                if let Err(error) = validate_app_owned_water_refraction_binding(
                    draw.draw_index,
                    resource,
                    "production water renderer",
                ) {
                    blockers.push(error.to_string());
                }
                continue;
            }
            let Some(file) = resource.file.as_ref() else {
                blockers.push(format!(
                    "draw {} shader resource {name} {} slot {} has a D3D11 descriptor but no raw texture payload file in the map-frame bundle",
                    draw.draw_index, resource.stage, resource.slot
                ));
                continue;
            };
            if file.trim().is_empty() || resource.fnv1a64.is_none() || resource.byte_len == 0 {
                blockers.push(format!(
                    "draw {} shader resource {name} {} slot {} has incomplete texture file/hash metadata",
                    draw.draw_index, resource.stage, resource.slot
                ));
                continue;
            }
            if let Err(error) = native_d3d11_validate_texture_payload_shape(resource) {
                blockers.push(format!(
                    "draw {} shader resource {name} {} slot {} {error}",
                    draw.draw_index, resource.stage, resource.slot
                ));
            }
        }
        let mut state_blockers = map_frame_post_diffuse_state_blockers(draw);
        if !require_pre_draw_outputs {
            state_blockers.retain(|blocker| !is_clear_provenance_blocker(blocker));
        }
        for blocker in state_blockers {
            blockers.push(format!("draw {}: {blocker}", draw.draw_index));
        }
        if let Some(state) = draw.draw_state.as_ref() {
            if let Some(output_merger) = state.output_merger.as_ref() {
                if output_merger.render_targets.len() != 1 {
                    blockers.push(format!(
                        "draw {} output-merger has {} render targets; native replay currently requires exactly one terrain render target",
                        draw.draw_index,
                        output_merger.render_targets.len()
                    ));
                }
                if require_pre_draw_outputs {
                    match output_merger.render_targets.first() {
                        Some(target) if target.pre_draw_payload.is_none() => blockers.push(format!(
                            "draw {} output-merger render target slot {:?} has no pre-draw payload; recapture the map-frame bundle with pre-draw OM payloads",
                            draw.draw_index, target.slot
                        )),
                        Some(target) if target.desc.sample_count > 1 => blockers.push(format!(
                            "draw {} output-merger render target slot {:?} is {}x MSAA; native replay requires unresolved per-sample pre-draw payload or the upstream draw sequence that produced it",
                            draw.draw_index, target.slot, target.desc.sample_count
                        )),
                        Some(_) => {}
                        None => {}
                    }
                    match output_merger.depth_stencil.as_ref() {
                        Some(depth) if depth.pre_draw_payload.is_none() => blockers.push(format!(
                            "draw {} output-merger depth-stencil has no pre-draw payload; recapture the map-frame bundle with pre-draw OM payloads",
                            draw.draw_index
                        )),
                        Some(depth) if depth.desc.sample_count > 1 => blockers.push(format!(
                            "draw {} output-merger depth-stencil is {}x MSAA; native replay requires unresolved per-sample pre-draw payload or the upstream draw sequence that produced it",
                            draw.draw_index, depth.desc.sample_count
                        )),
                        Some(_) => {}
                        None => blockers.push(format!(
                            "draw {} output-merger has no captured depth-stencil target",
                            draw.draw_index
                        )),
                    }
                }
            }
        }
    }
    blockers.extend(native_d3d11_post_pass_output_blockers(frame));
    blockers
}

pub(crate) fn production_map_target_evidence(
    frame: &MapFrameRuntimeBundle,
) -> Result<(&MapFrameOutputMergerTexture, &MapFrameOutputMergerTexture)> {
    let first_draw = frame
        .draws
        .first()
        .ok_or("production map renderer blocked: pass graph needs a terrain draw sequence")?;
    let state = first_draw.draw_state.as_ref().ok_or(
        "production map renderer blocked: terrain pass graph needs captured output-merger state",
    )?;
    let output_merger = state.output_merger.as_ref().ok_or(
        "production map renderer blocked: terrain pass graph needs captured color/depth targets",
    )?;
    let color = output_merger.render_targets.first().ok_or(
        "production map renderer blocked: terrain pass graph has no color render target evidence",
    )?;
    let depth = output_merger.depth_stencil.as_ref().ok_or(
        "production map renderer blocked: terrain pass graph has no depth target evidence",
    )?;
    if color.desc.width == 0
        || color.desc.height == 0
        || color.desc.format == 0
        || depth.desc.format == 0
        || color.desc.sample_count == 0
        || depth.desc.sample_count == 0
    {
        return Err(
            "production map renderer blocked: terrain pass graph target/depth descriptors are incomplete"
                .into(),
        );
    }
    Ok((color, depth))
}

pub(crate) fn production_map_camera_from_terrain_bundle(
    bundle: &NativeD3d11TerrainBundle,
) -> Result<ProductionMapCamera> {
    let mut expected = None;
    for draw in &bundle.frame.draws {
        let decoded = decode_map_frame_terrain_scene_cb0(draw, &bundle.bundle_dir)?;
        let camera = production_map_camera_from_scene_cb0(decoded)?;
        match expected {
            None => expected = Some(camera),
            Some(reference) if reference != camera => {
                return Err(format!(
                    "production map renderer blocked: terrain draw {} camera constants differ from the first draw; app-owned camera must be a single explicit graph input",
                    draw.draw_index
                )
                .into())
            }
            Some(_) => {}
        }
    }
    expected.ok_or_else(|| {
        "production map renderer blocked: cannot derive app-owned camera from an empty terrain draw sequence"
            .into()
    })
}

pub(crate) fn production_map_camera_from_scene_cb0(
    decoded: TerrainSceneCb0Decoded,
) -> Result<ProductionMapCamera> {
    ProductionMapCamera::new(
        decoded.view_projection_matrix,
        [
            decoded.v_cam_pos.x,
            decoded.v_cam_pos.y,
            decoded.v_cam_pos.z,
        ],
    )
}

pub(crate) fn native_d3d11_post_pass_output_blockers(frame: &MapFrameRuntimeBundle) -> Vec<String> {
    let mut blockers = Vec::new();
    let Some(output_merger) = frame.post_pass_output_merger.as_ref() else {
        blockers.push(
            "map-frame bundle has no post-pass output-merger terrain render target payload"
                .to_string(),
        );
        return blockers;
    };
    if output_merger.render_targets.len() != 1 {
        blockers.push(format!(
            "post-pass output-merger has {} render targets; native replay reference comparison requires exactly one terrain render target",
            output_merger.render_targets.len()
        ));
    }
    let Some(target) = output_merger.render_targets.first() else {
        return blockers;
    };
    validate_output_merger_texture(
        target,
        D3D11_BIND_RENDER_TARGET,
        "post-pass render target",
        &mut blockers,
    );
    let Some(payload) = target.post_pass_payload.as_ref() else {
        blockers.push(
            "post-pass output-merger terrain render target is missing captured bytes".to_string(),
        );
        return blockers;
    };
    if payload.file.trim().is_empty() || payload.byte_len == 0 || payload.fnv1a64.trim().is_empty()
    {
        blockers.push(
            "post-pass output-merger terrain render target has incomplete file/hash metadata"
                .to_string(),
        );
    }
    if payload.payload_format != NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT {
        blockers.push(format!(
            "post-pass output-merger terrain render target payload_format is {}, expected {NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT}",
            payload.payload_format
        ));
    }
    if payload.subresources.len() != 1 {
        blockers.push(format!(
            "post-pass output-merger terrain render target has {} subresources; expected exactly one",
            payload.subresources.len()
        ));
    }
    if let Some(subresource) = payload.subresources.first() {
        if subresource.mip_slice != 0 || subresource.array_slice != 0 {
            blockers.push(format!(
                "post-pass output-merger terrain render target references mip {} array slice {}; expected mip 0 array slice 0",
                subresource.mip_slice, subresource.array_slice
            ));
        }
        if subresource.width != target.desc.width || subresource.height != target.desc.height {
            blockers.push(format!(
                "post-pass output-merger terrain render target subresource is {}x{}, descriptor is {}x{}",
                subresource.width, subresource.height, target.desc.width, target.desc.height
            ));
        }
        match native_d3d11_render_target_row_bytes(target.desc.format, target.desc.width) {
            Ok(row_bytes) => {
                if subresource.row_pitch < row_bytes {
                    blockers.push(format!(
                        "post-pass output-merger terrain render target row_pitch {} is below row byte width {}",
                        subresource.row_pitch, row_bytes
                    ));
                }
                let minimum_depth_pitch =
                    u64::from(subresource.row_pitch) * u64::from(target.desc.height);
                if u64::from(subresource.depth_pitch) < minimum_depth_pitch {
                    blockers.push(format!(
                        "post-pass output-merger terrain render target depth_pitch {} is below row_pitch {} * height {}",
                        subresource.depth_pitch, subresource.row_pitch, target.desc.height
                    ));
                }
                let end = subresource.file_offset as u64 + u64::from(subresource.depth_pitch);
                if end > payload.byte_len as u64 {
                    blockers.push(format!(
                        "post-pass output-merger terrain render target subresource extends to byte {end}, beyond payload byte_len {}",
                        payload.byte_len
                    ));
                }
            }
            Err(error) => blockers.push(error.to_string()),
        }
    }
    let Some(evidence) = frame.runtime_files.get(&payload.file) else {
        blockers.push(format!(
            "post-pass output-merger terrain render target payload {} is missing runtime file evidence",
            payload.file
        ));
        return blockers;
    };
    if evidence.byte_len != payload.byte_len || evidence.fnv1a64 != payload.fnv1a64 {
        blockers.push(format!(
            "post-pass output-merger terrain render target evidence for {} does not match payload metadata",
            payload.file
        ));
    }
    blockers
}

pub(crate) fn native_d3d11_validate_texture_payload_shape(
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.width == 0 || resource.height == 0 || resource.mip_count == 0 {
        return Err("texture descriptor has zero width, height, or mip count".into());
    }
    native_d3d11_texture_array_size(resource)?;
    if resource.sample_count == 0 {
        return Err(format!(
            "native D3D11 replay requires captured Texture2D SampleDesc.Count for non-null SRV slot {}; recapture the map-frame bundle with full texture descriptors",
            resource.slot
        )
        .into());
    }
    native_d3d11_validate_srv_texture_desc(resource)?;
    if resource.payload_format.as_deref() != Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT)
        || resource.subresources.is_empty()
    {
        return native_d3d11_validate_flat_texture_payload_shape(resource);
    }
    native_d3d11_validate_mapped_subresources(resource)
}

pub(crate) fn native_d3d11_validate_flat_texture_payload_shape(
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    let array_size = native_d3d11_texture_array_size(resource)?;
    if resource.mip_count != 1 || array_size != 1 || resource.sample_count != 1 {
        return Err(format!(
            "native D3D11 replay requires exact {NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT} payload metadata for non-flat SRV slot {}, got payload_format {:?}, {} subresources, mip_count {}, array_size {}, sample_count {}",
            resource.slot,
            resource.payload_format,
            resource.subresources.len(),
            resource.mip_count,
            array_size,
            resource.sample_count
        )
        .into());
    }
    let row_pitch = native_d3d11_flat_texture_row_pitch(resource)?;
    let expected = u64::from(row_pitch) * u64::from(resource.height);
    if resource.byte_len as u64 != expected {
        return Err(format!(
            "native D3D11 replay flat SRV slot {} byte_len {} does not match row_pitch {} * height {} = {}",
            resource.slot, resource.byte_len, row_pitch, resource.height, expected
        )
        .into());
    }
    Ok(())
}

pub(crate) fn native_d3d11_flat_texture_row_pitch(
    resource: &MapFrameTextureBinding,
) -> Result<u32> {
    let bytes_per_texel = match resource.dxgi_format {
        DXGI_FORMAT_A8_UNORM => 1,
        DXGI_FORMAT_R8G8B8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => 4,
        DXGI_FORMAT_R32G32B32A32_FLOAT => 16,
        other => {
            return Err(format!(
                "native D3D11 replay requires mapped subresource metadata for SRV slot {} DXGI format {}; flat upload supports only RGBA8/BGRA8/RGBA32F",
                resource.slot, other
            )
            .into())
        }
    };
    resource
        .width
        .checked_mul(bytes_per_texel)
        .ok_or_else(|| "texture row pitch overflow".into())
}

pub(crate) fn native_d3d11_validate_mapped_subresources(
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.payload_format.as_deref() != Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT) {
        return Err(format!(
            "has subresource metadata but payload_format is {:?}, expected {NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT}",
            resource.payload_format
        )
        .into());
    }
    if resource.subresources.is_empty() {
        return Err("uses exact D3D11 payload format but has no subresource metadata".into());
    }
    let array_size = native_d3d11_texture_array_size(resource)?;
    let expected_subresources = usize::try_from(
        resource
            .mip_count
            .checked_mul(array_size)
            .ok_or("exact D3D11 texture payload subresource count overflow")?,
    )?;
    if resource.subresources.len() != expected_subresources {
        return Err(format!(
            "has {} subresource record(s), expected one per mip level ({}) per array slice ({}) for Texture2D replay",
            resource.subresources.len(),
            resource.mip_count,
            array_size
        )
        .into());
    }
    native_d3d11_format_bytes_per_pixel_or_block(resource.dxgi_format)?;
    for array_slice in 0..array_size {
        for expected_mip in 0..resource.mip_count {
            let subresource = resource
                .subresources
                .iter()
                .find(|subresource| {
                    subresource.mip_slice == expected_mip
                        && subresource.array_slice == array_slice
                })
                .ok_or_else(|| {
                    format!(
                        "is missing subresource metadata for mip {expected_mip}, array slice {array_slice}"
                    )
                })?;
            native_d3d11_validate_one_subresource(resource, subresource, expected_mip)?;
        }
    }
    let mut ordered = resource.subresources.clone();
    ordered.sort_by_key(|subresource| subresource.file_offset);
    let mut covered_until = 0usize;
    for subresource in &ordered {
        let end = subresource
            .file_offset
            .checked_add(subresource.byte_len)
            .ok_or("subresource file range overflow")?;
        if end > resource.byte_len {
            return Err(format!(
                "subresource mip {} array slice {} file range {}..{} exceeds texture byte_len {}",
                subresource.mip_slice,
                subresource.array_slice,
                subresource.file_offset,
                end,
                resource.byte_len
            )
            .into());
        }
        if subresource.file_offset < covered_until {
            return Err(format!(
                "subresource mip {} array slice {} overlaps a previous subresource byte range",
                subresource.mip_slice, subresource.array_slice
            )
            .into());
        }
        covered_until = end;
    }
    if covered_until != resource.byte_len {
        return Err(format!(
            "subresources cover {} bytes, but texture payload byte_len is {}",
            covered_until, resource.byte_len
        )
        .into());
    }
    Ok(())
}

pub(crate) fn native_d3d11_texture_array_size(resource: &MapFrameTextureBinding) -> Result<u32> {
    if resource.array_size == 0 {
        return Err(format!(
            "native D3D11 replay requires captured Texture2D ArraySize for non-null SRV slot {}; recapture the map-frame bundle with full texture descriptors",
            resource.slot
        )
        .into());
    }
    Ok(resource.array_size)
}

pub(crate) fn native_d3d11_validate_srv_texture_desc(
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    let Some(view) = resource.view_desc.as_ref() else {
        return Ok(());
    };
    let array_size = native_d3d11_texture_array_size(resource)?;
    match view.view_dimension {
        D3D11_SRV_DIMENSION_TEXTURE2D => {
            if view.first_array_slice != 0 || view.array_size > 1 || view.num_cubes != 0 {
                return Err(format!(
                    "native D3D11 replay blocked: SRV slot {} Texture2D view carries array/cube range metadata inconsistent with Texture2D",
                    resource.slot
                )
                .into());
            }
        }
        D3D11_SRV_DIMENSION_TEXTURE2DARRAY => {
            native_d3d11_validate_srv_array_range(
                resource,
                view.first_array_slice,
                view.array_size,
            )?;
        }
        D3D11_SRV_DIMENSION_TEXTURECUBE => {
            native_d3d11_validate_texturecube_flag(resource)?;
            if array_size < 6 {
                return Err(format!(
                    "native D3D11 replay blocked: SRV slot {} TextureCube view requires at least 6 array slices, captured Texture2D ArraySize is {array_size}",
                    resource.slot
                )
                .into());
            }
        }
        D3D11_SRV_DIMENSION_TEXTURECUBEARRAY => {
            native_d3d11_validate_texturecube_flag(resource)?;
            let slice_count = view
                .num_cubes
                .checked_mul(6)
                .ok_or("TextureCubeArray SRV cube count overflow")?;
            native_d3d11_validate_srv_array_range(resource, view.first_array_slice, slice_count)?;
        }
        other => {
            return Err(format!(
                "native D3D11 replay blocked: SRV slot {} uses unsupported view dimension {other}",
                resource.slot
            )
            .into())
        }
    }
    Ok(())
}

pub(crate) fn native_d3d11_validate_texturecube_flag(
    resource: &MapFrameTextureBinding,
) -> Result<()> {
    if resource.misc_flags & D3D11_RESOURCE_MISC_TEXTURECUBE == 0 {
        return Err(format!(
            "native D3D11 replay blocked: SRV slot {} is a TextureCube/TextureCubeArray view but captured Texture2D MiscFlags do not include D3D11_RESOURCE_MISC_TEXTURECUBE; recapture the map-frame bundle with full texture descriptors",
            resource.slot
        )
        .into());
    }
    Ok(())
}

pub(crate) fn native_d3d11_validate_srv_array_range(
    resource: &MapFrameTextureBinding,
    first_array_slice: u32,
    array_size: u32,
) -> Result<()> {
    if array_size == 0 {
        return Err(format!(
            "native D3D11 replay blocked: SRV slot {} has a zero-length array range",
            resource.slot
        )
        .into());
    }
    let texture_array_size = native_d3d11_texture_array_size(resource)?;
    let end = first_array_slice
        .checked_add(array_size)
        .ok_or("SRV array range overflow")?;
    if end > texture_array_size {
        return Err(format!(
            "native D3D11 replay blocked: SRV slot {} array range {}..{} exceeds captured Texture2D ArraySize {}",
            resource.slot, first_array_slice, end, texture_array_size
        )
        .into());
    }
    Ok(())
}

pub(crate) fn native_d3d11_validate_one_subresource(
    resource: &MapFrameTextureBinding,
    subresource: &MapFrameTextureSubresource,
    expected_mip: u32,
) -> Result<()> {
    let expected_width = resource.width.checked_shr(expected_mip).unwrap_or(0).max(1);
    let expected_height = resource
        .height
        .checked_shr(expected_mip)
        .unwrap_or(0)
        .max(1);
    if subresource.width != expected_width || subresource.height != expected_height {
        return Err(format!(
            "subresource mip {} array slice {} has dimensions {}x{}, expected {}x{}",
            expected_mip,
            subresource.array_slice,
            subresource.width,
            subresource.height,
            expected_width,
            expected_height
        )
        .into());
    }
    let minimum_row_pitch = native_d3d11_minimum_row_pitch(resource.dxgi_format, expected_width)?;
    if subresource.row_pitch < minimum_row_pitch {
        return Err(format!(
            "subresource mip {} array slice {} row_pitch {} is below minimum {} for DXGI format {}",
            expected_mip,
            subresource.array_slice,
            subresource.row_pitch,
            minimum_row_pitch,
            resource.dxgi_format
        )
        .into());
    }
    let row_count = native_d3d11_format_row_count(resource.dxgi_format, expected_height)?;
    let minimum_depth_pitch = subresource
        .row_pitch
        .checked_mul(row_count)
        .ok_or("subresource depth pitch overflow")?;
    if subresource.depth_pitch < minimum_depth_pitch {
        return Err(format!(
            "subresource mip {} array slice {} depth_pitch {} is below row_pitch {} * rows {}",
            expected_mip,
            subresource.array_slice,
            subresource.depth_pitch,
            subresource.row_pitch,
            row_count
        )
        .into());
    }
    if subresource.byte_len != usize::try_from(subresource.depth_pitch)? {
        return Err(format!(
            "subresource mip {} array slice {} byte_len {} does not equal captured depth_pitch {}",
            expected_mip, subresource.array_slice, subresource.byte_len, subresource.depth_pitch
        )
        .into());
    }
    Ok(())
}

pub(crate) fn native_d3d11_format_bytes_per_pixel_or_block(format: u32) -> Result<u32> {
    match format {
        DXGI_FORMAT_R8G8B8A8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM
        | DXGI_FORMAT_B8G8R8X8_UNORM
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => Ok(4),
        DXGI_FORMAT_A8_UNORM => Ok(1),
        DXGI_FORMAT_R16G16B16A16_FLOAT => Ok(8),
        DXGI_FORMAT_R32G32B32A32_FLOAT => Ok(16),
        DXGI_FORMAT_BC1_UNORM | DXGI_FORMAT_BC1_UNORM_SRGB => Ok(8),
        DXGI_FORMAT_BC3_UNORM | DXGI_FORMAT_BC3_UNORM_SRGB => Ok(16),
        other => Err(format!(
            "uses unsupported native texture DXGI format {other}; exact replay needs upload support for this format"
        )
        .into()),
    }
}

pub(crate) fn native_d3d11_minimum_row_pitch(format: u32, width: u32) -> Result<u32> {
    match format {
        DXGI_FORMAT_BC1_UNORM
        | DXGI_FORMAT_BC1_UNORM_SRGB
        | DXGI_FORMAT_BC3_UNORM
        | DXGI_FORMAT_BC3_UNORM_SRGB => width
            .max(1)
            .div_ceil(4)
            .checked_mul(native_d3d11_format_bytes_per_pixel_or_block(format)?)
            .ok_or_else(|| "compressed texture row pitch overflow".into()),
        _ => width
            .checked_mul(native_d3d11_format_bytes_per_pixel_or_block(format)?)
            .ok_or_else(|| "texture row pitch overflow".into()),
    }
}

pub(crate) fn native_d3d11_format_row_count(format: u32, height: u32) -> Result<u32> {
    match format {
        DXGI_FORMAT_BC1_UNORM
        | DXGI_FORMAT_BC1_UNORM_SRGB
        | DXGI_FORMAT_BC3_UNORM
        | DXGI_FORMAT_BC3_UNORM_SRGB => Ok(height.max(1).div_ceil(4)),
        _ => Ok(height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    use std::fs;
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    use winapi::um::d3dcommon::{D3D11_SRV_DIMENSION_TEXTURE2D, D3D11_SRV_DIMENSION_TEXTURECUBE};

    // WATER3 — the chunk-synthesis low-level selector chooses the
    // mechanism-correct WATER2 underwater seafloor pass as WaterRefraction only
    // under --water-full. Product replay adds a separate fail-closed gate when a
    // water pass is present.
    #[cfg(windows)]
    #[test]
    fn synthesis_underwater_refraction_selected_only_under_water_full() {
        // Low-level flag OFF: legacy inert source (no seafloor re-render). This
        // is product-blocked by product_water_source_policy when water exists.
        assert_eq!(
            world_synthesis_water_refraction_source(false),
            NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot,
        );
        // Flag ON: the real seafloor render (WATER2 underwater pass).
        assert_eq!(
            world_synthesis_water_refraction_source(true),
            NativeD3d11WaterRefractionSource::UnderwaterPass,
        );
        // The underwater pass is never the flag-OFF low-level selection.
        assert_ne!(
            world_synthesis_water_refraction_source(false),
            NativeD3d11WaterRefractionSource::UnderwaterPass,
        );
    }

    #[cfg(windows)]
    #[test]
    fn synthesis_reflection_cube_selected_only_under_water_full() {
        assert_eq!(
            world_synthesis_reflection_cube_source(false),
            NativeD3d11ReflectionCubeSource::MeanColorFlatten,
        );
        assert_eq!(
            world_synthesis_reflection_cube_source(true),
            NativeD3d11ReflectionCubeSource::ShippedFileCube,
        );
    }

    // WATER3 — the low-level synthesis path's --water-full refraction selection
    // must match the captured-terrain production path's non-reference,
    // water-full selection, so both product paths render the same
    // mechanism-correct WaterRefraction.
    #[cfg(windows)]
    #[test]
    fn synthesis_refraction_matches_captured_path_non_reference_selection() {
        let (captured_non_reference, _) =
            product_water_source_policy(false, true, true).expect("water-full product policy");
        assert_eq!(
            world_synthesis_water_refraction_source(true),
            captured_non_reference,
            "synthesis vs captured non-reference refraction diverges under --water-full",
        );
    }

    #[cfg(windows)]
    #[test]
    fn product_water_policy_blocks_app_owned_refraction_when_water_exists() {
        let err = product_water_source_policy(false, false, true)
            .expect_err("non-reference water pass without --water-full must fail closed");
        assert!(
            err.to_string()
                .contains("old AppOwnedTerrainColorSnapshot approximation is not engine-derived"),
            "{err}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn product_water_policy_keeps_reference_and_water_full_engine_sources() {
        assert_eq!(
            product_water_source_policy(true, false, true).expect("reference replay policy"),
            (
                NativeD3d11WaterRefractionSource::CapturedBundleTexture,
                NativeD3d11ReflectionCubeSource::ShippedFileCube
            )
        );
        assert_eq!(
            product_water_source_policy(false, true, true).expect("water-full policy"),
            (
                NativeD3d11WaterRefractionSource::UnderwaterPass,
                NativeD3d11ReflectionCubeSource::ShippedFileCube
            )
        );
        assert_eq!(
            product_water_source_policy(false, false, false).expect("no water pass to render"),
            (
                NativeD3d11WaterRefractionSource::AppOwnedTerrainColorSnapshot,
                NativeD3d11ReflectionCubeSource::MeanColorFlatten
            )
        );
    }

    #[test]
    fn native_d3d11_payload_accepts_exact_bc3_mapped_subresources() {
        let resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 0,
            declared_name: Some(TERRAIN_DIFFUSE_SYMBOL.to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_BC3_UNORM_SRGB,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 2,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: Some("terrain_diffuse.d3d11tex".to_string()),
            byte_len: 80,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_BC3_UNORM_SRGB,
            width: 8,
            height: 8,
            mip_count: 2,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string()),
            subresources: vec![
                MapFrameTextureSubresource {
                    mip_slice: 0,
                    array_slice: 0,
                    file_offset: 0,
                    byte_len: 64,
                    row_pitch: 32,
                    depth_pitch: 64,
                    width: 8,
                    height: 8,
                },
                MapFrameTextureSubresource {
                    mip_slice: 1,
                    array_slice: 0,
                    file_offset: 64,
                    byte_len: 16,
                    row_pitch: 16,
                    depth_pitch: 16,
                    width: 4,
                    height: 4,
                },
            ],
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: 0,
        };

        native_d3d11_validate_texture_payload_shape(&resource)
            .expect("BC3 mapped subresources are enough for exact native upload");
    }

    #[test]
    fn native_d3d11_payload_accepts_exact_bc1_mapped_subresources() {
        let resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 7,
            declared_name: Some("IceNoise".to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_BC1_UNORM,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 2,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: Some("water_frame_draw000_ps_srv_slot007_IceNoise.d3d11".to_string()),
            byte_len: 40,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_BC1_UNORM,
            width: 8,
            height: 8,
            mip_count: 2,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string()),
            subresources: vec![
                MapFrameTextureSubresource {
                    mip_slice: 0,
                    array_slice: 0,
                    file_offset: 0,
                    byte_len: 32,
                    row_pitch: 16,
                    depth_pitch: 32,
                    width: 8,
                    height: 8,
                },
                MapFrameTextureSubresource {
                    mip_slice: 1,
                    array_slice: 0,
                    file_offset: 32,
                    byte_len: 8,
                    row_pitch: 8,
                    depth_pitch: 8,
                    width: 4,
                    height: 4,
                },
            ],
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: 0,
        };

        native_d3d11_validate_texture_payload_shape(&resource)
            .expect("BC1 mapped subresources are enough for exact native upload");
    }

    /// Build a BGRA8_SRGB reflection-cube-shaped binding (WATER1 `--water-full`
    /// real-cube path). `size` is the face dimension, `mips` the mip count,
    /// with tightly-packed subresources (row_pitch = width*4). Faces are ordered
    /// array_slice 0..6, each carrying its full mip chain. When
    /// `drop_last_subresource` is true the final subresource is removed so the
    /// subresource count no longer equals mips*6 (a malformed cube payload).
    fn reflection_cube_binding(
        size: u32,
        mips: u32,
        drop_last_subresource: bool,
    ) -> MapFrameTextureBinding {
        let mut subresources = Vec::new();
        let mut offset = 0usize;
        for array_slice in 0..6u32 {
            for mip_slice in 0..mips {
                let w = (size >> mip_slice).max(1);
                let h = (size >> mip_slice).max(1);
                let row_pitch = w * 4;
                let byte_len = (row_pitch * h) as usize;
                subresources.push(MapFrameTextureSubresource {
                    mip_slice,
                    array_slice,
                    file_offset: offset,
                    byte_len,
                    row_pitch,
                    depth_pitch: byte_len as u32,
                    width: w,
                    height: h,
                });
                offset += byte_len;
            }
        }
        let byte_len = offset;
        if drop_last_subresource {
            subresources.pop();
        }
        MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 8,
            declared_name: Some("ReflectionCubeMap".to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURECUBE,
                most_detailed_mip: 0,
                mip_levels: mips,
                first_array_slice: 0,
                array_size: 6,
                num_cubes: 1,
            }),
            file: Some("reflection_cube.d3d11".to_string()),
            byte_len,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            width: size,
            height: size,
            mip_count: mips,
            array_size: 6,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string()),
            subresources,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: D3D11_RESOURCE_MISC_TEXTURECUBE,
        }
    }

    #[test]
    fn native_d3d11_payload_accepts_wellformed_reflection_cube() {
        // 4x4x6x3-mip BGRA8_SRGB cube — the shape of the shipped/captured
        // reflection cube (map/terrain/reflection.dds) in miniature. WATER1
        // `--water-full` uploads exactly this via the generic validated path.
        let resource = reflection_cube_binding(4, 3, false);
        native_d3d11_validate_texture_payload_shape(&resource)
            .expect("well-formed 6-face mip-chained reflection cube must validate");
    }

    #[test]
    fn native_d3d11_payload_rejects_reflection_cube_missing_a_face_mip() {
        // Fail-closed: a cube missing one (mip, face) subresource so the count
        // no longer equals mip_count * 6. The `--water-full` real-cube path must
        // refuse this rather than upload a malformed cube.
        let resource = reflection_cube_binding(4, 3, true);
        let err = native_d3d11_validate_texture_payload_shape(&resource)
            .expect_err("malformed reflection cube (missing subresource) must fail closed");
        let message = err.to_string();
        assert!(
            message.contains("subresource"),
            "expected a subresource-count failure, got: {message}"
        );
    }

    #[test]
    fn native_d3d11_payload_rejects_reflection_cube_truncated_payload() {
        // Fail-closed: subresources present but byte_len shrunk below what they
        // cover, so the payload does not fully cover the declared bytes.
        let mut resource = reflection_cube_binding(4, 3, false);
        resource.byte_len -= 4;
        let err = native_d3d11_validate_texture_payload_shape(&resource)
            .expect_err("reflection cube with truncated payload must fail closed");
        let message = err.to_string();
        assert!(
            message.contains("byte") || message.contains("cover") || message.contains("exceeds"),
            "expected a byte-coverage failure, got: {message}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn shipped_reflection_cube_contract_accepts_exact_shipped_shape() {
        // 256x256x6x9-mip BGRA8_SRGB TEXTURECUBE == the shipped
        // map/terrain/reflection.dds contract. --water-full must accept it.
        let resource = reflection_cube_binding(256, 9, false);
        native_d3d11_validate_shipped_reflection_cube_contract(0, &resource)
            .expect("exact shipped reflection cube shape must pass the --water-full contract");
    }

    #[cfg(windows)]
    #[test]
    fn shipped_reflection_cube_contract_rejects_wrong_size_and_format() {
        // Wrong face size (128 not 256) and wrong format (UNORM not SRGB): the
        // --water-full contract must fail closed naming both deviations so a
        // shape-valid but wrong-provenance cube cannot be uploaded in place of
        // the shipped reflection.dds.
        let mut resource = reflection_cube_binding(128, 9, false);
        resource.dxgi_format = DXGI_FORMAT_B8G8R8A8_UNORM;
        if let Some(view) = resource.view_desc.as_mut() {
            view.format = DXGI_FORMAT_B8G8R8A8_UNORM;
        }
        let err = native_d3d11_validate_shipped_reflection_cube_contract(3, &resource)
            .expect_err("wrong-size/format cube must fail the --water-full contract");
        let message = err.to_string();
        assert!(message.contains("reflection.dds"), "{message}");
        assert!(message.contains("dims 128x128"), "{message}");
        assert!(message.contains("B8G8R8A8_UNORM_SRGB"), "{message}");
    }

    #[cfg(windows)]
    #[test]
    fn shipped_reflection_cube_contract_rejects_missing_cube_misc_flag() {
        // A 6-slice array that is not flagged TEXTURECUBE is not a cube.
        let mut resource = reflection_cube_binding(256, 9, false);
        resource.misc_flags = 0;
        let err = native_d3d11_validate_shipped_reflection_cube_contract(0, &resource)
            .expect_err("array without TEXTURECUBE misc flag must fail the contract");
        assert!(
            err.to_string().contains("D3D11_RESOURCE_MISC_TEXTURECUBE"),
            "{err}"
        );
    }

    #[test]
    fn native_d3d11_payload_rejects_missing_texture_array_size() {
        let resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 4,
            declared_name: Some("SpecularMap".to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_BC3_UNORM,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 1,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: Some("water_frame_draw000_ps_srv_slot004_SpecularMap.d3d11".to_string()),
            byte_len: 16,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_BC3_UNORM,
            width: 4,
            height: 4,
            mip_count: 1,
            array_size: 0,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string()),
            subresources: vec![MapFrameTextureSubresource {
                mip_slice: 0,
                array_slice: 0,
                file_offset: 0,
                byte_len: 16,
                row_pitch: 16,
                depth_pitch: 16,
                width: 4,
                height: 4,
            }],
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: 0,
        };

        let err = native_d3d11_validate_texture_payload_shape(&resource)
            .expect_err("native replay must not infer missing Texture2D ArraySize");
        assert!(err.to_string().contains("Texture2D ArraySize"));
        assert!(err.to_string().contains("recapture"));
    }

    #[test]
    fn native_d3d11_payload_rejects_texturecube_without_misc_flag() {
        let subresources = (0..6)
            .map(|array_slice| MapFrameTextureSubresource {
                mip_slice: 0,
                array_slice,
                file_offset: usize::try_from(array_slice * 16).unwrap(),
                byte_len: 16,
                row_pitch: 8,
                depth_pitch: 16,
                width: 2,
                height: 2,
            })
            .collect::<Vec<_>>();
        let mut resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 8,
            declared_name: Some("ReflectionCubeMap".to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURECUBE,
                most_detailed_mip: 0,
                mip_levels: 1,
                first_array_slice: 0,
                array_size: 6,
                num_cubes: 0,
            }),
            file: Some("water_frame_draw000_ps_srv_slot008_ReflectionCubeMap.d3d11".to_string()),
            byte_len: 96,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
            width: 2,
            height: 2,
            mip_count: 1,
            array_size: 6,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string()),
            subresources,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: 0,
        };

        let err = native_d3d11_validate_texture_payload_shape(&resource)
            .expect_err("TextureCube replay must require captured cube misc flag");
        assert!(err.to_string().contains("D3D11_RESOURCE_MISC_TEXTURECUBE"));

        resource.misc_flags = D3D11_RESOURCE_MISC_TEXTURECUBE;
        native_d3d11_validate_texture_payload_shape(&resource)
            .expect("captured cube misc flag makes the TextureCube payload replayable");
    }

    #[test]
    fn native_d3d11_request_records_bundle_and_optional_output() {
        let camera = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [1.0, 2.0, 3.0],
        )
        .expect("finite production camera");
        let request = NativeD3d11ReplayRequest::new("capture/map_frame_bundle.json")
            .with_output("capture/terrain_native.r16g16b16a16_float")
            .with_readback()
            .with_per_draw_comparison()
            .with_production_camera(camera);

        assert_eq!(
            request.map_frame_bundle,
            PathBuf::from("capture/map_frame_bundle.json")
        );
        assert_eq!(
            request.output,
            Some(PathBuf::from("capture/terrain_native.r16g16b16a16_float"))
        );
        assert!(request.readback);
        assert!(request.compare_each_draw);
        assert_eq!(request.production_camera, Some(camera));
    }

    #[test]
    fn production_map_d3d11_request_records_named_layer_bundles_and_camera() {
        let camera = ProductionMapCamera::new(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [4.0, 5.0, 6.0],
        )
        .expect("finite production camera");
        let request = ProductionMapD3d11ReplayRequest::new("capture/map_frame_bundle.json", camera)
            .with_water_map_frame_bundle("capture/map_frame_water_pass_bundle.json")
            .with_rivers_map_frame_bundle("capture/map_frame_rivers_pass_bundle.json")
            .with_borders_map_frame_bundle("capture/map_frame_borders_pass_bundle.json")
            .with_trees_map_frame_bundle("capture/map_frame_trees_pass_bundle.json")
            .with_output("capture/map_native.b8g8r8a8")
            .with_readback()
            .with_per_draw_comparison();

        assert_eq!(
            request.terrain_map_frame_bundle,
            PathBuf::from("capture/map_frame_bundle.json")
        );
        assert_eq!(
            request.water_map_frame_bundle,
            Some(PathBuf::from("capture/map_frame_water_pass_bundle.json"))
        );
        assert_eq!(
            request.rivers_map_frame_bundle,
            Some(PathBuf::from("capture/map_frame_rivers_pass_bundle.json"))
        );
        assert_eq!(
            request.borders_map_frame_bundle,
            Some(PathBuf::from("capture/map_frame_borders_pass_bundle.json"))
        );
        assert_eq!(
            request.trees_map_frame_bundle,
            Some(PathBuf::from("capture/map_frame_trees_pass_bundle.json"))
        );
        assert_eq!(
            request.output,
            Some(PathBuf::from("capture/map_native.b8g8r8a8"))
        );
        assert!(request.readback);
        assert!(request.compare_each_draw);
        assert_eq!(request.production_camera, camera);
    }

    #[test]
    fn native_d3d11_render_options_record_output_and_readback() {
        let options = NativeD3d11RenderOptions::default()
            .with_output("capture/terrain_native.r16g16b16a16_float")
            .with_readback()
            .with_per_draw_comparison();

        assert_eq!(
            options.output,
            Some(PathBuf::from("capture/terrain_native.r16g16b16a16_float"))
        );
        assert!(options.readback);
        assert!(options.compare_each_draw);
        assert!(options.compare_final_reference);
        assert!(options.clear_targets);
        assert_eq!(options.map_clock_time, None);

        let interactive_options = options.without_final_reference_comparison();
        assert!(!interactive_options.compare_final_reference);
    }

    #[test]
    fn native_d3d11_render_options_map_clock_is_explicit_opt_in() {
        let default_options = NativeD3d11RenderOptions::default();
        assert_eq!(default_options.map_clock_time, None);

        let animated_options = NativeD3d11RenderOptions::default().with_map_clock_time(10.5);
        assert_eq!(animated_options.map_clock_time, Some(10.5));
    }

    #[test]
    fn native_d3d11_render_options_record_parity_report_dir() {
        let options = NativeD3d11RenderOptions::default().with_parity_report("capture/parity");

        assert_eq!(options.parity_report, Some(PathBuf::from("capture/parity")));
        assert!(options.compare_final_reference);
    }

    #[test]
    fn terrain_d3d11_acceptance_request_records_output_report_and_repeat() {
        let request = TerrainD3d11AcceptanceRequest::new("capture/map_frame_bundle.json")
            .with_output("capture/terrain_native.r16g16b16a16_float")
            .with_report("capture/terrain_d3d11_acceptance.json")
            .with_repeat(3);

        assert_eq!(
            request.map_frame_bundle,
            PathBuf::from("capture/map_frame_bundle.json")
        );
        assert_eq!(
            request.output,
            Some(PathBuf::from("capture/terrain_native.r16g16b16a16_float"))
        );
        assert_eq!(
            request.report,
            Some(PathBuf::from("capture/terrain_d3d11_acceptance.json"))
        );
        assert_eq!(request.repeat, 3);
    }

    #[test]
    fn terrain_d3d11_acceptance_report_records_reference_hashes() {
        let root = temp_test_dir("terrain_d3d11_acceptance_report_hashes");
        fs::create_dir_all(&root).expect("create temp capture dir");
        let bundle_path = root.join(MAP_FRAME_BUNDLE_FILE);
        let session_path = root.join(MAP_CAPTURE_SESSION_FILE);
        fs::write(&bundle_path, b"{\"schema\":\"bundle\"}").expect("write bundle");
        fs::write(&session_path, b"{\"schema\":\"session\"}").expect("write session");
        let bundle_hash = fnv1a64_file(&bundle_path).expect("bundle hash");
        let session_hash = fnv1a64_file(&session_path).expect("session hash");
        let result = NativeD3d11ReplayResult {
            summary: NativeD3d11ReplaySummary {
                draw_count: 7,
                vertex_buffer_count: 14,
                index_buffer_count: 7,
                constant_buffer_count: 35,
                shader_resource_count: 112,
                sampler_count: 112,
                target_width: 1920,
                target_height: 1080,
                target_format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            },
            readback: None,
            reference_comparison: Some(NativeD3d11ReferenceComparison {
                byte_len: 16_588_800,
                expected_fnv1a64: "e8273bf5ea8f8a6c".to_string(),
                actual_fnv1a64: "e8273bf5ea8f8a6c".to_string(),
            }),
        };

        let report = terrain_d3d11_acceptance_report(&bundle_path, 2, &result).expect("report");

        assert_eq!(report.schema, TERRAIN_D3D11_ACCEPTANCE_REPORT_SCHEMA);
        assert_eq!(report.status, "accepted");
        assert_eq!(report.capture_moment, TERRAIN_CAPTURE_MOMENT);
        assert_eq!(report.bundle_file, MAP_FRAME_BUNDLE_FILE);
        assert_eq!(report.map_frame_bundle_fnv1a64, bundle_hash);
        assert_eq!(report.map_capture_session_fnv1a64, session_hash);
        assert_eq!(report.repeat, 2);
        assert!(report.compare_each_draw);
        assert_eq!(report.draw_count, 7);
        assert_eq!(report.final_reference_byte_len, 16_588_800);
        assert_eq!(report.final_expected_fnv1a64, "e8273bf5ea8f8a6c");
        assert_eq!(report.final_actual_fnv1a64, "e8273bf5ea8f8a6c");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn terrain_d3d11_acceptance_report_requires_reference_comparison() {
        let result = NativeD3d11ReplayResult {
            summary: NativeD3d11ReplaySummary {
                draw_count: 7,
                vertex_buffer_count: 14,
                index_buffer_count: 7,
                constant_buffer_count: 35,
                shader_resource_count: 112,
                sampler_count: 112,
                target_width: 1920,
                target_height: 1080,
                target_format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            },
            readback: None,
            reference_comparison: None,
        };

        let err =
            terrain_d3d11_acceptance_report(Path::new("capture/map_frame_bundle.json"), 1, &result)
                .expect_err("missing reference comparison must fail closed");
        assert!(err.to_string().contains("reference comparison proof"));
    }

    #[test]
    fn native_d3d11_terrain_bundle_loads_validated_map_frame_contract() {
        let root = temp_test_dir("native_d3d11_loaded_bundle_valid");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.draw_state = Some(valid_map_frame_draw_state());
            for draw in &mut frame.draws {
                draw.draw_state = Some(valid_map_frame_draw_state());
            }
            frame
        });

        let bundle =
            NativeD3d11TerrainBundle::load(&path).expect("valid native D3D11 terrain bundle");

        assert_eq!(bundle.map_frame_bundle(), path.as_path());
        assert_eq!(bundle.bundle_dir(), root.as_path());
        assert_eq!(bundle.draw_count(), 2);
        assert_eq!(bundle.active_draw_index(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_camera_derives_from_captured_hoi4_scene_cb0() {
        let root = temp_test_dir("production_map_camera_from_capture");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.draw_state = Some(valid_map_frame_draw_state());
            for draw in &mut frame.draws {
                draw.draw_state = Some(valid_map_frame_draw_state());
            }
            frame
        });
        let bundle =
            NativeD3d11TerrainBundle::load(&path).expect("valid native D3D11 terrain bundle");

        let camera = bundle
            .production_camera_from_capture()
            .expect("derive app-owned camera from the game TerrainSceneCB0");

        assert_eq!(
            camera.view_projection_matrix,
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        );
        assert_close(camera.position[0], 2695.059);
        assert_close(camera.position[1], 310.11);
        assert_close(camera.position[2], 1569.962);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn production_map_camera_derivation_rejects_per_draw_camera_drift() {
        let root = temp_test_dir("production_map_camera_from_capture_drift");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.active_draw.draw_state = Some(valid_map_frame_draw_state());
            for draw in &mut frame.draws {
                draw.draw_state = Some(valid_map_frame_draw_state());
            }
            let mut drift_bytes = valid_scene_cb0_bytes();
            write_f32_le(&mut drift_bytes, 208, 1234.0);
            let drift_file = "map_frame_draw001_vs_constant_buffer_slot0_drift.bin";
            fs::write(root.join(drift_file), &drift_bytes).expect("write drifted scene CB0");
            let drift_hash = fnv1a64_file(&root.join(drift_file)).expect("hash drifted scene CB0");
            let binding = frame
                .active_draw
                .constant_buffers
                .iter_mut()
                .find(|binding| {
                    binding.stage == "VS"
                        && binding.slot == TERRAIN_SCENE_CB0_BIND_POINT
                        && binding.declared_name.as_deref() == Some(TERRAIN_SCENE_CB0_SYMBOL)
                })
                .expect("active VS TerrainSceneCB0 binding");
            binding.file = Some(drift_file.to_string());
            binding.byte_len = drift_bytes.len();
            binding.fnv1a64 = Some(drift_hash);
            frame
        });
        let bundle =
            NativeD3d11TerrainBundle::load(&path).expect("valid native D3D11 terrain bundle");

        let err = bundle
            .production_camera_from_capture()
            .expect_err("per-draw camera drift must fail closed");

        assert!(err.to_string().contains("camera constants differ"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn native_d3d11_result_can_carry_tightly_packed_readback() {
        let result = NativeD3d11ReplayResult {
            summary: NativeD3d11ReplaySummary {
                draw_count: 7,
                vertex_buffer_count: 14,
                index_buffer_count: 7,
                constant_buffer_count: 35,
                shader_resource_count: 112,
                sampler_count: 112,
                target_width: 2,
                target_height: 1,
                target_format: DXGI_FORMAT_R16G16B16A16_FLOAT,
            },
            readback: Some(NativeD3d11ReplayReadback {
                width: 2,
                height: 1,
                dxgi_format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                row_pitch: 16,
                bytes: vec![0; 16],
            }),
            reference_comparison: None,
        };

        let readback = result.readback.as_ref().expect("readback");
        assert_eq!(readback.row_pitch, 16);
        assert_eq!(readback.bytes.len(), 16);
    }

    #[test]
    fn native_d3d11_payload_rejects_mipped_texture_without_subresource_layout() {
        let resource = MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 0,
            declared_name: Some(TERRAIN_DIFFUSE_SYMBOL.to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: DXGI_FORMAT_BC3_UNORM_SRGB,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 2,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: Some("terrain_diffuse.d3d11tex".to_string()),
            byte_len: 80,
            fnv1a64: Some("hash".to_string()),
            dxgi_format: DXGI_FORMAT_BC3_UNORM_SRGB,
            width: 8,
            height: 8,
            mip_count: 2,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            usage: 1,
            cpu_access_flags: 0,
            misc_flags: 0,
        };

        let err = native_d3d11_validate_texture_payload_shape(&resource)
            .expect_err("mipped native replay texture must carry subresource layout");
        assert!(err
            .to_string()
            .contains(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT));
    }

    #[test]
    fn native_d3d11_payload_reports_fileless_shipped_srv_blocker() {
        let root = temp_test_dir("native_d3d11_fileless_shipped_srv");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .shader
                .reflection_resources
                .insert(TERRAIN_DIFFUSE_SYMBOL.to_string(), 0);
            frame
                .active_draw
                .shader_resources
                .push(MapFrameTextureBinding {
                    stage: "PS".to_string(),
                    slot: 0,
                    declared_name: Some(TERRAIN_DIFFUSE_SYMBOL.to_string()),
                    is_null: false,
                    view_desc: None,
                    file: None,
                    byte_len: 0,
                    fnv1a64: None,
                    dxgi_format: DXGI_FORMAT_BC3_UNORM_SRGB,
                    width: 2048,
                    height: 2048,
                    mip_count: 10,
                    array_size: 1,
                    sample_count: 1,
                    sample_quality: 0,
                    payload_format: None,
                    subresources: Vec::new(),
                    bind_flags: D3D11_BIND_SHADER_RESOURCE,
                    usage: 1,
                    cpu_access_flags: 0,
                    misc_flags: 0,
                });
            frame
        });
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid legacy bundle");
        let blockers = native_d3d11_replay_payload_blockers(&frame, false);

        assert!(blockers.iter().any(|blocker| {
            blocker.contains(TERRAIN_DIFFUSE_SYMBOL)
                && blocker.contains("no raw texture payload file")
        }));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn native_d3d11_payload_reports_missing_reflected_srv_binding() {
        let root = temp_test_dir("native_d3d11_missing_reflected_srv");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame
                .active_draw
                .shader
                .reflection_resources
                .insert(TERRAIN_DIFFUSE_SYMBOL.to_string(), 0);
            frame
        });
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid legacy bundle");
        let blockers = native_d3d11_replay_payload_blockers(&frame, false);

        assert!(blockers.iter().any(|blocker| {
            blocker.contains(TERRAIN_DIFFUSE_SYMBOL) && blocker.contains("no captured SRV binding")
        }));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn native_d3d11_payload_rejects_msaa_pre_draw_om_payloads_for_dependent_passes() {
        let root = temp_test_dir("native_d3d11_dependent_msaa_pre_draw_payload");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            frame.capture_moment = WATER_CAPTURE_MOMENT.to_string();
            let mut state = valid_map_frame_draw_state();
            state.depth_stencil = MapFrameDepthStencilState {
                default_state: false,
                stencil_ref: 0,
                desc: Some(valid_depth_enabled_desc()),
            };
            let mut output_merger = valid_output_merger_state();
            let render_payload = MapFrameOutputMergerTexturePayload {
                file: "pre_draw_rt.d3d11".to_string(),
                byte_len: 8,
                fnv1a64: "hash".to_string(),
                payload_format: NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string(),
                subresources: vec![MapFrameTextureSubresource {
                    mip_slice: 0,
                    array_slice: 0,
                    file_offset: 0,
                    byte_len: 8,
                    row_pitch: 8,
                    depth_pitch: 8,
                    width: 2,
                    height: 1,
                }],
                provenance: "unit_test_resolved_msaa_snapshot".to_string(),
            };
            output_merger.render_targets[0].desc.sample_count = 4;
            output_merger.render_targets[0].pre_draw_payload = Some(render_payload);
            let depth_payload = MapFrameOutputMergerTexturePayload {
                file: "pre_draw_depth.d3d11".to_string(),
                byte_len: 8,
                fnv1a64: "hash".to_string(),
                payload_format: NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string(),
                subresources: vec![MapFrameTextureSubresource {
                    mip_slice: 0,
                    array_slice: 0,
                    file_offset: 0,
                    byte_len: 8,
                    row_pitch: 8,
                    depth_pitch: 8,
                    width: 2,
                    height: 1,
                }],
                provenance: "unit_test_resolved_msaa_snapshot".to_string(),
            };
            output_merger
                .depth_stencil
                .as_mut()
                .expect("depth-stencil target")
                .desc
                .sample_count = 4;
            output_merger
                .depth_stencil
                .as_mut()
                .expect("depth-stencil target")
                .pre_draw_payload = Some(depth_payload);
            state.output_merger = Some(output_merger);
            frame.active_draw.draw_state = Some(state.clone());
            for draw in &mut frame.draws {
                draw.draw_state = Some(state.clone());
            }
            frame
        });
        let frame: MapFrameRuntimeBundle =
            serde_json::from_slice(&fs::read(&path).expect("read map-frame bundle"))
                .expect("parse dependent map-frame bundle");
        let blockers = native_d3d11_replay_payload_blockers(&frame, true);

        assert!(blockers.iter().any(|blocker| {
            blocker.contains("render target")
                && blocker.contains("4x MSAA")
                && blocker.contains("per-sample")
        }));
        assert!(blockers.iter().any(|blocker| {
            blocker.contains("depth-stencil")
                && blocker.contains("4x MSAA")
                && blocker.contains("per-sample")
        }));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn native_d3d11_production_dependent_pass_does_not_require_pre_draw_payloads() {
        let root = temp_test_dir("native_d3d11_dependent_pass_uses_graph_target");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_water_map_frame_bundle(&root, |mut frame| {
            let mut state = valid_map_frame_draw_state();
            state.depth_stencil = MapFrameDepthStencilState {
                default_state: false,
                stencil_ref: 0,
                desc: Some(valid_depth_enabled_desc()),
            };
            state.output_merger = Some(valid_output_merger_state());
            frame.active_draw.draw_state = Some(state.clone());
            for draw in &mut frame.draws {
                draw.draw_state = Some(state.clone());
            }
            frame
        });
        NativeD3d11TerrainBundle::load_water(&path)
            .expect("production dependent water pass does not need pre-draw payload bytes");
        let frame = validate_map_frame_runtime_bundle_for(&path, WATER_MAP_FRAME_SPEC)
            .expect("valid water map-frame bundle");

        let production_blockers = native_d3d11_replay_payload_blockers(&frame, false);
        assert!(
            !production_blockers
                .iter()
                .any(|blocker| blocker.contains("pre-draw payload")),
            "{production_blockers:?}"
        );

        let standalone_blockers = native_d3d11_replay_payload_blockers(&frame, true);
        assert!(
            standalone_blockers
                .iter()
                .any(|blocker| blocker.contains("pre-draw payload")),
            "{standalone_blockers:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn native_d3d11_terrain_first_pass_owns_target_without_pre_draw_payloads() {
        let root = temp_test_dir("native_d3d11_terrain_owns_target");
        fs::create_dir_all(&root).expect("create temp bundle dir");
        let path = write_valid_map_frame_bundle(&root, |mut frame| {
            let mut state = valid_map_frame_draw_state();
            state.depth_stencil = MapFrameDepthStencilState {
                default_state: false,
                stencil_ref: 0,
                desc: Some(valid_depth_enabled_desc()),
            };
            state.output_merger = Some(valid_output_merger_state());
            frame.active_draw.draw_state = Some(state.clone());
            for draw in &mut frame.draws {
                draw.draw_state = Some(state.clone());
            }
            frame
        });
        let frame = validate_map_frame_runtime_bundle(&path).expect("valid map-frame bundle");
        let blockers = native_d3d11_replay_payload_blockers(&frame, false);

        assert!(
            !blockers
                .iter()
                .any(|blocker| blocker.contains("pre-draw payload")),
            "{blockers:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
