use std::error::Error;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

mod bloom;
mod border_capture_evidence;
mod border_law;
mod border_pass;
mod border_texture_law;
mod border_world_synthesis;
mod building_files;
mod bundle_gen;
mod camera;
mod cli;
mod color_lut;
mod constants;
mod cpu_terrain;
mod d3d11_device;
mod d3d11_replay;
mod frame_bundle;
mod game_data;
mod image_load;
mod light_law;
mod luminance_law;
mod map_furniture;
mod map_label_capture;
mod map_label_law;
mod map_static_data;
mod map_text;
mod mod_resolver;
mod pass_graph;
mod pdx_mesh;
mod political_color_synthesis;
mod reflection_cube_law;
mod restore_evidence;
mod river_mesh_synthesis;
mod river_world_synthesis;
mod seasons;
mod shader_compile;
mod shader_meta;
mod snow_law;
mod terrain_chunk_library;
mod terrain_chunk_synthesis;
mod terrain_id_map_synthesis;
mod terrain_lod_law;
mod terrain_mesh_law;
#[cfg(test)]
mod test_support;
mod texture_sampling;
mod tree_files;
mod tree_instance_synthesis;
mod tree_second_subdraw;
mod types;
mod underwater_pass;
mod util;
mod water_law;
mod weather_law;

pub(crate) use border_pass::*;
pub(crate) use bundle_gen::*;
pub(crate) use camera::*;
pub(crate) use cli::*;
pub(crate) use constants::*;
pub(crate) use cpu_terrain::*;
pub(crate) use d3d11_device::*;
pub(crate) use d3d11_replay::*;
pub(crate) use frame_bundle::*;
pub(crate) use game_data::*;
pub(crate) use image_load::*;
pub(crate) use mod_resolver::*;
pub(crate) use pass_graph::*;
pub(crate) use reflection_cube_law::*;
pub(crate) use river_mesh_synthesis::*;
pub(crate) use shader_meta::*;
pub(crate) use terrain_chunk_library::*;
pub(crate) use terrain_chunk_synthesis::*;
pub(crate) use terrain_id_map_synthesis::*;
pub(crate) use terrain_mesh_law::build_terrain_chunk_synthetic_payloads;
pub(crate) use texture_sampling::*;
pub(crate) use tree_instance_synthesis::*;
pub(crate) use types::*;
pub(crate) use underwater_pass::*;
pub(crate) use util::*;

