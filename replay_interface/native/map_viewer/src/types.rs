use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ProvinceDefinition {
    pub(crate) rgb: [u8; 3],
    pub(crate) province_type: String,
    pub(crate) coastal: bool,
    pub(crate) terrain: String,
    pub(crate) continent: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct StateMeta {
    pub(crate) name_key: String,
    pub(crate) provinces: Vec<u32>,
    pub(crate) victory_points: BTreeMap<u32, i32>,
    pub(crate) vp_total: i32,
    pub(crate) state_category: Option<String>,
    pub(crate) manpower: u64,
    #[serde(default)]
    pub(crate) impassable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StrategicRegionMeta {
    pub(crate) name: String,
    pub(crate) province_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub(crate) schema: String,
    pub(crate) dimensions: Dimensions,
    pub(crate) province_id_encoding: String,
    pub(crate) files: BundleFiles,
    pub(crate) counts: BundleCounts,
    pub(crate) source: BundleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) modset: Option<BundleModSet>,
    pub(crate) viewer_note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Dimensions {
    pub(crate) width: usize,
    pub(crate) height: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BundleFiles {
    pub(crate) province_ids: String,
    #[serde(default = "default_terrain_id_map_file")]
    pub(crate) terrain_id_map: String,
    #[serde(default = "default_terrain_runtime_file")]
    pub(crate) terrain_runtime: String,
    #[serde(default = "default_snow_mud_data_file")]
    pub(crate) snow_mud_data: String,
    #[serde(default = "default_terrain_scene_cb0_file")]
    pub(crate) terrain_scene_cb0: String,
    #[serde(default = "default_province_secondary_color_map_file")]
    pub(crate) province_secondary_color_map: String,
    #[serde(default = "default_gradient_border_channel1_file")]
    pub(crate) gradient_border_channel1: String,
    #[serde(default = "default_gradient_border_channel2_file")]
    pub(crate) gradient_border_channel2: String,
    #[serde(default = "default_gradient_border_channel3_file")]
    pub(crate) gradient_border_channel3: String,
    #[serde(default = "default_light_index_map_file")]
    pub(crate) light_index_map: String,
    #[serde(default = "default_light_data_map_file")]
    pub(crate) light_data_map: String,
    #[serde(default = "default_shadow_map_file")]
    pub(crate) shadow_map: String,
    #[serde(default = "default_terrain_indices_file")]
    pub(crate) terrain_indices: String,
    #[serde(default = "default_terrain_texture_ids_file")]
    pub(crate) terrain_texture_ids: String,
    #[serde(default = "default_heightmap_file")]
    pub(crate) heightmap: String,
    #[serde(default = "default_rivers_file")]
    pub(crate) rivers: String,
    #[serde(default = "default_height_normal_file")]
    pub(crate) height_normal: String,
    #[serde(default = "default_cities_file")]
    pub(crate) cities: String,
    #[serde(default = "default_trees_file")]
    pub(crate) trees: String,
    #[serde(default = "default_tree_tint_file")]
    pub(crate) tree_tint: String,
    #[serde(default = "default_tree_season_file")]
    pub(crate) tree_season: String,
    #[serde(default = "default_terrain_diffuse_atlas_file")]
    pub(crate) terrain_diffuse_atlas: String,
    #[serde(default = "default_terrain_normal_atlas_file")]
    pub(crate) terrain_normal_atlas: String,
    #[serde(default = "default_terrain_diffuse_atlas_mips_file")]
    pub(crate) terrain_diffuse_atlas_mips: String,
    #[serde(default = "default_terrain_normal_atlas_mips_file")]
    pub(crate) terrain_normal_atlas_mips: String,
    #[serde(default)]
    pub(crate) shader_sources: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) map_sources: BTreeMap<String, String>,
    #[serde(default = "default_map_object_meta_file")]
    pub(crate) map_object_meta: String,
    #[serde(default = "default_map_material_contract_file")]
    pub(crate) map_material_contract: String,
    pub(crate) province_to_state: String,
    pub(crate) state_meta: String,
    pub(crate) state_owner_1936: String,
    pub(crate) country_colors: String,
    pub(crate) province_definitions: String,
    pub(crate) province_to_strategic_region: String,
    pub(crate) strategic_region_to_provinces: String,
    pub(crate) strategic_region_meta: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BundleCounts {
    pub(crate) definition_rows: usize,
    pub(crate) distinct_ids_in_provinces_bmp: usize,
    pub(crate) states: usize,
    pub(crate) state_owners_1936: usize,
    pub(crate) province_to_state: usize,
    pub(crate) country_colors: usize,
    pub(crate) strategic_regions: usize,
    pub(crate) province_to_strategic_region: usize,
    pub(crate) unknown_pixels: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BundleSource {
    pub(crate) game_dir_name: String,
    pub(crate) default_map: String,
    pub(crate) definition_csv_fnv1a64: String,
    pub(crate) provinces_bmp_fnv1a64: String,
    #[serde(default)]
    pub(crate) visual_input_fnv1a64: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) visual_input_dimensions: BTreeMap<String, Dimensions>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BundleModSet {
    pub(crate) modset_id: String,
    pub(crate) selection_source: String,
    pub(crate) mod_count: usize,
    pub(crate) mods: Vec<BundleMod>,
    pub(crate) overridden_file_count: usize,
    pub(crate) replace_path_excluded_file_count: usize,
    pub(crate) warnings: Vec<String>,
    pub(crate) note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct BundleMod {
    pub(crate) load_order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) supported_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workshop_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) descriptor: Option<String>,
    pub(crate) replace_path: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapObjectMeta {
    pub(crate) default_map: DefaultMapMeta,
    pub(crate) terrain: TerrainSourceMeta,
    pub(crate) cities: CitySourceMeta,
    pub(crate) indexed_maps: BTreeMap<String, IndexedMapMeta>,
    pub(crate) seasons: SeasonSourceMeta,
    pub(crate) static_sources: BTreeMap<String, TabularSourceMeta>,
    pub(crate) ambient_objects: AmbientSourceMeta,
    pub(crate) common_building_rules: BTreeMap<String, CommonBuildingRuleMeta>,
    pub(crate) render_source_catalogs: BTreeMap<String, TextSourceCatalog>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DefaultMapMeta {
    pub(crate) file_references: BTreeMap<String, String>,
    pub(crate) tree_indices: Vec<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainSourceMeta {
    pub(crate) index_to_texture_id: BTreeMap<u8, u8>,
    pub(crate) terrain_index_counts: BTreeMap<u8, usize>,
    pub(crate) unmapped_index_counts: BTreeMap<u8, usize>,
    pub(crate) implemented_parity_level: String,
    pub(crate) generated_runtime_gaps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CitySourceMeta {
    pub(crate) types_source: Option<String>,
    pub(crate) pixel_step_x: Option<u32>,
    pub(crate) pixel_step_y: Option<u32>,
    pub(crate) city_groups: Vec<CityGroupMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CityGroupMeta {
    pub(crate) color_index: Option<u32>,
    pub(crate) density: Option<f32>,
    pub(crate) buildings: Vec<CityBuildingMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CityBuildingMeta {
    pub(crate) distance: Option<u32>,
    pub(crate) meshes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct IndexedMapMeta {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) distinct_indices: usize,
    pub(crate) nonzero_pixels: usize,
    pub(crate) index_counts: BTreeMap<u8, usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SeasonSourceMeta {
    pub(crate) blocks: Vec<SeasonBlockMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SeasonBlockMeta {
    pub(crate) name: String,
    pub(crate) start_date: Option<String>,
    pub(crate) end_date: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TabularSourceMeta {
    pub(crate) rows: usize,
    pub(crate) malformed_rows: usize,
    pub(crate) field_counts: BTreeMap<usize, usize>,
    pub(crate) category_column: Option<usize>,
    pub(crate) category_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AmbientSourceMeta {
    pub(crate) type_blocks: usize,
    pub(crate) object_blocks: usize,
    pub(crate) type_counts: BTreeMap<String, usize>,
    pub(crate) object_name_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CommonBuildingRuleMeta {
    pub(crate) source: String,
    pub(crate) show_on_map: Option<u32>,
    pub(crate) icon_frame: Option<u32>,
    pub(crate) value: Option<u32>,
    pub(crate) spawn_point: Option<String>,
    pub(crate) province_max: Option<u32>,
    pub(crate) state_max: Option<u32>,
    pub(crate) centered: Option<bool>,
    pub(crate) always_shown: Option<bool>,
    pub(crate) only_display_if_exists: Option<bool>,
    pub(crate) tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TextSourceCatalog {
    pub(crate) bytes: u64,
    pub(crate) lines: usize,
    pub(crate) key_counts: BTreeMap<String, usize>,
    pub(crate) quoted_values: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapMaterialContract {
    pub(crate) implemented_visible_level: String,
    pub(crate) shader_constants: BTreeMap<String, ShaderConstantMeta>,
    #[serde(default)]
    pub(crate) shader_vector3_constants: BTreeMap<String, ShaderVector3Meta>,
    pub(crate) shipped_sources: BTreeMap<String, MaterialSourceMeta>,
    pub(crate) generated_runtime_inputs: BTreeMap<String, GeneratedRuntimeInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ShaderConstantMeta {
    pub(crate) value: f32,
    pub(crate) source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ShaderVector3Meta {
    pub(crate) value: [f32; 3],
    pub(crate) source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MaterialSourceMeta {
    pub(crate) output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mip_output: Option<String>,
    pub(crate) role: String,
    pub(crate) shader_samplers: Vec<String>,
    pub(crate) dimensions: Dimensions,
    pub(crate) dds_format: String,
    pub(crate) mip_count: u32,
    pub(crate) preservation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct GeneratedRuntimeInput {
    pub(crate) shader_symbols: Vec<String>,
    pub(crate) consumers: Vec<String>,
    pub(crate) status: String,
    pub(crate) reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainRuntimeInputs {
    pub(crate) schema: String,
    pub(crate) terrain_id_map: TerrainIdMapRuntimeMeta,
    pub(crate) map_pow2: MapPow2RuntimeMeta,
    pub(crate) fow_pow2: MapPow2RuntimeMeta,
    #[serde(default)]
    pub(crate) runtime_textures: BTreeMap<String, RuntimeTextureRuntimeMeta>,
    #[serde(default)]
    pub(crate) runtime_constant_buffers: BTreeMap<String, RuntimeConstantBufferRuntimeMeta>,
    #[serde(default)]
    pub(crate) runtime_vertex_buffers: BTreeMap<String, RuntimeVertexBufferRuntimeMeta>,
    #[serde(default)]
    pub(crate) runtime_input_layouts: BTreeMap<String, RuntimeInputLayoutRuntimeMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameRuntimeBundle {
    pub(crate) schema: String,
    #[serde(default)]
    pub(crate) provenance: String,
    pub(crate) capture_moment: String,
    pub(crate) session_manifest: String,
    #[serde(default)]
    pub(crate) shader_cache_constants: Option<MapFrameShaderCacheConstants>,
    pub(crate) active_draw: MapFrameDraw,
    #[serde(default)]
    pub(crate) draws: Vec<MapFrameDraw>,
    #[serde(default)]
    pub(crate) post_pass_output_merger: Option<MapFrameOutputMergerState>,
    #[serde(default)]
    pub(crate) vanilla_sources: BTreeMap<String, MapFrameFileEvidence>,
    #[serde(default)]
    pub(crate) runtime_files: BTreeMap<String, MapFrameFileEvidence>,
    #[serde(default)]
    pub(crate) blockers: Vec<String>,
    #[serde(skip)]
    pub(crate) synthetic_payloads: BTreeMap<String, MapFrameSyntheticPayload>,
    #[serde(skip)]
    pub(crate) vanilla_source_verification: Option<MapFrameVanillaSourceSeal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MapFrameVanillaSourceVerification {
    ResolverVerified,
    ProducerVerified,
    Unverified {
        unverified_sources: Vec<String>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct MapFrameSyntheticPayload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) fnv1a64: String,
    pub(crate) provenance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameShaderCacheConstants {
    pub(crate) map_size_x: f32,
    pub(crate) map_size_y: f32,
    pub(crate) map_pow2_x: f32,
    pub(crate) map_pow2_y: f32,
    pub(crate) fow_pow2_x: f32,
    pub(crate) fow_pow2_y: f32,
    pub(crate) terrain_id_srv_slot: u32,
    #[serde(default)]
    pub(crate) cache_files: Vec<String>,
    pub(crate) provenance: String,
    pub(crate) extraction: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapCaptureSession {
    pub(crate) schema: String,
    pub(crate) provenance: String,
    pub(crate) required_first_moment: String,
    pub(crate) moments: Vec<MapCaptureSessionMoment>,
    #[serde(default)]
    pub(crate) deferred_moments: Vec<MapCaptureDeferredMoment>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapCaptureSessionMoment {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) bundle_file: String,
    #[serde(default)]
    pub(crate) required_payload: Vec<String>,
    #[serde(default)]
    pub(crate) blockers: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapCaptureDeferredMoment {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) required_after: String,
    pub(crate) blocker: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameDraw {
    #[serde(default)]
    pub(crate) draw_index: u32,
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
    #[serde(default)]
    pub(crate) topology: Option<u32>,
    pub(crate) shader: MapFrameShader,
    #[serde(default)]
    pub(crate) vertex_shader: Option<MapFrameShader>,
    pub(crate) input_layout: MapFrameInputLayout,
    #[serde(default)]
    pub(crate) vertex_buffers: BTreeMap<String, MapFrameBufferBinding>,
    #[serde(default)]
    pub(crate) index_buffer: Option<MapFrameIndexBufferBinding>,
    #[serde(default)]
    pub(crate) constant_buffers: Vec<MapFrameBufferBinding>,
    #[serde(default)]
    pub(crate) shader_resources: Vec<MapFrameTextureBinding>,
    #[serde(default)]
    pub(crate) samplers: Vec<MapFrameSamplerBinding>,
    #[serde(default)]
    pub(crate) draw_state: Option<MapFrameDrawState>,
    #[serde(default)]
    pub(crate) post_draw_output_merger: Option<MapFrameOutputMergerState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameDrawState {
    #[serde(default)]
    pub(crate) viewports: Vec<MapFrameViewport>,
    #[serde(default)]
    pub(crate) scissor_rects: Vec<MapFrameScissorRect>,
    pub(crate) rasterizer: MapFrameRasterizerState,
    pub(crate) blend: MapFrameBlendState,
    pub(crate) depth_stencil: MapFrameDepthStencilState,
    #[serde(default)]
    pub(crate) output_merger: Option<MapFrameOutputMergerState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameViewport {
    pub(crate) top_left_x: f32,
    pub(crate) top_left_y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) min_depth: f32,
    pub(crate) max_depth: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameScissorRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameRasterizerState {
    pub(crate) default_state: bool,
    #[serde(default)]
    pub(crate) desc: Option<MapFrameRasterizerDesc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameRasterizerDesc {
    pub(crate) fill_mode: u32,
    pub(crate) cull_mode: u32,
    pub(crate) front_counter_clockwise: bool,
    pub(crate) depth_bias: i32,
    pub(crate) depth_bias_clamp: f32,
    pub(crate) slope_scaled_depth_bias: f32,
    pub(crate) depth_clip_enable: bool,
    pub(crate) scissor_enable: bool,
    pub(crate) multisample_enable: bool,
    pub(crate) antialiased_line_enable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameBlendState {
    pub(crate) default_state: bool,
    pub(crate) blend_factor: [f32; 4],
    pub(crate) sample_mask: u32,
    #[serde(default)]
    pub(crate) desc: Option<MapFrameBlendDesc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameBlendDesc {
    pub(crate) alpha_to_coverage_enable: bool,
    pub(crate) independent_blend_enable: bool,
    #[serde(default)]
    pub(crate) render_targets: Vec<MapFrameRenderTargetBlendDesc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameRenderTargetBlendDesc {
    pub(crate) blend_enable: bool,
    pub(crate) src_blend: u32,
    pub(crate) dest_blend: u32,
    pub(crate) blend_op: u32,
    pub(crate) src_blend_alpha: u32,
    pub(crate) dest_blend_alpha: u32,
    pub(crate) blend_op_alpha: u32,
    pub(crate) render_target_write_mask: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameDepthStencilState {
    pub(crate) default_state: bool,
    pub(crate) stencil_ref: u32,
    #[serde(default)]
    pub(crate) desc: Option<MapFrameDepthStencilDesc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameDepthStencilDesc {
    pub(crate) depth_enable: bool,
    pub(crate) depth_write_mask: u32,
    pub(crate) depth_func: u32,
    pub(crate) stencil_enable: bool,
    pub(crate) stencil_read_mask: u8,
    pub(crate) stencil_write_mask: u8,
    pub(crate) front_face: MapFrameDepthStencilOpDesc,
    pub(crate) back_face: MapFrameDepthStencilOpDesc,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameDepthStencilOpDesc {
    pub(crate) stencil_fail_op: u32,
    pub(crate) stencil_depth_fail_op: u32,
    pub(crate) stencil_pass_op: u32,
    pub(crate) stencil_func: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameOutputMergerState {
    #[serde(default)]
    pub(crate) render_targets: Vec<MapFrameOutputMergerTexture>,
    #[serde(default)]
    pub(crate) depth_stencil: Option<MapFrameOutputMergerTexture>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameOutputMergerTexture {
    #[serde(default)]
    pub(crate) slot: Option<u32>,
    pub(crate) view_pointer: String,
    pub(crate) resource_pointer: String,
    pub(crate) desc: MapFrameTexture2DDesc,
    #[serde(default)]
    pub(crate) view_desc: Option<MapFrameOutputMergerViewDesc>,
    #[serde(default)]
    pub(crate) pre_draw_payload: Option<MapFrameOutputMergerTexturePayload>,
    #[serde(default)]
    pub(crate) post_pass_payload: Option<MapFrameOutputMergerTexturePayload>,
    #[serde(default)]
    pub(crate) last_render_target_clear: Option<MapFrameRenderTargetClear>,
    #[serde(default)]
    pub(crate) last_depth_stencil_clear: Option<MapFrameDepthStencilClear>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameOutputMergerViewDesc {
    pub(crate) format: u32,
    pub(crate) view_dimension: u32,
    pub(crate) mip_slice: u32,
    pub(crate) first_array_slice: u32,
    pub(crate) array_size: u32,
    pub(crate) flags: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameOutputMergerTexturePayload {
    pub(crate) file: String,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) payload_format: String,
    #[serde(default)]
    pub(crate) subresources: Vec<MapFrameTextureSubresource>,
    pub(crate) provenance: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameRenderTargetClear {
    pub(crate) sequence: u64,
    pub(crate) color: [f32; 4],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameDepthStencilClear {
    pub(crate) sequence: u64,
    pub(crate) clear_flags: u32,
    pub(crate) depth: f32,
    pub(crate) stencil: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameTexture2DDesc {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mip_levels: u32,
    pub(crate) array_size: u32,
    pub(crate) format: u32,
    pub(crate) sample_count: u32,
    pub(crate) sample_quality: u32,
    pub(crate) usage: u32,
    pub(crate) bind_flags: u32,
    pub(crate) cpu_access_flags: u32,
    pub(crate) misc_flags: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameShader {
    pub(crate) stage: String,
    pub(crate) pointer: String,
    #[serde(default)]
    pub(crate) bytecode_file: String,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
    #[serde(default)]
    pub(crate) reflection_resources: BTreeMap<String, u32>,
    #[serde(default)]
    pub(crate) input_parameters: Vec<MapFrameShaderInputParameter>,
    #[serde(default)]
    pub(crate) constant_buffers: BTreeMap<String, u32>,
    #[serde(default)]
    pub(crate) source_evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameShaderInputParameter {
    pub(crate) semantic_name: String,
    pub(crate) semantic_index: u32,
    pub(crate) register: u32,
    pub(crate) component_type: u32,
    pub(crate) mask: u8,
    pub(crate) read_write_mask: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameInputLayout {
    pub(crate) pointer: String,
    pub(crate) elements: Vec<RuntimeInputElementMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameBufferBinding {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    #[serde(default)]
    pub(crate) declared_name: Option<String>,
    #[serde(default)]
    pub(crate) file: Option<String>,
    #[serde(default)]
    pub(crate) byte_len: usize,
    #[serde(default)]
    pub(crate) fnv1a64: Option<String>,
    #[serde(default)]
    pub(crate) stride: Option<u32>,
    #[serde(default)]
    pub(crate) offset: Option<u32>,
    #[serde(default)]
    pub(crate) dxgi_format: Option<u32>,
    #[serde(default)]
    pub(crate) bind_flags: u32,
    #[serde(default)]
    pub(crate) usage: u32,
    #[serde(default)]
    pub(crate) cpu_access_flags: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameIndexBufferBinding {
    #[serde(default)]
    pub(crate) file: Option<String>,
    pub(crate) byte_len: usize,
    #[serde(default)]
    pub(crate) fnv1a64: Option<String>,
    pub(crate) dxgi_format: u32,
    pub(crate) offset: u32,
    pub(crate) bind_flags: u32,
    pub(crate) usage: u32,
    pub(crate) cpu_access_flags: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameTextureBinding {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    #[serde(default)]
    pub(crate) declared_name: Option<String>,
    #[serde(default)]
    pub(crate) is_null: bool,
    #[serde(default)]
    pub(crate) view_desc: Option<MapFrameShaderResourceViewDesc>,
    #[serde(default)]
    pub(crate) file: Option<String>,
    #[serde(default)]
    pub(crate) byte_len: usize,
    #[serde(default)]
    pub(crate) fnv1a64: Option<String>,
    pub(crate) dxgi_format: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) mip_count: u32,
    #[serde(default)]
    pub(crate) array_size: u32,
    #[serde(default)]
    pub(crate) sample_count: u32,
    #[serde(default)]
    pub(crate) sample_quality: u32,
    #[serde(default)]
    pub(crate) payload_format: Option<String>,
    #[serde(default)]
    pub(crate) subresources: Vec<MapFrameTextureSubresource>,
    pub(crate) bind_flags: u32,
    pub(crate) usage: u32,
    #[serde(default)]
    pub(crate) cpu_access_flags: u32,
    #[serde(default)]
    pub(crate) misc_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MapFrameShaderResourceViewDesc {
    pub(crate) format: u32,
    pub(crate) view_dimension: u32,
    pub(crate) most_detailed_mip: u32,
    pub(crate) mip_levels: u32,
    pub(crate) first_array_slice: u32,
    pub(crate) array_size: u32,
    pub(crate) num_cubes: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameTextureSubresource {
    pub(crate) mip_slice: u32,
    pub(crate) array_slice: u32,
    pub(crate) file_offset: usize,
    pub(crate) byte_len: usize,
    pub(crate) row_pitch: u32,
    pub(crate) depth_pitch: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameSamplerBinding {
    pub(crate) stage: String,
    pub(crate) slot: u32,
    pub(crate) filter: u32,
    pub(crate) address_u: u32,
    pub(crate) address_v: u32,
    pub(crate) address_w: u32,
    #[serde(default)]
    pub(crate) mip_lod_bias: f32,
    #[serde(default)]
    pub(crate) max_anisotropy: u32,
    #[serde(default)]
    pub(crate) comparison_func: u32,
    #[serde(default)]
    pub(crate) border_color: [f32; 4],
    #[serde(default)]
    pub(crate) min_lod: f32,
    pub(crate) max_lod: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapFrameFileEvidence {
    pub(crate) file: String,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) provenance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resolved_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) effective_vfs_verified: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainIdMapRuntimeMeta {
    pub(crate) file: String,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) format: String,
    pub(crate) channel_order: Vec<String>,
    pub(crate) sampler: TerrainRuntimeSampler,
    pub(crate) source: RuntimeInputSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerrainRuntimeSampler {
    pub(crate) mag_filter: String,
    pub(crate) min_filter: String,
    pub(crate) mip_filter: String,
    pub(crate) address_u: String,
    pub(crate) address_v: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct MapPow2RuntimeMeta {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) source: RuntimeInputSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RuntimeTextureRuntimeMeta {
    pub(crate) file: String,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) format: String,
    pub(crate) sampler: TerrainRuntimeSampler,
    pub(crate) source: RuntimeInputSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RuntimeConstantBufferRuntimeMeta {
    pub(crate) file: String,
    pub(crate) byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) format: String,
    pub(crate) bind_point: u32,
    pub(crate) layout_source: String,
    pub(crate) layout_byte_len: usize,
    pub(crate) required_offsets: BTreeMap<String, usize>,
    pub(crate) source: RuntimeInputSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RuntimeVertexBufferRuntimeMeta {
    pub(crate) file: String,
    pub(crate) source_byte_width: usize,
    pub(crate) captured_byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) format: String,
    pub(crate) stride: u32,
    pub(crate) offset: u32,
    pub(crate) minimum_stride_source: String,
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    pub(crate) terrain_id_srv_slot: u32,
    pub(crate) source: RuntimeInputSource,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RuntimeInputLayoutRuntimeMeta {
    pub(crate) element_count: usize,
    pub(crate) elements: Vec<RuntimeInputElementMeta>,
    pub(crate) format: String,
    pub(crate) vertex_stride: u32,
    pub(crate) vertex_offset: u32,
    pub(crate) shader_input_source: String,
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    pub(crate) terrain_id_srv_slot: u32,
    pub(crate) source: RuntimeInputSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RuntimeInputElementMeta {
    pub(crate) semantic_name: String,
    pub(crate) semantic_index: u32,
    pub(crate) format: u32,
    pub(crate) input_slot: u32,
    pub(crate) aligned_byte_offset: u32,
    pub(crate) input_slot_class: u32,
    pub(crate) instance_data_step_rate: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RuntimeInputSource {
    pub(crate) provenance: String,
    pub(crate) extraction: String,
}

#[derive(Clone)]
pub(crate) struct RgbaSourceSpec {
    pub(crate) source: String,
    pub(crate) output: String,
    pub(crate) path: PathBuf,
    pub(crate) role: String,
    pub(crate) shader_samplers: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct RawMaterialSpec {
    pub(crate) source: String,
    pub(crate) output: String,
    pub(crate) path: PathBuf,
    pub(crate) role: String,
    pub(crate) shader_samplers: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct DdsInfo {
    pub(crate) dimensions: Dimensions,
    pub(crate) format: String,
    pub(crate) mip_count: u32,
}

pub(crate) struct RgbxImage {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<u32>,
}

pub(crate) struct RgbaImage {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rgba32fImage {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<[f32; 4]>,
}

pub(crate) struct RgbaMipChain {
    pub(crate) levels: Vec<RgbaImage>,
    pub(crate) color_space: RgbaColorSpace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RgbaColorSpace {
    Linear,
    Srgb,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShaderConstantBufferLayout {
    pub(crate) byte_len: usize,
    pub(crate) fields: Vec<ShaderConstantBufferField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShaderConstantBufferField {
    pub(crate) type_name: String,
    pub(crate) name: String,
    pub(crate) offset: usize,
    pub(crate) byte_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vec3 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SampledRgba {
    pub(crate) rgb: u32,
    pub(crate) alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SampledLinearRgba {
    pub(crate) rgb: Vec3,
    pub(crate) alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainMaterialSample {
    pub(crate) diffuse: Vec3,
    pub(crate) normal: Vec3,
    pub(crate) glossiness: f32,
    pub(crate) specular: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainSceneCb0Decoded {
    pub(crate) view_projection_matrix: [[f32; 4]; 4],
    pub(crate) v_gb_cam_dist_override_gb_outline_cutoff: [f32; 4],
    pub(crate) v_virtual_sun_pos: Vec3,
    pub(crate) v_virtual_moon_pos: Vec3,
    pub(crate) v_second_virtual_sun_pos: Vec3,
    pub(crate) v_second_virtual_moon_pos: Vec3,
    pub(crate) v_cam_pos: Vec3,
    pub(crate) hdr_range: f32,
    pub(crate) v_global_time: f32,
    pub(crate) v_fow_opacity_fow_time_snow_mud_fade_max_game_speed: [f32; 4],
    pub(crate) day_night_hour_sun_dir: [f32; 4],
    pub(crate) ambient_pos_x: Vec3,
    pub(crate) shadow_fade_factor: f32,
    pub(crate) ambient_neg_x: Vec3,
    pub(crate) fow_fade_factor: f32,
    pub(crate) ambient_pos_y: Vec3,
    pub(crate) ambient_neg_y: Vec3,
    pub(crate) neg_fog_multiplier: f32,
    pub(crate) ambient_pos_z: Vec3,
    pub(crate) ambient_neg_z: Vec3,
    pub(crate) sun_diffuse_intensity: [f32; 4],
    pub(crate) moon_diffuse_intensity: [f32; 4],
    pub(crate) gb_texture_height: f32,
    pub(crate) sun_specular_intensity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MapFrameTerrainShaderConstants {
    pub(crate) map_size_x: f32,
    pub(crate) map_size_y: f32,
    pub(crate) map_pow2_x: f32,
    pub(crate) map_pow2_y: f32,
    pub(crate) fow_pow2_x: f32,
    pub(crate) fow_pow2_y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainPointLightCb2Decoded {
    pub(crate) grid_start_inv_cell_size: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TerrainShadowMapState {
    NullSrv,
}

pub(crate) struct TerrainPostDiffuseResources {
    pub(crate) snow_mud_data: RgbaImage,
    pub(crate) snow_texture: RgbaMipChain,
    pub(crate) city_lights_snow_noise: RgbaMipChain,
    pub(crate) mud_diffuse_gloss: RgbaMipChain,
    pub(crate) mud_normal_spec: RgbaMipChain,
    pub(crate) province_secondary_color_map: RgbaMipChain,
    pub(crate) gradient_border_channel1: RgbaMipChain,
    pub(crate) gradient_border_channel2: RgbaMipChain,
    pub(crate) light_index_map: RgbaImage,
    pub(crate) light_data_map: Rgba32fImage,
    pub(crate) shadow_map: TerrainShadowMapState,
    pub(crate) scene_cb0: TerrainSceneCb0Decoded,
    pub(crate) point_light_cb2: TerrainPointLightCb2Decoded,
    pub(crate) constants: MapFrameTerrainShaderConstants,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainPostDiffuseRuntimeSample {
    pub(crate) mud_snow: [f32; 4],
    pub(crate) point_light_indices: [u8; 4],
    pub(crate) point_light_count: usize,
    pub(crate) shadow_term: f32,
    pub(crate) surface: TerrainSurfaceSample,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainSecondaryMaskSample {
    pub(crate) surface: TerrainSurfaceSample,
    pub(crate) bloom_alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainSurfaceSample {
    pub(crate) diffuse: Vec3,
    pub(crate) normal: Vec3,
    pub(crate) glossiness: f32,
    pub(crate) specular: f32,
    pub(crate) snow_alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainLightingProperties {
    pub(crate) world_space_pos: Vec3,
    pub(crate) to_camera_dir: Vec3,
    pub(crate) normal: Vec3,
    pub(crate) diffuse: Vec3,
    pub(crate) specular_color: Vec3,
    pub(crate) glossiness: f32,
    pub(crate) non_linear_glossiness: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainPointLight {
    pub(crate) position: Vec3,
    pub(crate) radius: f32,
    pub(crate) color: Vec3,
    pub(crate) falloff: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainPixelShaderInput {
    pub(crate) uv: Vec2,
    pub(crate) uv2: Vec2,
    pub(crate) prepos: Vec3,
    pub(crate) v_shadow_proj: Vec4,
    pub(crate) v_screen_coord: Vec4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainPixelShaderDerivatives {
    pub(crate) prepos_dx: Vec3,
    pub(crate) prepos_dy: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainDrawCb1Decoded {
    pub(crate) shadow_map_texture_matrix: [[f32; 4]; 4],
    pub(crate) quad_offset_scale_is_detail: [f32; 4],
    pub(crate) v_snap: [f32; 4],
    pub(crate) v_border_lookup_height_scale_use_multisample_season_lerp: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainVertexShaderInput {
    pub(crate) position: [i16; 4],
    pub(crate) height: [i16; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vec2 {
    pub(crate) x: f32,
    pub(crate) y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vec4 {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) w: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TerrainVertexShaderOutput {
    pub(crate) input: TerrainVertexShaderInput,
    pub(crate) position: Vec4,
    pub(crate) uv: Vec2,
    pub(crate) uv2: Vec2,
    pub(crate) prepos: Vec3,
    pub(crate) v_shadow_proj: Vec4,
    pub(crate) v_screen_coord: Vec4,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg(test)]
pub(crate) struct MapFrameTerrainReplaySummary {
    pub(crate) draw_index: u32,
    pub(crate) index_count: usize,
    pub(crate) triangle_count: usize,
    pub(crate) unique_vertex_count: usize,
    pub(crate) uv_min: Vec2,
    pub(crate) uv_max: Vec2,
    pub(crate) uv2_min: Vec2,
    pub(crate) uv2_max: Vec2,
    pub(crate) prepos_min: Vec3,
    pub(crate) prepos_max: Vec3,
}

pub(crate) struct MapFrameTerrainRenderSummary {
    pub(crate) draw_count: usize,
    pub(crate) index_count: usize,
    pub(crate) triangle_count: usize,
    pub(crate) covered_pixel_count: usize,
    pub(crate) output_width: usize,
    pub(crate) output_height: usize,
    pub(crate) snow_mud_data_loaded: bool,
}

pub struct IndexImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct TerrainRendererInputs {
    pub(crate) terrain_runtime: Option<PathBuf>,
    pub(crate) terrain_id_map: Option<PathBuf>,
    pub(crate) terrain_diffuse_atlas: PathBuf,
    pub(crate) terrain_normal_atlas: PathBuf,
    pub(crate) terrain_diffuse_atlas_mips: PathBuf,
    pub(crate) terrain_normal_atlas_mips: PathBuf,
    pub(crate) terrain_color_tint: PathBuf,
    pub(crate) height_normal: PathBuf,
    pub(crate) constants_source: PathBuf,
    pub(crate) pdxmap_shader_source: PathBuf,
    pub(crate) pdxmap_include_source: PathBuf,
    pub(crate) standard_functions_source: PathBuf,
    pub(crate) shadow_include_source: PathBuf,
    pub(crate) fow_include_source: PathBuf,
    pub(crate) runtime: Option<TerrainRuntimeInputs>,
}

impl TerrainRendererInputs {
    pub(crate) fn validated_input_count(&self) -> usize {
        let required_file_count = [
            &self.terrain_diffuse_atlas,
            &self.terrain_normal_atlas,
            &self.terrain_diffuse_atlas_mips,
            &self.terrain_normal_atlas_mips,
            &self.terrain_color_tint,
            &self.height_normal,
            &self.constants_source,
            &self.pdxmap_shader_source,
            &self.pdxmap_include_source,
            &self.standard_functions_source,
            &self.shadow_include_source,
            &self.fow_include_source,
        ]
        .len();
        let legacy_file_count =
            self.terrain_runtime.iter().count() + self.terrain_id_map.iter().count();
        let runtime_input_count = self
            .runtime
            .as_ref()
            .map(|runtime| {
                runtime.runtime_textures.len()
                    + runtime.runtime_constant_buffers.len()
                    + runtime.runtime_vertex_buffers.len()
                    + runtime.runtime_input_layouts.len()
            })
            .unwrap_or(0);
        required_file_count + legacy_file_count + runtime_input_count
    }
}

#[derive(Clone)]
pub(crate) struct ImportTerrainRuntimeOptions {
    pub(crate) bundle: PathBuf,
    pub(crate) terrain_id_map_rgba: PathBuf,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) map_pow2_x: f32,
    pub(crate) map_pow2_y: f32,
    pub(crate) map_pow2_extraction: String,
    pub(crate) fow_pow2_x: f32,
    pub(crate) fow_pow2_y: f32,
    pub(crate) fow_pow2_extraction: String,
    pub(crate) provenance: String,
    pub(crate) force: bool,
}

#[derive(Clone)]
pub(crate) struct ImportRuntimeTextureOptions {
    pub(crate) bundle: PathBuf,
    pub(crate) source_rgba: PathBuf,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) shader_symbol: String,
    pub(crate) provenance: String,
    pub(crate) force: bool,
}

#[derive(Clone)]
pub(crate) struct ImportRuntimeConstantBufferOptions {
    pub(crate) bundle: PathBuf,
    pub(crate) source_bin: PathBuf,
    pub(crate) shader_symbol: String,
    pub(crate) provenance: String,
    pub(crate) force: bool,
}

#[derive(Clone)]
pub(crate) struct ImportRuntimeVertexBufferOptions {
    pub(crate) bundle: PathBuf,
    pub(crate) capture_json: PathBuf,
    pub(crate) shader_symbol: String,
    pub(crate) provenance: String,
    pub(crate) force: bool,
}

#[derive(Clone)]
pub(crate) struct ImportRuntimeInputLayoutOptions {
    pub(crate) bundle: PathBuf,
    pub(crate) capture_json: PathBuf,
    pub(crate) shader_symbol: String,
    pub(crate) provenance: String,
    pub(crate) force: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainVertexBufferCaptureFile {
    pub(crate) schema: String,
    pub(crate) symbol: String,
    pub(crate) file: String,
    pub(crate) source_byte_width: usize,
    pub(crate) captured_byte_len: usize,
    pub(crate) fnv1a64: String,
    pub(crate) stride: u32,
    pub(crate) offset: u32,
    pub(crate) minimum_stride_source: String,
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    pub(crate) terrain_id_srv_slot: u32,
    pub(crate) source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TerrainInputLayoutCaptureFile {
    pub(crate) schema: String,
    pub(crate) symbol: String,
    pub(crate) element_count: usize,
    pub(crate) elements: Vec<RuntimeInputElementMeta>,
    pub(crate) vertex_stride: u32,
    pub(crate) vertex_offset: u32,
    pub(crate) shader_input_source: String,
    pub(crate) draw_call: String,
    pub(crate) draw_count: u32,
    pub(crate) draw_start: u32,
    pub(crate) base_vertex_location: i32,
    pub(crate) terrain_id_srv_slot: u32,
    pub(crate) source: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimePow2Source {
    pub(crate) map_pow2_x: f32,
    pub(crate) map_pow2_y: f32,
    pub(crate) map_pow2_extraction: String,
    pub(crate) fow_pow2_x: f32,
    pub(crate) fow_pow2_y: f32,
    pub(crate) fow_pow2_extraction: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ShaderCacheMapConstants {
    pub(crate) cache_file: String,
    pub(crate) map_size_x: f32,
    pub(crate) map_size_y: f32,
    pub(crate) map_pow2_x: f32,
    pub(crate) map_pow2_y: f32,
    pub(crate) fow_pow2_x: f32,
    pub(crate) fow_pow2_y: f32,
    pub(crate) terrain_id_srv_slot: u32,
}

pub(crate) struct ProductionTerrainRenderSummary {
    pub(crate) validated_input_count: usize,
    pub(crate) terrain_id_map_width: usize,
    pub(crate) terrain_id_map_height: usize,
    pub(crate) map_pow2_x: f32,
    pub(crate) map_pow2_y: f32,
    pub(crate) fow_pow2_x: f32,
    pub(crate) fow_pow2_y: f32,
}

pub(crate) struct TerrainIndexBlend {
    pub(crate) index_u: [f32; 4],
    pub(crate) index_v: [f32; 4],
    pub(crate) all_same: bool,
}