pub use bloom::{
    f32_to_half_float, half_float_to_f32, parse_define_number, resolve_bloom_params,
    resolve_bloom_params_for_volume, run_bloom_chain, BloomBlurKernel, BloomChainOutput,
    BloomImageRgba, BloomParams, BLOOM_DEFAULT_VOLUME, BLOOM_LUMINANCE_VECTOR,
};
pub use border_capture_evidence::{
    classify_captured_border_geometry, CapturedBorderGeometryKind, CapturedBorderIaLayout,
    DIAG23B_BORDERS_PASS_LAYOUT, SYNTHESIZED_BORDER_LAYOUT_POSITION_FORMAT,
    SYNTHESIZED_BORDER_LAYOUT_TEXCOORD_FORMAT, SYNTHESIZED_BORDER_VERTEX_STRIDE,
};
pub use border_law::{
    border_diffuse_source, border_fade_windows_from_graphics_defines,
    border_fade_windows_from_resolver, border_transparency, defines as border_fade_defines,
    BorderFadeWindow, BorderFadeWindows, BORDER_PASS_DRAW_ORDER_EVIDENCE,
};
pub use border_pass::{BorderFilesPassStats, BORDER_PASS_EFFECT_NAME, BORDER_PASS_SHADER_SOURCE};
pub use border_texture_law::{
    all_border_texture_profiles_from_resolver, border_texture_profile_from_resolver,
    BorderTextureProfile, BORDER_TILE, INK_ALPHA_THRESHOLD,
};
pub use border_world_synthesis::{
    border_material_sources_from_resolver, border_source_evidence_from_resolver,
    synthesize_border_world_mesh_from_resolver, BorderChain, BorderClassIndexPartition,
    BorderClassIndexRange, BorderMaterial, BorderMaterialSource, BorderRank,
    BorderSourceEvidenceReport, BorderSourceExcerpt, BorderWaterClass, BorderWorldMesh,
    BorderWorldSynthesisStats, BorderWorldVertex, BORDER_CONSTANTS_SOURCE,
    BORDER_DEFAULT_MAP_SOURCE, BORDER_DEFINITION_CSV_SOURCE, BORDER_HEIGHTMAP_BMP_SOURCE,
    BORDER_HISTORY_STATES_DIR, BORDER_PROVINCES_BMP_SOURCE, BORDER_SHADER_SOURCE,
    BORDER_VERTEX_STRIDE_BYTES,
};
pub use building_files::{
    building_entity_bindings_from_resolver, building_file_derived_layer_from_resolver,
    building_mesh_vertex_buffer_from_model, building_pdxmesh_bindings_from_resolver,
    building_preview_instances, parse_building_entity_bindings, parse_building_pdxmesh_bindings,
    resolve_show_on_map_building_bindings, BuildingBindingCoverageRow, BuildingEntityBinding,
    BuildingFileDerivedLayer, BuildingFileDerivedLayerStats, BuildingMeshVertexBuffer,
    BuildingPdxmeshBinding, BuildingPreviewInstance, BuildingShaderCBufferMapping,
    BuildingShaderPreparation, ResolvedBuildingEntity, BUILDINGS_ENTITY_ASSET_SOURCE,
    BUILDINGS_GFX_SOURCE, BUILDING_MESH_VERTEX_STRIDE_BYTES, BUILDING_SHADER_EFFECT,
    BUILDING_SHADER_SOURCE,
};
pub use camera::{
    hoi4_map_camera_defines_from_bundle, map_wrap_offsets, Hoi4MapCameraDefines,
    ProductionMapCamera, ProductionMapCameraProjection,
};
pub use cli::run_cli;
pub use color_lut::{
    color_correction_lut_from_resolver, lut_blend1, lut_blend2, parse_color_correction_tga,
    scan_color_correction_luts_from_resolver, ColorCorrectionLut, ColorLutProbeRow,
    COLOR_CORRECTION_DIR,
};
pub use d3d11_replay::{
    accept_terrain_d3d11, accept_terrain_d3d11_with_resolver, render_production_map_d3d11,
    render_production_map_d3d11_with_resolver, render_terrain_d3d11, render_terrain_d3d11_bundle,
    render_terrain_d3d11_with_resolver, smoke_terrain_chunk_synthesis,
    validate_terrain_chunk_synthesis, validate_terrain_d3d11_bundle,
    validate_terrain_d3d11_bundle_with_resolver, NativeD3d11PreparedProductionMap,
    NativeD3d11PreparedSynthesizedProductionMap, NativeD3d11PreparedTerrainBundle,
    NativeD3d11PreparedTerrainChunkSynthesis, NativeD3d11ReferenceComparison,
    NativeD3d11RenderOptions, NativeD3d11ReplayReadback, NativeD3d11ReplayRequest,
    NativeD3d11ReplayResult, NativeD3d11ReplaySummary, NativeD3d11TerrainBundle,
    NativeD3d11TerrainRenderer, ProductionMapD3d11ReplayRequest, TerrainD3d11AcceptanceReport,
    TerrainD3d11AcceptanceRequest, UnderwaterValidationRender,
};
pub use frame_bundle::{
    validate_borders_map_frame_bundle, validate_rivers_map_frame_bundle,
    validate_trees_map_frame_bundle, validate_water_map_frame_bundle,
};
pub use light_law::{defines as light_defines, offsets as light_offsets};
pub use light_law::{
    diffuse_color_rgb_from_hsv_define, night_yzw_residual, render_sun_dir_yzw,
    validate_lighting_law_against_cb0, LightingLawResidual, SceneLighting,
};
pub use luminance_law::{
    f16_bits_to_f32 as luminance_f16_bits_to_f32, f32_to_f16_bits as luminance_f32_to_f16_bits,
    scene_average_luminance, scene_average_luminance_r16g16b16a16_float, AverageLuminanceReduction,
    LuminanceClampBounds, LOG_LUMINANCE_EPSILON, LUMINANCE_VECTOR,
};
pub use map_furniture::{
    associate_naval_spawns_to_provinces, base_icon_defines_from_resolver, base_icon_frame,
    base_icon_layer_from_resolver, building_locator_pixel, capital_states_from_resolver,
    compute_base_icon_markers, compute_furniture_markers, cosmetic_tags_from_resolver,
    cosmetic_tags_from_resolver_with_dlc, country_colors_from_game_resolver,
    furniture_layer_from_resolver, furniture_static_data_from_resolver,
    heightmap_grid_from_resolver, load_vp_label_font, load_vp_map_text_font, load_vp_strip_rgba,
    parse_base_icon_defines,
    parse_bmfont, parse_state_building_levels, parse_vp_icon_defines, ruling_parties_from_resolver,
    state_building_levels_from_resolver, vp_icon_defines_from_resolver, vp_strip_frame_index,
    vp_strip_texture_path, BaseIconDefines, BaseIconInputs, BaseIconKind, BaseIconLayer,
    BaseIconMarker, BmFont, BmFontGlyph, FurnitureLawInputs, FurnitureLayer, FurnitureMarker,
    FurnitureMarkerKind, FurnitureStaticData, HeightmapGrid, RgbaFontAtlas, StateBuildingLevels,
    VpIconDefines, AIRBASE_ICON_DISTANCE_CUTOFF_KEY, AIR_BASE_BUILDING, AIR_BASE_SPRITE_FRAMES,
    AIR_BASE_SPRITE_NAME, AIR_BASE_SPRITE_TEXTURE, AIR_BASE_STATE_MAX_LEVEL, CAPITAL_ICON_CUTOFF_KEY,
    CORE_GFX_SOURCE, HOI_20BS_ATLAS_SOURCE, HOI_20BS_FNT_SOURCE, MAPICONS_GFX_SOURCE,
    MAPICONS_GUI_SOURCE, MAPICON_CONTAINER_HEIGHT, MAPICON_CONTAINER_WIDTH, MAPICON_ICON_OFFSET_X,
    MAPICON_ICON_OFFSET_Y, MAPICON_TEXT_MAX_HEIGHT, MAPICON_TEXT_MAX_WIDTH, MAPICON_TEXT_OFFSET_X,
    MAPICON_TEXT_OFFSET_Y, NAVALBASE_ICON_DISTANCE_CUTOFF_KEY, NAVAL_BASE_BUILDING,
    NAVAL_BASE_PROVINCE_MAX_LEVEL, NAVAL_BASE_SPAWN_LOCATOR, NAVAL_BASE_SPRITE_FRAMES,
    NAVAL_BASE_SPRITE_NAME, NAVAL_BASE_SPRITE_TEXTURE, NAVAL_SPAWN_LAND_SNAP_RADIUS,
    VICTORY_POINTS_LOC_PREFIX, VICTORY_POINT_LEVELS_KEY, VICTORY_POINT_MAP_ICON_AFTER_KEY,
    VP_LABEL_FONT_NAME, VP_MAP_TEXT_ATLAS_SOURCE, VP_MAP_TEXT_FNT_SOURCE, VP_MAP_TEXT_FONT_NAME,
    VP_MAP_TEXT_SCALE, VP_STRIP_COLOR_NEUTRAL_GRAY, VP_STRIP_COLOR_SLOTS, VP_STRIP_FRAME_COUNT,
    VP_STRIP_SPRITE_NAME, VP_STRIP_SPRITE_TEXTURE,
};
pub use map_label_capture::{
    decode_label, decode_vertices, fit_baseline, glyph_quads, match_glyph, parse_fnt_chars,
    BaselineFit, DecodeError, DecodedLabel, FntChar, GlyphQuad, MapnameVertex,
    MAPNAME_VERTEX_STRIDE, POSITION_OFFSET, TEXCOORD_OFFSET, VERTS_PER_GLYPH,
};
pub use map_label_law::{
    compute_country_label_placements, compute_country_label_placements_with_text,
    label_draw_visible, principal_axis, CountryLabelPlacement, EngineCurve, GlyphScale,
    LabelLawInputs, LabelTextMetrics, ProvinceKind, ENGINE_TITLE_CAP_COEF, ENGINE_TITLE_FIT_COEF,
    LABEL_AB_HYSTERESIS, LABEL_HEIGHT_BREADTH_K, LABEL_OFFSET_PX, LABEL_SAMPLE_STEP,
};
pub use map_static_data::{
    map_building_placements_from_game_dir, map_building_placements_from_resolver,
    map_building_placements_from_source, railway_segments_from_game_dir,
    railway_segments_from_resolver, railway_segments_from_source,
    show_on_map_buildings_from_game_dir, show_on_map_buildings_from_resolver,
    show_on_map_buildings_from_source, supply_nodes_from_game_dir, supply_nodes_from_resolver,
    supply_nodes_from_source, unit_stack_placements_from_game_dir,
    unit_stack_placements_from_resolver, unit_stack_placements_from_source,
    weather_positions_from_game_dir, weather_positions_from_resolver,
    weather_positions_from_source, MapBuildingPlacement, RailwaySegment, ShowOnMapBuilding,
    SupplyNode, UnitStackPlacement, WeatherPosition,
};
pub use map_text::{
    draw_country_names_cutoff_from_resolver, localisation_table_from_resolver,
    merge_localisation_source, DRAW_COUNTRY_NAMES_CUTOFF_KEY, LOCALISATION_ROOT,
};
pub use mod_resolver::{
    sensitive_file_overrides, GameFileResolver, GameFileResolverOptions, GameFileResolverStats,
    SensitiveFileOverrideReport,
};
pub use pass_graph::{
    ProductionMapPassGraph, ProductionMapPassKind, ProductionMapPassPlan, ProductionMapPassStatus,
    ProductionMapTargetPlan,
};
pub use pdx_mesh::{
    extract_pdx_anim, extract_pdx_model, parse_pdx_asset, pdx_anim_from_resolver,
    pdx_model_from_game_dir, pdx_model_from_resolver, pdx_model_to_obj,
    scan_pdx_assets_from_resolver, validate_pdx_model, PdxAnimData, PdxAnimJoint,
    PdxAssetScanReport, PdxAssetTree, PdxJoint, PdxLocator, PdxMaterial, PdxMeshData,
    PdxMeshValidationRow, PdxModel, PdxNode, PdxProperty, PdxShape, PdxSkin, PdxValues,
    PDX_ASSET_BINARY_MAGIC, PDX_ASSET_SUPPORTED_VERSION, PDX_ASSET_TEXT_MAGIC, PDX_MODELS_ROOT,
};
pub use political_color_synthesis::{
    compare_political_colors_to_capture, dump_political_color_comparison,
    political_color_composition_law, synthesize_political_colors, PoliticalColorComparisonReport,
    PoliticalColorCompositionLaw, PoliticalColorDumpPaths, PoliticalColorMismatchSample,
    PoliticalColorSamplerDescriptor, PoliticalColorSynthesis, PoliticalColorSynthesisSource,
    PoliticalColorTextureDescriptor, PROVINCE_COLOR_MAP_DXGI_FORMAT, PROVINCE_COLOR_MAP_HEIGHT,
    PROVINCE_COLOR_MAP_WIDTH, PROVINCE_SECONDARY_COLOR_MAP_SHADER_SLOT,
};
pub use restore_evidence::{
    load_restore_scene_bundle, load_restore_scene_bundle_session_dir,
    probe_restore_scene_evidence_session_dir, RestoreSceneAverageLuminance, RestoreSceneBundle,
    RestoreSceneColorCube, RestoreSceneEvidence, RestoreSceneEvidenceProbeReport,
    RestoreSceneTexturePayloadSummary, RestoreSceneUniforms, RESTORE_PASS_BUNDLE_FILE,
};
pub use river_mesh_synthesis::{
    MapLayerSynthesisInputs, RiverCaptureDecodeReport, RiverCaptureStation, RiverMeshSynthesis,
    RiverMeshSynthesisStats,
};
pub use river_world_synthesis::{
    compare_river_world_mesh_to_capture, heightmap_bmp_from_resolver, load_captured_river_world,
    river_world_width_for_class, rivers_bmp_from_resolver, synthesize_river_world_mesh,
    synthesize_river_world_mesh_from_resolver, CapturedRiverWorld, RiverDistanceStats,
    RiverWidthClassRow, RiverWorldComparisonReport, RiverWorldMesh, RiverWorldStrip,
    RiverWorldSynthesisParams, RiverWorldVertex, HEIGHTMAP_BMP_SOURCE, RIVERS_BMP_SOURCE,
};
pub use seasons::{
    apply_tree_season_cb_override, seasons_table_from_game_dir, seasons_table_from_resolver,
    seasons_table_from_source, tree_season_cb_values, SeasonBlock, SeasonColorGrade, SeasonDate,
    SeasonDateRange, SeasonId, SeasonsTable, TreeSeasonId,
};
pub use shader_meta::PoliticalSceneCb0;

pub use shader_compile::{
    build_stage_hlsl, effect_file_from_resolver, parse_dxbc_reflection, parse_effect_file,
    scan_effect_files_from_resolver, DxbcCbuffer, DxbcCbufferVar, DxbcReflection, DxbcResource,
    DxbcSignatureParam, EffectDef, EffectElement, EffectFile, EffectParseScan, EffectSampler,
    EffectStage, ShaderCompileComparison, ShaderStage, DX11_PLATFORM_HEADER, MAP_EFFECT_CANDIDATES,
    MAP_EFFECT_COMPILE_ONLY, SHADER_FX_DIR,
};
#[cfg(windows)]
pub use shader_compile::{
    compare_effect_stage_to_captured, compare_map_effects_to_capture, d3d_compile_stage,
    COMPILE_FLAG_MATRIX,
};
pub use snow_law::{
    captured_snow_mud_stats, compare_floor_to_capture, decode_snow_mud_texel,
    encode_snow_mud_texel, parse_region_weather, region_weather_from_game_dir,
    region_weather_from_resolver, snow_mud_fade_from_cb0, synthesize_snow_floor_from_resolver,
    synthesize_snow_floor_texture, world_to_snow_mud_texel, CapturedSnowMudStats, RegionWeather,
    SnowDate, SnowFloorInputs, SnowFloorResidual, SnowFloorTexture, SnowMudAddressing,
    SnowMudChannels, WeatherPeriod, WeatherPoint, CB0_OFFSET_SNOW_MUD_FADE,
    CB0_OFFSET_SNOW_MUD_FADE_VECTOR, OBSERVED_MIN_SNOW_LEVELS, SNOW_DEFINITION_CSV_SOURCE,
    SNOW_MUD_SOURCE_DXGI_FORMAT, SNOW_MUD_TEXTURE_BYTE_LEN, SNOW_MUD_TEXTURE_HEIGHT,
    SNOW_MUD_TEXTURE_WIDTH, SNOW_PROVINCES_BMP_SOURCE, STRATEGIC_REGIONS_DIR, VANILLA_MAP_SIZE_X,
    VANILLA_MAP_SIZE_Y,
};
pub use terrain_chunk_library::NativeD3d11TerrainChunkLibrary;
pub use terrain_chunk_synthesis::{
    TerrainChunkSynthesisComparison, TerrainChunkSynthesisLodMapping, TerrainChunkSynthesisPlan,
    TerrainChunkSynthesisSmokeReport, TerrainChunkSynthesisTile,
    TerrainChunkSynthesisValidationReport, TerrainChunkSynthesisWorldRect,
};
pub use terrain_id_map_synthesis::{
    compare_terrain_id_map_to_capture, dump_terrain_id_map_comparison_bmps,
    synthesize_terrain_id_map, SynthesizedTerrainIdMap, TerrainIdMapChannelLaw,
    TerrainIdMapComparisonReport, TerrainIdMapDescriptor, TerrainIdMapMismatchBounds,
    TerrainIdMapMismatchSample, TerrainIdMapOrientationEvidence, TerrainIdMapPackingLaw,
    TerrainIdMapSamplerDescriptor, TerrainIdMapSynthesisSource,
};
pub use terrain_lod_law::{
    terrain_lod_map_validate_corpus, terrain_lod_map_validate_corpus_with_resolver,
    terrain_lod_stride_map, terrain_lod_tiles, terrain_vsnap_from_neighbour_lods,
    terrain_vsnap_law_validate_corpus, terrain_vsnap_law_validate_corpus_with_resolver,
    TerrainLodCorpusReport, TerrainLodStrideMap, TerrainLodTile, TerrainTileEdge,
    TerrainVsnapCorpusReport, TerrainVsnapEdgeCheck, TerrainVsnapEdgeClass,
    TERRAIN_LOD_BASE_TILE_SIZE, TERRAIN_LOD_EDGE_COUNT, TERRAIN_LOD_REFERENCE_HEIGHT,
    TERRAIN_LOD_THRESHOLDS_SQ,
};
pub use terrain_mesh_law::{
    generate_terrain_chunk_cb1, generate_terrain_chunk_index, generate_terrain_chunk_slot0,
    generate_terrain_chunk_slot1, generate_terrain_chunk_slot1_pool, terrain_chunk_pool_grid_dims,
    terrain_chunk_scale_for_tile_size, terrain_mesh_law_validate_corpus,
    terrain_mesh_law_validate_corpus_with_resolver, TerrainMeshLawChunkComparison,
    TerrainMeshLawCorpusReport, TerrainMeshLawLodTally, TERRAIN_CHUNK_CB1_BYTES,
    TERRAIN_CHUNK_GRID_SIDE_CELLS, TERRAIN_CHUNK_GRID_SIDE_VERTS, TERRAIN_CHUNK_HEIGHTMAP_SOURCE,
    TERRAIN_CHUNK_INDEX_BYTES, TERRAIN_CHUNK_INDEX_COUNT, TERRAIN_CHUNK_INDEX_FORMAT,
    TERRAIN_CHUNK_SLOT0_BYTES, TERRAIN_CHUNK_SLOT0_POSITION_FORMAT, TERRAIN_CHUNK_SLOT0_STRIDE,
    TERRAIN_CHUNK_SLOT1_STRIDE, TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT,
    TERRAIN_CHUNK_SLOT1_TILE_BYTES,
};
pub use tree_files::{
    compare_tree_instances_to_capture, default_map_tree_indices_from_resolver,
    parse_tree_gfx_bindings, tree_file_derived_layer_from_resolver,
    tree_gfx_bindings_from_resolver, tree_mesh_vertex_buffer_from_model, TreeFileDerivedLayer,
    TreeFileDerivedLayerStats, TreeFileTexture, TreeGfxBinding, TreeInstanceComparisonReport,
    TreeMeshVertexBuffer, TreePositionDistanceStats, TreeShaderPreparation, TREES_BMP_SOURCE,
    TREE_DEFAULT_MAP_SOURCE, TREE_GFX_BINDING_SOURCE, TREE_HEIGHTMAP_SOURCE,
    TREE_MESH_VERTEX_STRIDE_BYTES, TREE_SEASON_TEXTURE_SOURCE, TREE_SHADER_EFFECT,
    TREE_SHADER_SOURCE, TREE_TINT_TEXTURE_SOURCE,
};
pub use tree_instance_synthesis::{
    synthesize_tree_instances_from_resolver, TreeCaptureDecodeReport, TreeInstanceSynthesis,
    TreeInstanceSynthesisStats,
};
pub use tree_second_subdraw::{
    identify_tree_pixel_shader, tree_second_subdraw_verdict, TreePixelShaderEvidence,
    TreePixelShaderKind, TreeSecondSubDrawVerdict, TREE_FULL_PS_FNV1A64, TREE_LOW_GFX_PS_FNV1A64,
};
pub use types::IndexImage;
pub use underwater_pass::{UNDERWATER_EFFECT_NAME, UNDERWATER_SHADER_SOURCE};
pub use water_law::{
    apply_underwater_light_absorption, lean_flattened_det, lean_specular_peak,
    lean_specular_peak_glossiness, underwater_cb0_absorption_ratio,
    underwater_clip_height_threshold, underwater_defines, underwater_depth_fog_alpha,
    validate_packing_law_against_cb0, PackingLawResidual, UnderwaterLightDelta, VirtualLightPacked,
    CB0_OFFSET_SECOND_VIRTUAL_MOON_POS, CB0_OFFSET_SECOND_VIRTUAL_SUN_POS,
    CB0_OFFSET_VIRTUAL_MOON_POS, CB0_OFFSET_VIRTUAL_SUN_POS, UNDERWATER_LIGHT_ABSORPTION,
    UNDERWATER_PS_REQUIRED_SRV_SLOTS,
};
pub use weather_law::{
    animation_curve_peak, asset_subsystem_count, cloud_law_from_asset, cloud_law_from_resolver,
    cloud_texture_rgba_from_resolver, entity_particle_name, evaluate_animation_curve,
    pdxparticle_type, render_weather_overlay, weather_camera_fade, weather_effects_from_resolver,
    weather_effects_from_source, weather_emitters_from_resolver, weather_emitters_from_source,
    weather_fade_defines_from_resolver, weather_fade_defines_from_source, CloudRenderLaw,
    ParticleBlend, WeatherEffectBinding, WeatherEffectsTable, WeatherEmitter, WeatherEmitterTable,
    WeatherFadeDefines, WeatherOverlay, WeatherPositionSize, WeatherType, GRAPHICS_DEFINES_SOURCE,
    MAP_PLANE_X, MAP_PLANE_Z, PARTICLES_GFX_SOURCE, TO_LINEAR_GAMMA, VANILLA_WEATHER_FADE_DEFINES,
    WEATHER_EFFECTS_SOURCE, WEATHER_ENTITIES_SOURCE, WEATHER_POSITIONS_SOURCE,
};
