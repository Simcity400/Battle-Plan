use std::error::Error;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub use portable_render_stream::{
    AssignedStage, AssignedStageEdge, AssignedStageIdentity, AssignedStageSchedule,
    AssignedStageTopology, ElementDiagnostic, ElementOutcome, FamilyCoverageRow,
    FamilyDisposition, FoundationGap, FoundationOperationIdentity, FoundationRenderEnvelope,
    MapObjectDrawArguments, MapObjectMaterialFamily, MapObjectMeshGroupContent,
    MapObjectRenderContent, MapObjectSubmeshContent, MapVisualFamily, PreparedMapObjectDraw,
    ASSIGNED_STAGE_SCHEDULE_SCHEMA, ASSIGNED_STAGE_SCHEMA, ASSIGNED_STAGE_TOPOLOGY_SCHEMA,
    FOUNDATION_RENDER_SCHEMA, MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY,
};
pub use portable_render_stream::collapse_element_diagnostics;

mod bloom;
mod border_capture_evidence;
mod border_classification;
mod border_law;
mod border_pass;
mod border_texture_law;
mod border_world_synthesis;
mod building_files;
mod bundle_gen;
mod camera;
mod camera_interp_law;
mod cap4_sea_referee;
mod cli;
mod color_lut;
mod combat_indicator_assets;
mod constants;
mod counter_render;
mod cpu_terrain;
mod d3d11_device;
mod d3d11_replay;
mod fog_derivation_law;
mod fow_law;
mod frame_bundle;
mod game_data;
pub mod grading_law;
mod image_load;
mod json_store;
mod keying;
mod light_depth_projection;
mod light_law;
mod luminance_law;
mod m01_runtime_admission;
mod map_authority;
mod map_furniture;
mod map_icon_runtime;
mod map_label_capture;
mod map_label_law;
mod map_mode_admin;
mod map_mode_registry;
mod map_mode_terrain_resource;
mod map_static_data;
mod map_text;
mod mapmode_air;
mod mapmode_diplomacy;
mod mapmode_factions;
mod mapmode_infrastructure;
mod mod_resolver;
mod naval_supremacy_tint;
mod o05_fixed_object_law;
mod occupation_mapmode;
mod occupation_stripe_law;
mod pass_graph;
mod pdx_mesh;
mod political_color_synthesis;
mod posteffect_region_weights;
mod production_d3d;
mod production_features;
mod production_foundation;
mod production_map_interpreter;
mod production_map_renderer;
mod production_map_target;
mod rail_files;
mod reflection_cube_law;
mod restore_evidence;
mod restore_scene_law;
mod river_mesh_synthesis;
mod river_world_synthesis;
mod seasons;
mod shader_compile;
mod shader_meta;
mod shadow_law;
mod shadow_pass_plan;
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
mod tree_runtime_selection;
mod tree_second_subdraw;
mod types;
mod underwater_pass;
mod unit_counter_assets;
mod unit_placement;
mod util;
mod water_law;
mod weather_law;
mod xmap04_d3d_handoff;
mod xmap05_d3d_handoff;
mod xmap06_d3d_handoff;
mod xmap07_d3d_handoff;
mod xmap08_d3d_handoff;
mod xmap09_d3d_handoff;
mod xmap09_lighting_law;
mod xmap10_d3d_handoff;
mod xmap10_scene_cb0;
mod xmap11_d3d_handoff;
pub mod xmap12_building_law;
mod xmap12_d3d_handoff;
mod xmap12_pdxmesh;
mod xmap12_population;
pub mod xmap13_counter_control_law;
pub mod xmap13_counter_grouping_law;
mod xmap15_d3d_handoff;
mod xmap16_d3d_handoff;
mod xmap16_hub_handoff;
mod xmap16_supply_arrow_handoff;
mod xn13_unit_presentation;
mod xmap20_direct_visual_law;
mod xmap20_entity_presentation_law;

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
pub use border_classification::{
    border_selection_palette_from_graphics_defines, classify_border_chain, partition_border_chains,
    BorderClassification, BorderClassificationResidual, BorderSelectionPalette,
    BorderSelectorReason, BorderWorldFacts, ClassifiedBorderChain, TutorialBorderHighlight,
    ViewerBorderOverlays, XMAP06_CLASSIFICATION_RESIDUALS, XMAP06_CUSTOM_HIGHLIGHTS_DEFINE,
    XMAP06_FIRST_CUSTOM_HIGHLIGHT_REASON, XMAP06_HIDDEN_ROW_RESIDUAL,
    XMAP06_NAMED_PALETTE_DEFINES,
};
pub use border_law::{
    border_diffuse_source, border_fade_windows_from_graphics_defines,
    border_fade_windows_from_resolver, border_transparency, defines as border_fade_defines,
    BorderFadeWindow, BorderFadeWindows, BORDER_PASS_DRAW_ORDER_EVIDENCE,
};
pub use border_pass::{
    BorderFilesPassRenderReport, BorderFilesPassStats, BORDER_PASS_EFFECT_NAME,
    BORDER_PASS_SHADER_SOURCE,
};
pub use border_texture_law::{
    all_border_texture_profiles_from_resolver, border_texture_profile_from_resolver,
    BorderTextureProfile, BORDER_TILE, INK_ALPHA_THRESHOLD,
};
pub use border_world_synthesis::{
    border_material_sources_from_resolver, border_source_evidence_from_resolver,
    synthesize_border_world_mesh_from_resolver, synthesize_border_world_mesh_with_keying,
    BorderChain, BorderClassIndexPartition, BorderClassIndexRange, BorderMaterial,
    BorderMaterialSource, BorderRank, BorderSourceEvidenceReport, BorderSourceExcerpt,
    BorderWaterClass, BorderWorldMesh, BorderWorldSynthesisStats, BorderWorldVertex,
    BORDER_ADJACENCIES_CSV_SOURCE, BORDER_CONSTANTS_SOURCE, BORDER_DEFAULT_MAP_SOURCE,
    BORDER_DEFINITION_CSV_SOURCE, BORDER_HEIGHTMAP_BMP_SOURCE, BORDER_HISTORY_STATES_DIR,
    BORDER_PROVINCES_BMP_SOURCE, BORDER_SHADER_SOURCE, BORDER_VERTEX_STRIDE_BYTES,
};
pub use building_files::{
    building_entity_bindings_from_resolver, building_file_derived_layer_from_resolver,
    building_mesh_vertex_buffer_from_model, building_pdxmesh_bindings_from_resolver,
    building_preview_instances, parse_building_entity_bindings, parse_building_pdxmesh_bindings,
    resolve_building_material_textures, resolve_show_on_map_building_bindings,
    BuildingBindingCoverageRow, BuildingEntityBinding, BuildingFileDerivedLayer,
    BuildingFileDerivedLayerStats, BuildingMaterialTextureRef, BuildingMaterialTextureReport,
    BuildingMeshVertexBuffer, BuildingPdxmeshBinding, BuildingPreviewInstance,
    BuildingShaderCBufferMapping, BuildingShaderPreparation, MeshSettingTexture,
    ResolvedBuildingEntity, BUILDINGS_ENTITY_ASSET_SOURCE, BUILDINGS_GFX_SOURCE,
    BUILDING_MESH_VERTEX_STRIDE_BYTES, BUILDING_SHADER_EFFECT, BUILDING_SHADER_SOURCE,
};
pub use camera::{
    hoi4_map_camera_defines_from_bundle, map_wrap_offsets, Hoi4MapCameraDefines,
    ProductionMapCamera, ProductionMapCameraProjection, ViewerMapCameraState,
    DEFAULT_VIEWER_CAMERA_SPEED_SETTING, VIEWER_MAP_CAMERA_SCHEMA, XMAP03_A729_EXECUTABLE_SHA256,
    XMAP03_CAMERA_LAW_SCHEMA, XMAP03_ZOOM_IN_ACTION_ID, XMAP03_ZOOM_OUT_ACTION_ID,
};
pub use camera_interp_law::{
    camera_temporal_update_and_bounds_repair, CameraTemporalInputs, CameraTemporalOutcome,
    CameraTemporalState, CAMERA_DISTANCE_SQUARED_GATE, CAMERA_INTERPOLATION_SPEED,
    CAMERA_MAX_HEIGHT, CAMERA_MIN_HEIGHT, CAMERA_RAW_DELTA_CAP, CAMERA_SPEED_IN_MENUS,
};
pub use cap4_sea_referee::{
    find_unique_payload, mine_sea_base_referee, mine_sea_base_referee_round_dir, SeaBaseReferee,
};
pub use cli::run_cli;
pub use color_lut::{
    color_correction_lut_from_resolver, lut_blend1, lut_blend2, parse_color_correction_tga,
    scan_color_correction_luts_from_resolver, ColorCorrectionLut, ColorLutProbeRow,
    COLOR_CORRECTION_DIR,
};
pub use counter_render::{
    build_counter_instances, counter_render_assets_from_resolver,
    counter_texture_rgba_from_resolver, select_counter_display_inputs, CounterBadgeKind,
    CounterBadgeSelection, CounterColorMode, CounterDisplayOmission, CounterDisplayOmissionReason,
    CounterDisplaySelection, CounterFrameSelection, CounterHonesty, CounterPlayerMapping,
    CounterQuad, CounterQuadLayer, CounterQuadVertex, CounterRenderAssets, CounterRenderBatch,
    CounterRenderDomain, CounterRenderInput, CounterRenderInstance, CounterRenderOmission,
    CounterRenderOmissionReason, CounterScreenLaw, CounterStackingLaw, CounterTextureRgba,
    ScreenPointPx, ACTIVE_ABILITY_ICON_SCALE_FILE, ACTIVE_ABILITY_ITEM_SIZE_GUI_FILE,
    COUNTER_BAR_ROTATION_RADIANS_FILE, COUNTER_CAPTURE_STACK_STEP_GUI, COUNTER_FRAME_SPRITE,
    COUNTER_ORG_BAR_SPRITE, COUNTER_STRENGTH_BAR_SPRITE,
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
pub use fow_law::{
    analyze_fow_capture_pair, analyze_wave1_cap4_fow_pair, exact_a729_intel_map,
    exact_a729_intel_map_texel, exact_a729_land_unit_disclosure, exact_a729_taskforce_disclosure,
    pdxmap_fow_effective_alpha, shadow_map_bgra8_unorm_sample, ExactUnitDisclosure,
    FowCapturePairReport, FowChannelDiffCounts, FowFogFactors, FowPairDrawDiffReport,
    FowParityStats, FowShadowMapReport, ShadowMapFogSample, FOW_COLOR_MAX, FOW_COLOR_MIN,
    FOW_NOISE_SPEED, FOW_NOISE_TILING, FOW_SHADER_SOURCE, FOW_TRANSPARENCY_MAX,
    FOW_TRANSPARENCY_MIN, PDXMAP_SHADER_SOURCE, SHADOW_MAP_DXGI_FORMAT,
    WAVE1_CAP4_FOW_OFF_SHADOWMAP_FNV1A64, WAVE1_CAP4_FOW_ON_SHADOWMAP_FNV1A64,
};
pub use frame_bundle::{
    validate_borders_map_frame_bundle, validate_rivers_map_frame_bundle,
    validate_trees_map_frame_bundle, validate_water_map_frame_bundle, AdmittedProductionMapBundle,
    MapFrameVanillaSourceRefusal,
};
pub use keying::KeyingTables;
pub use light_depth_projection::{
    project_l03_light_depth, L03DrawArguments, L03LightDepthProjection, L03LightDepthSubmission,
    L03LightDepthTarget, L03MaterialFamily, L03PdxMeshGroupInput, L03PdxMeshObjectInput,
    L03PdxMeshSubmeshInput, L03ProjectionRequest, L03SelectedLight, L03SelectedLightFrame,
    L03SemanticInputRow, L03ShadowPipeline, L03TreeBatchInput, L03TypedOwner,
    L03_LIGHT_DEPTH_DXGI_FORMAT, L03_LIGHT_DEPTH_HEIGHT, L03_LIGHT_DEPTH_MIP_LEVELS,
    L03_LIGHT_DEPTH_WIDTH, L03_PARTITION_MASK_ALL, L03_PARTITION_PRIMARY, L03_PARTITION_WRAP_EAST,
    L03_PARTITION_WRAP_WEST,
};
pub use light_law::{defines as light_defines, offsets as light_offsets};
pub use light_law::{
    diffuse_color_rgb_from_hsv_define, night_yzw_residual, render_sun_dir_yzw,
    validate_lighting_law_against_cb0, LightingLawResidual, SceneLighting,
};
pub use xmap09_lighting_law::{
    engine_hour_of_day, engine_hour_of_day_fraction, virtual_rig, xmap09_shadow_fade_factor,
    whole_map_point_light_grid, Xmap09LightingDefines, Xmap09VirtualRig,
    XMAP09_ENGINE_HOUR_EPOCH, XMAP09_GRAPHICS_DEFINES_DIR,
    XMAP09_REPLAY_DAY_NIGHT_FACTOR,
};
pub use luminance_law::{
    adapt_luminance_temporal, f16_bits_to_f32 as luminance_f16_bits_to_f32,
    f32_to_f16_bits as luminance_f32_to_f16_bits, scene_average_luminance,
    scene_average_luminance_r16g16b16a16_float, AuthenticatedLuminanceCadence,
    AverageLuminanceReduction, LuminanceClampBounds, TemporalLuminanceContinuity,
    TemporalLuminanceReceipt, TemporalLuminanceResetReason, TemporalLuminanceState,
    TemporalLuminanceStep, LOG_LUMINANCE_EPSILON, LUMINANCE_VECTOR,
};
pub use m01_runtime_admission::{
    admit_m01_runtime_rows, parse_and_admit_m01_runtime_rows, M01A14ControlState,
    M01AdmissionError, M01AdmittedRuntimeRows, M01AirBaseProvinceIfWing, M01NegativeRuntimeControl,
    M01PositiveRuntimeRow, M01RuntimeInputEnvelope, M01RuntimeRowPair, M01UnitKind,
    M01VisibilityOrIntelState, M01_RUNTIME_INPUT_SCHEMA,
};
pub use map_authority::{
    account_owned_map_authority_inputs, admit_map_authority_json, admit_map_authority_projection,
    map_authority_observed_status_label, map_authority_owner_proven_admission,
    map_authority_projection_from_world_frame,
    map_authority_requirement_metadata, player_country_set_leaf_status,
    refuse_missing_xmap00_authority,
    refuse_product_map_until_xmap01, require_promoted_xmap01_renderer, MapAuthorityAdmission,
    MapAuthorityAdmittedInput, MapAuthorityGap, MapAuthorityOwnerProvenAdmission,
    MapAuthorityProjection, MapAuthorityRequirement, MapAuthorityRequirementDisposition,
    MapAuthorityRequirementMetadata, Xmap01ProductMapRefusal, MAP_AUTHORITY_INPUT_SCHEMA,
    MAP_AUTHORITY_OWNER_PROVEN_INPUTS, MAP_AUTHORITY_REQUIRED_INPUTS,
};
pub use map_furniture::{
    associate_naval_spawns_to_provinces, base_icon_defines_from_resolver, base_icon_frame,
    base_icon_layer_from_resolver, building_locator_pixel, capital_states_from_resolver,
    compute_base_icon_markers, compute_furniture_markers, cosmetic_tags_from_resolver,
    cosmetic_tags_from_resolver_with_dlc, country_colors_from_game_resolver,
    country_colors_from_game_resolver_with_session_metadata, furniture_layer_from_resolver,
    furniture_layer_with_keying, furniture_static_data_from_resolver, heightmap_grid_from_resolver,
    load_vp_label_font, load_vp_strip_rgba, parse_base_icon_defines, parse_bmfont,
    parse_state_building_levels, parse_vp_icon_defines, ruling_parties_from_resolver,
    state_building_levels_from_resolver, vp_icon_defines_from_resolver, vp_icon_relation_column,
    vp_strip_frame_index, vp_strip_texture_path, ActiveDlcPresentation, BaseIconDefines,
    BaseIconInputs, BaseIconKind, BaseIconLayer, BaseIconMarker, BmFont, BmFontGlyph,
    FurnitureLawInputs, FurnitureLayer, FurnitureMarker, FurnitureMarkerKind, FurnitureStaticData,
    HeightmapGrid, RgbaFontAtlas, StateBuildingLevels, VpIconDefines, VpIconDiplomaticPairState,
    AIRBASE_ICON_DISTANCE_CUTOFF_KEY, AIR_BASE_BUILDING, AIR_BASE_SPRITE_FRAMES,
    AIR_BASE_SPRITE_NAME, AIR_BASE_SPRITE_TEXTURE, AIR_BASE_STATE_MAX_LEVEL,
    CAPITAL_ICON_CUTOFF_KEY, CORE_GFX_SOURCE, HOI_20BS_ATLAS_SOURCE, HOI_20BS_FNT_SOURCE,
    MAPICONS_GFX_SOURCE, MAPICONS_GUI_SOURCE, MAPICON_CONTAINER_HEIGHT, MAPICON_CONTAINER_WIDTH,
    MAPICON_ICON_OFFSET_X, MAPICON_ICON_OFFSET_Y, MAPICON_TEXT_MAX_HEIGHT, MAPICON_TEXT_MAX_WIDTH,
    MAPICON_TEXT_OFFSET_X, MAPICON_TEXT_OFFSET_Y, NAVALBASE_ICON_DISTANCE_CUTOFF_KEY,
    NAVAL_BASE_BUILDING, NAVAL_BASE_PROVINCE_MAX_LEVEL, NAVAL_BASE_SPAWN_LOCATOR,
    NAVAL_BASE_SPRITE_FRAMES, NAVAL_BASE_SPRITE_NAME, NAVAL_BASE_SPRITE_TEXTURE,
    NAVAL_SPAWN_LAND_SNAP_RADIUS, VICTORY_POINTS_LOC_PREFIX, VICTORY_POINT_LEVELS_KEY,
    VICTORY_POINT_MAP_ICON_AFTER_KEY, VP_LABEL_FONT_NAME, VP_STRIP_COLOR_ENEMY_RED,
    VP_STRIP_COLOR_ENGINE_UNREACHED_1, VP_STRIP_COLOR_ENGINE_UNREACHED_3,
    VP_STRIP_COLOR_NEUTRAL_GRAY, VP_STRIP_COLOR_SELF_OR_FACTION, VP_STRIP_COLOR_SLOTS,
    VP_STRIP_FRAME_COUNT, VP_STRIP_SPRITE_NAME, VP_STRIP_SPRITE_TEXTURE,
};
pub use map_icon_runtime::{
    admit_victory_point_furniture_icons, admit_raid_furniture_icons, admit_resource_furniture_icons,
    exact_vp_file_anchors, hit_test_map_furniture, hit_test_prepared_map_furniture,
    load_map_furniture_sprite_assets, load_vp_strip_asset, prepare_map_furniture_packet,
    project_map_furniture_icons, project_raid_furniture, project_resource_furniture,
    resolve_resource_flag_assets, AdmittedMapFurnitureIcon, AdmittedRaidFurnitureIcon,
    AdmittedResourceFurnitureEntry, AdmittedResourceFurnitureIcon, ExactVpFileAnchor,
    MapFurnitureDistanceDefines, MapFurnitureGuiGeometry, MapFurnitureMaskedShieldDescriptor,
    MapFurniturePacketDisposition, MapFurniturePrepareRequest,
    MapFurnitureResourceFlagAssetDescriptor, MapFurnitureRgbaImage, MapFurnitureSharedIdentities,
    MapFurnitureSpriteAsset, MapFurnitureTarget, MapIconChildGeometry, MapIconContainerGeometry,
    MapIconRootGeometry, MapIconTextGeometry, MapModeFurnitureSelection,
    PreparedMapFurniturePacket, ProjectedMapFurnitureIcon, ProjectedRaidFurnitureIcon,
    ProjectedResourceFurnitureIcon, RaidMapIconGeometry, ScreenRect, Xmap21ResourceTextBatch,
    Xmap21ResourceTextOperation, Xmap21VpCapitalTextBatch, Xmap21VpCapitalTextOperation,
    RAID_MAP_ICON_DRAW_DISTANCE_MAX_KEY, RAID_MAP_ICON_DRAW_DISTANCE_MAX_RAID_MAP_MODE_KEY,
    RAID_MAP_ICON_DRAW_DISTANCE_MIN_KEY, RESOURCE_MAP_ICON_TEXT_CUTOFF_KEY,
    XMAP21_A729_EXECUTABLE_SHA256, XMAP21_ALIGNMENT_BITS, XMAP21_EXECUTE_OPERATION_NAME,
    XMAP21_EXECUTE_STAGE_NAME, XMAP21_PREPARED_PACKET_SCHEMA, XMAP21_PREPARE_OPERATION_NAME,
    XMAP21_PREPARE_RECEIPT_SCHEMA, XMAP21_PREPARE_STAGE_NAME, XMAP21_RAID_ALIGNMENT_BITS,
    XMAP21_RAID_GFX_SOURCES, XMAP21_RAID_MODE_ID, XMAP21_RAID_VIEWER_TOKEN,
    XMAP21_RESOURCE_ALIGNMENT_BITS, XMAP21_RESOURCE_FLAG_SAMPLING_LAW,
    XMAP21_RESOURCE_FLAG_SPRITE_NAME, XMAP21_RESOURCE_MODE_ID, XMAP21_RESOURCE_TEXT_BATCH_SCHEMA,
    XMAP21_RESOURCE_VIEWER_TOKEN, XMAP21_SORT_DEPTH_SCALE, XMAP21_VP_ANCHOR_SLOT,
    XMAP21_VP_CAPITAL_TEXT_BATCH_SCHEMA, XMAP21_VP_FALLBACK_SLOT,
};
pub use map_label_capture::{
    decode_label, decode_vertices, fit_baseline, glyph_quads, match_glyph, parse_fnt_chars,
    BaselineFit, DecodeError, DecodedLabel, FntChar, GlyphQuad, MapnameVertex,
    MAPNAME_VERTEX_STRIDE, POSITION_OFFSET, TEXCOORD_OFFSET, VERTS_PER_GLYPH,
};
pub use map_label_law::{
    build_mapname_vertices, build_mapname_vertices_with_height,
    compute_controller_label_placements, compute_controller_label_placements_with_text,
    compute_country_label_placements, compute_country_label_placements_with_text,
    compute_explicit_area_label_placements_with_text, label_draw_visible,
    mapname_measured_glyph_parameters, mapname_tracked_advance_px, mapname_tracking_px,
    principal_axis, ControllerLabelLawInputs, CountryLabelPlacement, EngineCurve,
    ExplicitAreaLabelLawInputs, GlyphScale, LabelLawInputs, LabelTextMetrics,
    MapLabelGlyphLayoutQuad, MapLabelStraightLayoutStation, ProvinceKind, ENGINE_TITLE_CAP_COEF,
    ENGINE_TITLE_FIT_COEF, LABEL_AB_HYSTERESIS, LABEL_HEIGHT_BREADTH_K, LABEL_OFFSET_PX,
    LABEL_SAMPLE_STEP,
};
pub use map_mode_admin::{
    admin_mapmode_defines_from_resolver, admin_mapmode_defines_from_source,
    ideology_colors_from_resolver, ideology_colors_from_source, ideology_fill,
    parse_ideology_colors_into, players_fill, state_boundary_outline, states_admin_fill,
    vanilla_ideology_colors, AdminMapModeDefines, IdeologyColorTable, StateAdminFill,
    StateBoundaryOutline, StateIdeologyFill, StatePlayersFill,
    ADMIN_MAPMODE_GRAPHICS_DEFINES_SOURCE, IDEOLOGIES_DIR, VANILLA_ADMIN_MAPMODE_DEFINES,
};
pub use map_mode_registry::{
    cycle_viewer_map_mode, HardcodedMapMode, ViewerMapModeSelection, FIRST_SCRIPTED_MAP_MODE,
    RAID_MAP_MODE_ENGINE_ALIAS, XMAP18_A729_EXECUTABLE_SHA256,
    XMAP18_REGISTRY_GHIDRA_MANIFEST_SHA256,
};
pub use map_mode_terrain_resource::{
    classify_resource_deposits, parse_resource_definitions, parse_state_resource_deposits,
    parse_terrain_category_colors, resource_definitions_from_resolver,
    resource_deposit_set_from_resolver, resource_state_fill, resource_state_overlay,
    terrain_category_colors_from_resolver, terrain_map_mode_from_resolver, ResourceDefinition,
    ResourceDepositSet, ResourceStateOverlay, TerrainMapMode, RESOURCE_DEFINITIONS_SOURCE,
    RESOURCE_DEPOSITS_STATES_DIR, RESOURCE_ICON_STRIP_SOURCE, TERRAIN_CATEGORIES_SOURCE,
    TERRAIN_DEFINITION_CSV_SOURCE, VANILLA_RESOURCE_DEFINITIONS, VANILLA_TERRAIN_CATEGORY_COLORS,
};
pub use map_static_data::{
    building_slot_rows_from_resolver, building_slot_rows_from_source,
    BuildingSlotSourceRows,
    map_building_placements_from_resolver,
    map_building_placements_from_source, railway_segments_from_resolver, railway_segments_from_source,
    show_on_map_buildings_from_resolver,
    show_on_map_buildings_from_source, supply_nodes_from_resolver,
    supply_nodes_from_source, unit_stack_placements_from_resolver, unit_stack_placements_from_source,
    weather_positions_from_resolver,
    weather_positions_from_source, MapBuildingPlacement, RailwaySegment, ShowOnMapBuilding,
    SupplyNode, UnitStackPlacement, WeatherPosition,
};
pub use map_text::{
    draw_country_names_cutoff_from_resolver, localisation_table_from_resolver,
    merge_localisation_source, merge_localisation_source_exact,
    DRAW_COUNTRY_NAMES_CUTOFF_KEY, LOCALISATION_ROOT,
};
pub use mapmode_air::{
    air_overlay_alpha, air_region_set_from_resolver, air_region_status, air_region_tint,
    air_region_tints, build_air_regions, pov_air_status_color, strategic_air_colors_from_resolver,
    strategic_air_colors_from_source, AirActivityInput, AirRegion, AirRegionSet, AirRegionStatus,
    AirRegionTint, PovAirRelation, StrategicAirColors, AIR_GRAPHICS_DEFINES_SOURCE,
    AIR_STRATEGIC_REGIONS_DIR, VANILLA_STRATEGIC_AIR_COLORS,
};
pub use mapmode_diplomacy::{
    classify_relation, country_relations, relation_fill, relation_to_vp_column, DiploPairInput,
    DiplomacyPalette, DiplomacyRelation, DIPLOMACY_GRAPHICS_DEFINES_SOURCE,
    DIPLOMACY_VP_STRIP_TEXTURE,
};
pub use mapmode_factions::{
    country_faction_memberships, faction_fill, factions_defines_from_resolver,
    factions_defines_from_source, membership_for, FactionInput, FactionMembership, FactionsDefines,
    FACTIONS_GRAPHICS_DEFINES_SOURCE, VANILLA_FACTIONS_DEFINES,
};
pub use mapmode_infrastructure::{
    infra_gradient_colors_from_resolver, infra_gradient_colors_from_source, infra_gradient_rgba,
    infra_ratio, infra_state_max_from_resolver, infra_state_tint, infra_state_tints,
    parse_infrastructure_state_max, InfraGradientColors, InfraLevelInput, InfraStateTint,
    INFRASTRUCTURE_BUILDING_KEY, INFRA_BUILDINGS_SOURCE, INFRA_GRAPHICS_DEFINES_SOURCE,
    VANILLA_INFRA_GRADIENT_COLORS, VANILLA_INFRA_STATE_MAX,
};
pub use mod_resolver::{
    semantic_descriptor_sha256, sensitive_file_overrides, stable_dlc_product_id,
    ContentAdmissionLease, GameContentSource, GameFileResolver, GameFileResolverOptions,
    GameFileResolverStats, PlaybackEnvironment, RecordedContentSource, RequiredGameContent,
    ResolvedGameFile, RestrictedGameFileResolver, SensitiveFileOverrideReport,
};
pub use naval_supremacy_tint::{
    classify_naval_regions, naval_region_set_from_resolver, naval_supremacy_tints,
    naval_tint_defines_from_resolver, naval_tint_defines_from_source, pov_stripe_color,
    province_is_sea, supremacy_alpha, supremacy_ratio, zone_supremacy_status, zone_tint,
    NavalRegionSet, NavalTintDefines, PovNavalRelation, SeaZone, ZoneSupremacyInput,
    ZoneSupremacyStatus, ZoneTint, NAVAL_SUPREMACY_DEFINITION_CSV_SOURCE,
    NAVAL_SUPREMACY_GRAPHICS_DEFINES_SOURCE, NAVAL_SUPREMACY_STRATEGIC_REGIONS_DIR,
    VANILLA_NAVAL_TINT_DEFINES,
};
pub use o05_fixed_object_law::{
    admit_o05_fixed_objects, admit_o05_fixed_objects_from_resolver,
    o05_fixed_object_source_hashes_from_resolver,
    o05_static_building_source_identities_from_resolver,
    o05_static_building_source_identities_from_source, O05FixedObjectAdmission,
    O05FixedObjectClass, O05StaticBuildingSourceIdentity, O05_FIXED_OBJECT_SOURCE_HASHES,
    O05_MAP_BUILDINGS_SOURCE,
};
pub use occupation_mapmode::{
    occupation_mapmode_color, occupation_palette_from_resolver, occupation_palette_from_source,
    OccupationColorStop, OccupationMapMode, OccupationMapModePalette,
    OCCUPATION_GRAPHICS_DEFINES_SOURCE,
};
pub use occupation_stripe_law::{
    apply_secondary_color, cam_distance as occupation_cam_distance, occupation_stripe_mask,
    smoothstep as occupation_smoothstep, SEC_MAP_TILE, STRIPE_ANGLE,
};
pub use pass_graph::{
    ProductionMapPassGraph, ProductionMapPassKind, ProductionMapPassPlan, ProductionMapPassStatus,
    ProductionMapTargetPlan,
};
pub use pdx_mesh::{
    extract_pdx_anim, extract_pdx_model, parse_pdx_asset, pdx_anim_from_resolver,
    pdx_model_from_resolver, pdx_model_to_obj,
    scan_pdx_assets_from_resolver, validate_pdx_model, PdxAnimData, PdxAnimJoint,
    PdxAssetScanReport, PdxAssetTree, PdxJoint, PdxLocator, PdxMaterial, PdxMeshData,
    PdxMeshValidationRow, PdxModel, PdxNode, PdxProperty, PdxShape, PdxSkin, PdxValues,
    PDX_ASSET_BINARY_MAGIC, PDX_ASSET_SUPPORTED_VERSION, PDX_ASSET_TEXT_MAGIC, PDX_MODELS_ROOT,
};
pub use political_color_synthesis::{
    compare_political_colors_to_capture, dump_political_color_comparison,
    political_color_composition_law, synthesize_political_colors,
    synthesize_political_colors_with_keying, InitialStateControl, PoliticalColorComparisonReport,
    PoliticalColorCompositionLaw, PoliticalColorDumpPaths, PoliticalColorMismatchSample,
    PoliticalColorSamplerDescriptor, PoliticalColorSynthesis, PoliticalColorSynthesisSource,
    PoliticalColorTextureDescriptor, PROVINCE_COLOR_MAP_DXGI_FORMAT, PROVINCE_COLOR_MAP_HEIGHT,
    PROVINCE_COLOR_MAP_WIDTH, PROVINCE_SECONDARY_COLOR_MAP_SHADER_SLOT,
};
pub use portable_render_stream::{Xmap15OwnerKind, Xmap15PlanKey};
pub use posteffect_region_weights::{
    posteffect_region_weights_from_resolver, PostEffectRegionBoxInput,
    PostEffectRegionWeightManifest, PostEffectRegionWeightSet, RegionProvinceWeight,
    POSTEFFECT_REGION_DEFINITION_SOURCE, POSTEFFECT_REGION_PROVINCES_SOURCE,
    POSTEFFECT_REGION_STATES_SOURCE, POSTEFFECT_REGION_WEIGHT_SCHEMA,
};
pub use production_d3d::{
    building_presentation_priority, execute_production_d3d_frame,
    has_authenticated_d80_map_operations, prepare_production_d3d_frame,
    prepare_s1_shadow_execution, validate_s1_shadow_execution, BuildingPresentationPriority,
    OccupationSecondaryD3dExecutionProof, PreparedProductionD3dFrame, PreparedS1ShadowExecution,
    ProductionD3dExecutedDraw, ProductionD3dExecutionBridge, ProductionD3dExecutionReceipt,
    ProductionD3dOmission, ProductionD3dStaticPassProgram, SnowMudD3dExecutionProof,
    PRODUCTION_D3D_EXECUTION_RECEIPT_SCHEMA,
};
#[cfg(windows)]
pub use production_d3d::{
    execute_foundation_content_operation, NativeProductionD3dBackend,
    NativeProductionD3dExecutor, ProductionD3dReadback,
};
pub use production_features::xmap13::{
    prepare_xmap13_effect_identity, PreparedXmap13CounterGroup, PreparedXmap13CounterItem,
    PreparedXmap13Frame, PreparedXmap13HitTarget, PreparedXmap13Unit, Xmap13AuthoredEffectKind,
    Xmap13AuthoredStateEvent, Xmap13CounterAllegiance, Xmap13CounterFrameSelection,
    Xmap13CounterGroupMembership, Xmap13CounterHorizontalAlignment, Xmap13CounterItemKind,
    Xmap13CounterPresentation, Xmap13CounterPrimitive, Xmap13CounterSemantic,
    Xmap13CounterSpriteBinding, Xmap13CounterTextValue, Xmap13CounterVerticalAlignment,
    Xmap13EffectIdentityProvider, Xmap13EffectIdentityReceipt, Xmap13FormationIdentity,
    Xmap13IdentityProvider, Xmap13NativeComponent, Xmap13NativeComponentReceipt,
    Xmap13NativeDrawClosure, Xmap13SemanticEnvelope, Xmap13SemanticRow,
    Xmap13TargetExecutionIdentity, Xmap13TaskForceMember, Xmap13UnitIdentity,
    XMAP13_COUNTER_QUAD_OPERATION_NAME, XMAP13_COUNTER_QUAD_STAGE_NAME, XMAP13_DOMAIN,
    XMAP13_EFFECT_IDENTITY_RECEIPT_SCHEMA, XMAP13_INPUT_SCHEMA, XMAP13_NATIVE_RECEIPT_SCHEMA,
    XMAP13_PREPARED_SCHEMA, XMAP13_UNIT_ENTITY_OPERATION_NAME, XMAP13_UNIT_ENTITY_STAGE_NAME,
};
pub use production_features::xmap14::{
    PreparedXmap14CounterDraw, PreparedXmap14Draw, PreparedXmap14Frame, PreparedXmap14HitTarget,
    PreparedXmap14Indicator, PreparedXmap14ProgressDraw, PreparedXmap14SpriteDraw,
    PreparedXmap14TextDraw, Xmap14CombatEnvelope, Xmap14ExecutionReceipt, Xmap14IndicatorIdentity,
    Xmap14LandCarrier, Xmap14LandTint, Xmap14NativeDrawClosure, Xmap14NavalCarrier,
    Xmap14NavalCategoryCounts, Xmap14NavalResultCarrier, Xmap14NavalResultMember,
    Xmap14NavalResultSubtype, Xmap14PreparedKind, Xmap14ViewerSide, XMAP14_COMBAT_OPERATION_NAME,
    XMAP14_COMBAT_STAGE_NAME, XMAP14_NATIVE_RECEIPT_SCHEMA,
};
pub use production_features::xmap15::{
    prepare_xmap15_frame, xmap15_viewer_selection, PreparedXmap15Frame,
    PreparedXmap15MovementDefinition, PreparedXmap15PlanDefinition, Xmap15LifecycleState,
    Xmap15LifecycleUpdate, Xmap15UnitIdentityProvider, Xmap15ViewerSelection,
    XMAP15_ASSIGNED_OPERATION_NAME, XMAP15_ASSIGNED_STAGE_NAME, XMAP15_HEIGHTMAP_PATH,
    XMAP15_MAPARROWS_PATH, XMAP15_MAPARROW_SHADER_PATH, XMAP15_MAP_DIMENSIONS_PATH,
    XMAP15_MOVEMENT_DEFINITIONS_PATH, XMAP15_MOVEMENT_EFFECT_PATH, XMAP15_MOVEMENT_SHADER_PATH,
    XMAP15_SEMANTIC_RAW_PATH,
};
pub use production_features::xmap16::{
    PreparedXmap16Frame, PreparedXmap16RailProgram, PreparedXmap16RailSampler,
    XMAP16_HUB_OPERATION_NAME, XMAP16_HUB_STAGE_NAME, XMAP16_RAIL_SUPPLY_SCHEMA,
};
pub use production_features::xmap17::{
    MapLabelFinalImageDependencies, MapNameAlphaState, PreparedMapLabelFrame,
    PreparedMapLabelProgram, PreparedMapLabelRow, PreparedMapLabelSet, ReadyMapLabelDrawContract,
    Xmap17DrawDisposition, Xmap17DrawReceipt, Xmap17ExecutionIdentity, XMAP17_EXECUTE_OPERATION,
    XMAP17_EXECUTE_STAGE, XMAP17_PREPARE_OPERATION, XMAP17_PREPARE_STAGE,
};
pub use production_features::xmap18::{
    EngineFillUpdate, EngineMapModeConfiguration, EngineProvinceColorWrite, MapModeComposition,
    MapModeFillSource, PreparedMapModeFrame, Xmap18OwnerResourceRef, Xmap18OwnerResourceRole,
    XMAP18_FILL_SOURCE_SCHEMA, XMAP18_MAP_MODE_SCHEMA,
};
pub use production_features::xmap19::{
    PreparedWeatherAuthoredEvent, PreparedWeatherEntityProgram, PreparedWeatherParticleEffect,
    PreparedWeatherPosition, PreparedWeatherRegionEpoch, PreparedWeatherSelection,
    PreparedXmap19WeatherFrame, Xmap19WeatherTrigger, XMAP19_A729_EXECUTABLE_SHA256,
    XMAP19_A729_PRODUCER_RVA, XMAP19_ASSIGNED_OPERATION_NAME, XMAP19_ASSIGNED_STAGE_NAME,
    XMAP19_WEATHER_SCHEMA,
};
#[cfg(windows)]
pub use production_foundation::execute_foundation_content_preflight;
pub use production_foundation::{
    foundation_map_dimensions, prepare_foundation_frame,
    product_feature_adapters_with_assigned_stage_schedule,
    product_foundation_assigned_stage_topology, require_fresh_foundation_frame,
    FoundationContentExecutionReceipt, FoundationFeatureContext, FoundationFeatureOutput,
    FoundationPlaybackOutcome, FoundationPlaybackState, FoundationProductUnavailable,
    PreparedFoundationFrame, ProductionMapFeatureAdapter, ProductionMapFeatureAdapters,
    ViewerOwnedMapState, Xmap15FoundationState, Xmap16FoundationState,
    FOUNDATION_CONTENT_EXECUTION_RECEIPT_SCHEMA,
};
pub use production_map_interpreter::{
    materialize_static_program, MapInterpreterFailure, RAW_VERTEX_BUFFERS_V1,
    RAW_VERTEX_INDEX_BUFFERS_V1,
};
pub use production_map_renderer::{
    selected_c03_camera_replay_binding, validate_c03_camera_consumer,
    validate_c03_camera_replay_binding, C03CameraReplayBinding, C03CameraReplayMode,
    PreparedProductionMapFrame, PreparedRuntimeDraw, ProductionMapDataNeed, ProductionMapRenderer,
    ProductionMapResource, C03_CAMERA_REPLAY_BINDING_SCHEMA,
};
#[cfg(windows)]
pub use production_map_target::NativeProductionMainSceneTarget;
pub use production_map_target::{
    production_main_scene_desc, production_main_scene_desc_from_outputs,
    production_main_scene_inputs_from_viewer, ProductionMainSceneInputs,
    ProductionMainSceneTargetDesc,
};
pub use rail_files::{
    bind_exact_rail_construction_arrows, bind_exact_rail_d80_draws, compute_province_centroids,
    cubic_bezier, exact_rail_segments_from_replay, parse_rail_geometry_defines,
    prepare_exact_rail_replay_carrier, rail_control_handle_len,
    rail_file_derived_layer_from_resolver, rail_geometry_defines_from_resolver, rail_level_v_band,
    rail_max_level_from_resolver, rail_polylines, rail_render_law, rail_ribbon_edges,
    railway_alpha_draws, railway_alpha_step, supply_hub_markers, ExactRailConstructionArrowBinding,
    ExactRailConstructionArrowEntityDrawBinding, ExactRailD80Binding, ExactRailD80DrawBinding,
    ExactRailHubVisualState, ExactRailReplayCarrier, ExactRailResourceIdentity, ExactRailSegment,
    ProvinceCentroid, RailFileDerivedLayer, RailFileDerivedStats, RailGeometryDefines, RailLawRow,
    RailPolyline, SupplyHubContentIdentity, SupplyHubMarker, SupplyHubSpriteBinding,
    A729_RAILROAD_EFFECT_REGISTRATION_ROOT_RVA, A729_RAILROAD_VERTEX_STRIDE_BYTES,
    DEFINES_SOURCE as RAIL_DEFINES_SOURCE, DEFINITION_CSV_SOURCE as RAIL_DEFINITION_CSV_SOURCE,
    GRAPHICS_DEFINES_SOURCE as RAIL_GRAPHICS_DEFINES_SOURCE, MAP_HEIGHT as RAIL_MAP_HEIGHT,
    MAP_WIDTH as RAIL_MAP_WIDTH, PROVINCES_BMP_SOURCE as RAIL_PROVINCES_BMP_SOURCE,
    RAILWAY_ALPHA_DRAW_THRESHOLD, RAILWAY_ALPHA_INITIAL, RAIL_LEVEL_MAX, RAIL_MAX_LEVEL_DEFINE,
    RAIL_SHADER_EFFECT, RAIL_SHADER_SOURCE, RAIL_TEXTURE_DAMAGED_SOURCE, RAIL_TEXTURE_SOURCE,
    SUPPLY_GFX_SOURCE, SUPPLY_HUB_ICON_FRAMES, SUPPLY_HUB_ICON_SPRITE, SUPPLY_HUB_ICON_TEXTURE,
};
pub use restore_evidence::{
    load_restore_scene_bundle, load_restore_scene_bundle_session_dir,
    probe_restore_scene_evidence_session_dir, RestoreSceneAverageLuminance, RestoreSceneBundle,
    RestoreSceneColorCube, RestoreSceneEvidence, RestoreSceneEvidenceProbeReport,
    RestoreSceneTexturePayloadSummary, RestoreSceneUniforms, RESTORE_PASS_BUNDLE_FILE,
};
pub use river_mesh_synthesis::{
    MapLayerSynthesisInputs, RiverCaptureDecodeReport, RiverCaptureStation, RiverFadeDefines,
    RiverMeshSynthesis, RiverMeshSynthesisStats, RiverPaletteDefines, RIVER_FADE_DEFINES_SOURCE,
    RIVER_PALETTE_DEFINES_SOURCE,
};
pub use river_world_synthesis::{
    classify_river_pixel, compare_river_world_mesh_to_capture, heightmap_bmp_from_resolver,
    load_captured_river_world, river_extraction_stats, river_world_width_for_class,
    rivers_bmp_from_resolver, synthesize_river_world_mesh,
    synthesize_river_world_mesh_from_resolver, synthesize_river_world_mesh_with_defines,
    CapturedRiverWorld, RiverDistanceStats, RiverExtractionStats, RiverPixelKind,
    RiverWidthClassRow, RiverWorldComparisonReport, RiverWorldMesh, RiverWorldStrip,
    RiverWorldSynthesisParams, RiverWorldVertex, HEIGHTMAP_BMP_SOURCE, RIVERS_BMP_SOURCE,
};
pub use seasons::{
    apply_tree_season_cb_override, seasons_table_from_resolver,
    seasons_table_from_source, tree_season_cb_values, SeasonBlock, SeasonColorGrade, SeasonDate,
    SeasonDateRange, SeasonId, SeasonsTable, TreeSeasonId,
};
pub use shader_meta::PoliticalSceneCb0;
pub use shadow_law::{
    build_shadow_frame, calculate_shadow, compose_shadow_map_texture_matrix,
    fit_shadow_map_matrices, get_shadow_scaled, pcf_tap_offsets, select_shadow_light,
    shadow_blur_3tap, shadow_blur_constant_buffers, shadow_blur_constant_buffers_from_factor,
    shadow_fade_factor, shadow_gated_by_daynight, terrain_shadow_term, SelectedShadowLight,
    ShadowBlurConstantBuffers, ShadowFrameInput, ShadowFrameMatrices, ShadowLightKind, ShadowMat4,
    DRAW_SHADOWS_CUTOFF, DRAW_SHADOWS_FADE_LENGTH, LIGHT_SHADOW_DIRECTION, PCF_BIAS,
    SHADOW_BLUR_CAMERA_DISTANCE, SHADOW_BLUR_OFFSET_1_BASE, SHADOW_BLUR_OFFSET_2_BASE,
    SHADOW_BLUR_WEIGHTS, SHADOW_FIT_EYE_MARGIN, SHADOW_FIT_GRID, SHADOW_MAP_SIZE,
    SHADOW_WEIGHT_TERRAIN, SHADOW_WEIGHT_WATER,
};
pub use shadow_pass_plan::{
    plan_s1_shadow_screen_chain, ShadowScreenChainPlan, ShadowScreenPassKind, ShadowScreenPassPlan,
    ShadowScreenTarget, SHADOW_SCREEN_BIND_FLAGS, SHADOW_SCREEN_DXGI_FORMAT,
    SHADOW_SCREEN_MIP_LEVELS,
};
pub use xmap12_building_law::*;
pub use xmap12_d3d_handoff::*;
pub use xmap12_pdxmesh::*;
pub use xmap12_population::*;
#[cfg(windows)]
pub use xmap15_d3d_handoff::{NativeXmap15DrawInputs, NativeXmap15Handoff};
pub use xmap15_d3d_handoff::{
    Xmap15NativeExecutionReceipt, Xmap15NativeRowReceipt, XMAP15_MAPARROW_CB3_BYTES,
    XMAP15_MOVEMENT_CB1_BYTES,
};
pub use xn13_unit_presentation::{
    admit_xn13_rows, parse_and_admit_xn13_rows, Xn13AdmissionError, Xn13AdmittedPresentationRows,
    Xn13AnimationBinding, Xn13DamageVariant, Xn13EffectKind, Xn13EffectTrigger, Xn13InputEnvelope,
    Xn13PassPoses, Xn13PoseBinding, Xn13PresentationRow, Xn13SelectionEffect, XN13_INPUT_SCHEMA,
};

pub use xmap20_direct_visual_law::{
    derive_direct_visual_coverage, derive_direct_visual_coverage_from_parts,
    DerivedDirectVisualCoverage,
};
pub use xmap20_entity_presentation_law::{
    derive_entity_presentation, derive_entity_presentation_from_parts,
    DerivedEntityPresentation,
};


pub use combat_indicator_assets::{
    combat_indicator_assets_from_resolver, combat_indicator_fonts_from_resolver,
    combat_indicator_gui_container_count, combat_indicator_gui_layouts_from_resolver,
    combat_indicator_progress_bars_from_resolver, land_combat_indicator_sprites_from_resolver,
    naval_combat_indicator_sprites_from_resolver, CombatIndicatorAssetManifest,
    CombatIndicatorGuiLayouts, CombatIndicatorKind, CombatIndicatorPlacement,
    COMBAT_INDICATOR_LANDCOMBAT_GFX_SOURCE, COMBAT_INDICATOR_MAPICONS_GFX_SOURCE,
    COMBAT_INDICATOR_MAPICONS_GUI_SOURCE, COMBAT_INDICATOR_NAVALCOMBAT_GFX_SOURCE,
    COMBAT_INDICATOR_NAVAL_LEADER_MODULE_GFX_SOURCE,
};
pub use production_features::xmap04::{
    xmap04_map_copy_receipt_set_sha256, xmap04_replay_frame_identity_sha256, PreparedTerrainDraw,
    PreparedTerrainFrame, PreparedTerrainProgram, PreparedUnderwaterTerrainProgram,
    ReadyTerrainDrawContract, TerrainDependentResourceBinding, TerrainGeneratedResourceDescriptor,
    TerrainSamplerRequirement, TerrainVisibleDrawDependencies, Xmap04MapCopyReceiptRow,
    XMAP04_REPLAY_FRAME_IDENTITY_DOMAIN,
};
pub use production_features::xmap05::{
    xmap05_admission_verdict, PreparedPoliticalFrame, PreparedXmap05Visibility,
    Xmap05AdmissionVerdict, Xmap05AuthenticatedMapClock,
    Xmap05GeneratedTexture, Xmap05SceneCb0Patch, Xmap05TextureDescriptor, Xmap05VisibilityReason,
    Xmap05VisibleEntity, GRADIENT_BORDER_CHANNEL3_DXGI_FORMAT, GRADIENT_BORDER_CHANNEL_DXGI_FORMAT,
    INTEL_MAP_DXGI_FORMAT, XMAP05_MAP_CLOCK_SCHEMA, XMAP05_POLITICAL_SCHEMA,
    XMAP05_SCENE_CB0_PATCH_SCHEMA, XMAP05_VISIBILITY_SCHEMA,
};
pub use production_features::xmap06::{
    PreparedThinBorderBucketDraw, PreparedThinBorderBucketGeometry, PreparedThinBorderClass,
    PreparedThinBorderFadeWindow, PreparedThinBorderFadeWindows, PreparedThinBorderFrame,
    PreparedThinBorderRuntimePartition, XMAP06_BORDER_CB2_BYTES, XMAP06_EARLY_OPERATION_NAME,
    XMAP06_EARLY_STAGE_NAME, XMAP06_LATE_OPERATION_NAME, XMAP06_LATE_STAGE_NAME,
    XMAP06_THIN_BORDER_SCHEMA,
};
pub use production_features::xmap07::{
    PreparedWaterFrame, PreparedWaterGeneratedTexture, PreparedWaterProgram, PreparedWaterSampler,
    ReadyWaterDrawContract, WaterDependentResourceBinding, WaterVisibleDrawDependencies,
    Xmap07SceneSnowMudInput, Xmap07SurfaceTargetRequirement, Xmap07WaterTemporalBinding,
    XMAP07_ASSIGNED_OPERATION_NAME, XMAP07_ASSIGNED_STAGE_NAME, XMAP07_REFRACTION_FORMAT,
    XMAP07_SNOW_MUD_FORMAT, XMAP07_SURFACE_TARGET_REQUIREMENT_SCHEMA, XMAP07_WATER_EFFECT_FILE,
    XMAP07_WATER_SCHEMA, XMAP07_WATER_TOPOLOGY, XMAP07_WATER_VERTEX_COUNT,
    XMAP07_WATER_VERTEX_FORMAT, XMAP07_WATER_VERTEX_STRIDE,
};
pub use production_features::xmap08::{
    xmap08_assigned_stage_edges, xmap08_assigned_stage_identities, MapConstantBufferBinding,
    MapDependentResourceBinding, PreparedMapEffectProgram, PreparedMapInputElement,
    PreparedMapPipelineState, PreparedMapSamplerRequirement, PreparedRiverDraw,
    PreparedRiversStraitsFrame, PreparedStraitAdjacency, PreparedStraitDraw,
    ReadyRiversStraitsDrawContract, RiversStraitsVisibleDrawDependencies, StraitAdjacencyKind,
    XMAP08_RIVERS_STRAITS_SCHEMA, XMAP08_RIVER_OPERATION_ID, XMAP08_RIVER_STAGE_NAME,
    XMAP08_STRAIT_OPERATION_ID, XMAP08_STRAIT_STAGE_NAME,
};
pub use production_features::xmap09::{
    xmap09_assigned_stage_topology, PreparedXmap09LightingFrame, Xmap09ComposedSceneCb0,
    Xmap09LightingInput, Xmap09PointLight, Xmap09ReflectedSceneCb0Field,
    Xmap09ReflectedSceneCb0Layout, Xmap09SceneCb0ApplyReceipt, Xmap09SceneCb0FieldPatch,
    Xmap09SceneCb0Patch, Xmap09SceneConstantPatch, Xmap09ShadowMatrixBinding,
    Xmap09ShadowResourcePlan, Xmap09TextureResource, XMAP09_A729_EXECUTABLE_SHA256,
    XMAP09_EXECUTE_OPERATION_NAME, XMAP09_EXECUTE_STAGE_NAME, XMAP09_INPUT_SCHEMA,
    XMAP09_LIGHT_DATA_MAP_SYMBOL, XMAP09_LIGHT_INDEX_MAP_SYMBOL,
    XMAP09_POINT_LIGHT_CB2_SHADER_REGISTER, XMAP09_POINT_LIGHT_CB2_SYMBOL, XMAP09_PREPARED_SCHEMA,
    XMAP09_PREPARE_OPERATION_NAME, XMAP09_PREPARE_STAGE_NAME, XMAP09_SCENE_CB0_APPLY_SCHEMA,
    XMAP09_SCENE_CB0_BYTE_LEN, XMAP09_SCENE_CB0_PATCH_SCHEMA, XMAP09_SCENE_LIGHT_WRITER_RVA,
    XMAP09_SHADOW_BLUR_FACTOR_RVA, XMAP09_SHADOW_CHAIN_RVA, XMAP09_SHADOW_FIT_RVA,
    XMAP09_SHADOW_MAP_SYMBOL,
};
pub use production_features::xmap10::{
    PreparedXmap10Frame, ReadyXmap10Frame, Xmap10ActiveEffectVariant,
    Xmap10CompiledProgramIdentity, Xmap10ContentClosure, Xmap10LutBlendPass,
    Xmap10LutBlendPassKind, Xmap10LutBlendProgram, Xmap10LutInput, Xmap10MainSceneDepthDescriptor,
    Xmap10NativeCompositorReceiptRef, Xmap10NativeProgramReceiptRef, Xmap10PassKind,
    Xmap10PassStep, Xmap10PerFrameGradeInputs, Xmap10ProgramRole, Xmap10SceneCb0ReceiptRef,
    Xmap10ShaderTranslationUnit, Xmap10TargetDescriptor, Xmap10TargetPlan, Xmap10TargetRole,
    Xmap10TemporalAdaptationReceiptRef, Xmap10TemporalAdaptationSource, Xmap10ViewportExtent,
    XMAP10_LUT_BLEND_PROGRAM_SCHEMA, XMAP10_NATIVE_COMPOSITOR_RECEIPT_SCHEMA,
    XMAP10_NATIVE_PROGRAM_RECEIPT_SCHEMA, XMAP10_PREPARATION_SCHEMA, XMAP10_READY_SCHEMA,
    XMAP10_POSTFX_DRAW_CONTRACT, XMAP10_SCENE_CB0_RECEIPT_SCHEMA,
    XMAP10_TEMPORAL_RECEIPT_SCHEMA,
};
pub use production_features::xmap20::{
    prepare_xmap20_effects, PreparedXmap20Frame, Xmap20EffectsAdapter,
    Xmap20ProductionTriggerProvider, Xmap20TriggerProvider, Xmap20TriggerReceipt,
    XMAP20_ASSIGNED_OPERATION_NAME, XMAP20_ASSIGNED_STAGE_NAME, XMAP20_CARRIER_ABSENT_GAP_ID,
    XMAP20_OPEN_CARRIER_GATES, XMAP20_PREPARED_FRAME_SCHEMA, XMAP20_TRIGGER_RECEIPT_SCHEMA,
};
pub use production_features::xmap21::{
    Xmap21MapFurnitureAdapter, XMAP21_FURNITURE_ABSENT_GAP_ID, XMAP21_REQUIRED_AUTHORITY_INPUTS,
};
pub use restore_scene_law::{restore_scene_pixel_rgb, RestoreSceneColorCube32, RestoreSceneGrade};
pub use shader_compile::{
    build_stage_hlsl, derive_a729_engine_map_shader_constants, effect_file_from_resolver,
    effect_translation_unit_files, engine_map_shader_defines, parse_dxbc_reflection,
    parse_effect_file, scan_effect_files_from_resolver, vanilla_engine_shader_defines, DxbcCbuffer,
    DxbcCbufferVar, DxbcReflection, DxbcResource, DxbcSignatureParam, EffectDef, EffectElement,
    EffectFile, EffectParseScan, EffectSampler, EffectStage, EngineMapShaderConstants,
    ShaderCompileComparison, ShaderStage, DX11_PLATFORM_HEADER, MAP_EFFECT_CANDIDATES,
    MAP_EFFECT_COMPILE_ONLY, SHADER_FX_DIR,
};
#[cfg(windows)]
pub use shader_compile::{
    compare_effect_stage_to_captured, compare_map_effects_to_capture, d3d_compile_stage,
    COMPILE_FLAG_MATRIX,
};
pub use snow_law::{
    captured_snow_mud_stats, compare_floor_to_capture, decode_snow_mud_texel,
    encode_snow_mud_texel, parse_region_weather, region_weather_from_resolver, snow_mud_fade_from_cb0, synthesize_snow_floor_from_resolver,
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
    terrain_lod_cull_update, terrain_lod_map_validate_corpus,
    terrain_lod_map_validate_corpus_with_resolver, terrain_lod_stride_map, terrain_lod_tiles,
    terrain_vsnap_from_neighbour_lods, terrain_vsnap_law_validate_corpus,
    terrain_vsnap_law_validate_corpus_with_resolver, TerrainLodCorpusReport, TerrainLodCullOptions,
    TerrainLodCullRecord, TerrainLodCullState, TerrainLodStrideMap, TerrainLodTile,
    TerrainRenderViewFrustum, TerrainRenderViews, TerrainTileEdge, TerrainVsnapCorpusReport,
    TerrainVsnapEdgeCheck, TerrainVsnapEdgeClass, TERRAIN_LOD_BASE_TILE_SIZE,
    TERRAIN_LOD_EDGE_COUNT, TERRAIN_LOD_REFERENCE_HEIGHT, TERRAIN_LOD_THRESHOLDS_SQ,
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
    tree_gfx_bindings_from_resolver, tree_mesh_vertex_buffer_for_shape,
    tree_mesh_vertex_buffer_from_model, TreeFileDerivedLayer, TreeFileDerivedLayerStats,
    TreeFileTexture, TreeGfxBinding, TreeInstanceComparisonReport, TreeMeshVertexBuffer,
    TreePositionDistanceStats, TreeShaderPreparation, TREES_BMP_SOURCE, TREE_DEFAULT_MAP_SOURCE,
    TREE_GFX_BINDING_SOURCE, TREE_HEIGHTMAP_SOURCE, TREE_MESH_VERTEX_STRIDE_BYTES,
    TREE_SEASON_TEXTURE_SOURCE, TREE_SHADER_EFFECT, TREE_SHADER_LOW_GFX_EFFECT,
    TREE_SHADER_SHADOW_EFFECT, TREE_SHADER_SOURCE, TREE_SHADER_UNLIT_EFFECT,
    TREE_ENVIRONMENT_TEXTURE_SOURCE,
    TREE_TINT_TEXTURE_SOURCE,
};
pub use tree_instance_synthesis::{
    exact_tree_yaw_radians, sample_map_height_a729, synthesize_tree_instances_from_resolver,
    TreeCaptureDecodeReport, TreeInstanceBatch, TreeInstanceSynthesis, TreeInstanceSynthesisStats,
    TREE_DEGREES_TO_RADIANS_BITS, TREE_INSTANCE_BUCKET_COUNT, TREE_INSTANCE_STRIDE_BYTES,
    TREE_YAW_HASH_ADDEND, TREE_YAW_HASH_MULTIPLIER_1, TREE_YAW_HASH_MULTIPLIER_2,
    TREE_YAW_HASH_SUBTRAHEND, TREE_YAW_KEY_MULTIPLIER, TREE_YAW_SEED,
};
pub use tree_runtime_selection::{
    prepare_xmap11_tree_frame, select_tree_chunk_partition, select_tree_lod_index,
    synthesize_tree_mask_texture, tree_fade_window_from_graphics_defines,
    tree_fade_window_from_resolver, tree_target_bound_draw_receipt, xmap11_assigned_stage_edges,
    xmap11_assigned_stage_rows, xmap11_l03_shadow_batches, PreparedTreeBatch, PreparedTreeFrame,
    PreparedTreeStaticPayload,
    TreeActualDrawExecution, TreeCompiledProgram, TreeFadeWindow, TreeFrameInput, TreeFrustumPlane,
    TreeMaskTexture, TreePipelineStateDescriptor, TreePreparedAsset, TreeSamplerStateDescriptor,
    TreeShaderPrograms, TreeTargetBoundDrawReceipt, TreeViewerPartition, TreeViewerState,
    XMAP11_TREE_CULL_MAX_Y_BITS, XMAP11_TREE_DISTANCE_Y_BITS, XMAP11_TREE_FRAME_SCHEMA,
    XMAP11_TREE_GRAPHICS_DEFINES_SOURCE, XMAP11_TREE_LOW_GFX_EFFECT, XMAP11_TREE_MAIN_EFFECT,
    XMAP11_TREE_MASK_DXGI_FORMAT, XMAP11_TREE_MASK_TEXTURE_SCHEMA, XMAP11_TREE_OPERATION_NAME,
    XMAP11_TREE_SHADOW_EFFECT, XMAP11_TREE_STAGE_NAME,
    XMAP11_TREE_TARGET_BOUND_DRAW_SCHEMA, XMAP11_TREE_UNLIT_EFFECT,
};
pub use tree_second_subdraw::{
    identify_tree_pixel_shader, tree_second_subdraw_verdict, TreePixelShaderEvidence,
    TreePixelShaderKind, TreeSecondSubDrawVerdict, TREE_FULL_PS_FNV1A64, TREE_LOW_GFX_PS_FNV1A64,
};
pub use types::IndexImage;
pub use underwater_pass::{UNDERWATER_EFFECT_NAME, UNDERWATER_SHADER_SOURCE};
pub use unit_counter_assets::{
    army_badge_progress_bars_from_resolver, counter_fonts_from_resolver,
    division_template_counter_layout_from_resolver,
    map_counter_code_textured_sprites_from_resolver, map_counter_sprites_from_resolver,
    nato_text_icon_sprites_from_resolver, on_map_division_counter_inventory_from_resolver,
    on_map_division_counter_sprites_from_resolver, on_map_ship_counter_sprites_from_resolver,
    parse_sprite_defs, unit_counter_assets_from_resolver, unit_counter_gui_layouts_from_resolver,
    unitview_counter_sprites_from_resolver, AtlasFrame, BitmapFontBinding, CounterSpriteAsset,
    CounterSpriteInventory, DivisionTemplateCounterLayout, GuiContainerLayout, GuiControlLayout,
    GuiPoint, GuiSize, PixelRect, ProgressBarAsset, RawSpriteDef, UnitCounterAssetManifest,
    UnitCounterGuiLayouts, UnresolvedSpriteAsset, UvRect, NATO_TEXT_ICON_TEXTURE_PREFIX,
    ON_MAP_DIVISION_SYMBOL_TEXTURE_PREFIX, ON_MAP_SHIP_SYMBOL_TEXTURE_PREFIX,
    UNIT_COUNTER_CORE_GFX_SOURCE, UNIT_COUNTER_DIVISION_DESIGNER_GUI_SOURCE,
    UNIT_COUNTER_GAMEPLAY_DEFINES_SOURCE, UNIT_COUNTER_GRAPHICS_DEFINES_SOURCE,
    UNIT_COUNTER_MAPICONS_GFX_SOURCE, UNIT_COUNTER_MAPICONS_GUI_SOURCE,
    UNIT_COUNTER_SUBUNITICONS_GFX_SOURCE, UNIT_COUNTER_TEXTICONS_GFX_SOURCE,
    UNIT_COUNTER_UNITVIEW_GFX_SOURCE, UNIT_COUNTER_UNITVIEW_GUI_SOURCE,
};
pub use unit_placement::{
    place_division, place_divisions, place_fleet, place_fleets, place_frame_inputs, place_wing,
    place_wings, wing_placement_input_from_world_row, AnchorProvenance, AnchorProvinceSource,
    DivisionPlacementInput, FleetPlacementInput, PlacementStatus, ProvinceAnchor,
    ProvinceAnchorTable, UnitKind, UnitPlacement, UnitPlacementSet, WingPlacementInput,
    WorldAnchor, UNIT_PLACEMENT_MAP_PLANE_X, UNIT_PLACEMENT_MAP_PLANE_Z, UNIT_STACK_BASE_SLOT_KIND,
};
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
    animation_curve_peak, apply_weather_distance_fog, asset_subsystem_count, cloud_law_from_asset,
    cloud_law_from_resolver, cloud_texture_rgba_from_resolver, entity_default_state_events,
    entity_definition_count, entity_particle_name, entity_particle_names, evaluate_animation_curve,
    particle_asset_defines, particle_asset_definition_count, particle_asset_shader_effects,
    particle_asset_texture_files, pdxparticle_registration_count, pdxparticle_type,
    weather_distance_fog_defines_from_source, weather_distance_fog_factor,
    weather_effects_from_resolver, weather_effects_from_source,
    weather_emitters_from_resolver, weather_emitters_from_source,
    weather_fade_defines_from_resolver, weather_fade_defines_from_source,
    derive_region_weather_membership, weather_epoch_join_law,
    weather_membership_producer_instance_id, weather_selected_candidate_index,
    weather_selection_word, weather_type_for_airborne_code, weather_visibility_law,
    CloudRenderLaw, DerivedRegionWeatherMembership, DerivedWeatherPositionSelection, ParticleBlend,
    WeatherAuthoredEvent, WeatherAuthoredParticleEffect,
    WeatherDistanceFogDefines, WeatherEffectBinding, WeatherEffectsTable, WeatherEmitter,
    WeatherEmitterTable, WeatherEntityStateEvents, WeatherFadeDefines, WeatherPositionSize,
    WeatherType, WeatherVisibilityLaw, FOG_CONSTANTS_SOURCE,
    GRAPHICS_DEFINES_SOURCE, MAP_PLANE_X, MAP_PLANE_Z, PARTICLES_GFX_SOURCE,
    PARTICLE_SHADER_SOURCE, TO_LINEAR_GAMMA, VANILLA_WEATHER_DISTANCE_FOG,
    VANILLA_WEATHER_FADE_DEFINES, WEATHER_EFFECTS_SOURCE, WEATHER_ENTITIES_SOURCE,
    WEATHER_POSITIONS_SOURCE,
};
pub use xmap04_d3d_handoff::{
    xmap04_underwater_color_descriptor_sha256, xmap04_underwater_depth_descriptor_sha256,
    xmap04_underwater_producer_allocation_identity_sha256,
    xmap04_underwater_resource_identity_sha256, xmap04_underwater_rtv_descriptor_sha256,
    xmap04_underwater_srv_descriptor_sha256, xmap04_underwater_texture_descriptor_sha256,
    Xmap04D3dHandoffReceipt, Xmap04NativeMapShaderResourceBinding, Xmap04TerrainDrawReceipt,
    Xmap04UnderwaterDrawReceipt, Xmap04UnderwaterTargetContract,
    XMAP04_TERRAIN_DRAW_RECEIPT_SCHEMA,
    XMAP04_WATER_REFRACTION_PRODUCER_ALLOCATION_IDENTITY_DOMAIN,
    XMAP04_WATER_REFRACTION_RESOURCE_IDENTITY_DOMAIN,
};
#[cfg(windows)]
pub use xmap04_d3d_handoff::{
    NativeXmap04MapShaderResourceView, NativeXmap04ShaderResourceView, NativeXmap04TerrainHandoff,
    NativeXmap04UnderwaterPointLightsConstantBuffer, NativeXmap04UnderwaterSceneConstantBuffer,
    NativeXmap04UnderwaterShaderResource, NativeXmap04UnderwaterTarget,
};
pub use xmap05_d3d_handoff::Xmap05D3dHandoffReceipt;
#[cfg(windows)]
pub use xmap05_d3d_handoff::{NativeXmap05PoliticalHandoff, NativeXmap05ShaderResource};
#[cfg(windows)]
pub use xmap06_d3d_handoff::NativeXmap06ThinBorderHandoff;
pub use xmap06_d3d_handoff::{
    Xmap06BorderBucketBank, Xmap06BorderDdsDescriptor, Xmap06BorderDdsMipDescriptor,
    Xmap06BorderDrawBucket, Xmap06BorderDrawPhase, Xmap06BorderExecutionPhase,
    Xmap06D3dHandoffReceipt, Xmap06PhaseDrawReceipt, Xmap06PhaseExecutionIdentity,
    XMAP06_BORDER_CB2_BYTE_WIDTH, XMAP06_BORDER_DRAW_PLAN, XMAP06_BORDER_INDEX_FORMAT_DXGI,
    XMAP06_BORDER_VERTEX_STRIDE_BYTES, XMAP06_PHASE_DRAW_RECEIPT_SCHEMA,
};
#[cfg(windows)]
pub use xmap07_d3d_handoff::{
    NativeXmap07DependentResourceView, NativeXmap07PointLightsConstantBufferView,
    NativeXmap07RefractionTargetView, NativeXmap07ResourceView,
    NativeXmap07SceneConstantBufferView, NativeXmap07SurfaceDrawInputs, NativeXmap07WaterHandoff,
};
pub use xmap07_d3d_handoff::{
    NativeXmap07ShaderResourceBinding, Xmap07D3dHandoffReceipt, Xmap07SurfaceDrawReceipt,
};
pub use xmap08_d3d_handoff::{
    CompositionComponentDisposition, Xmap08ComponentClosure, Xmap08D3dHandoffReceipt,
    Xmap08ExecutionFrameAuthority, Xmap08RiverShadowMatrixBinding, Xmap08RiverTemporalBinding,
    Xmap08TargetBoundDrawReceipt,
};
#[cfg(windows)]
pub use xmap08_d3d_handoff::{
    NativeXmap08RiverProviderInputs, NativeXmap08RiverShaderResource,
    NativeXmap08RiversStraitsHandoff, NativeXmap08SceneConstantBuffer,
};
#[cfg(windows)]
pub use xmap09_d3d_handoff::{
    NativeXmap09ConstantBufferView, NativeXmap09LightDepthCaster, NativeXmap09LightingHandoff,
    NativeXmap09ShaderResourceView,
};
pub use xmap09_d3d_handoff::{
    Xmap09D3dHandoffReceipt, Xmap09LightDepthExecutionReceipt, Xmap09NativeConstantBufferBinding,
    Xmap09NativeShaderResourceBinding,
};
#[cfg(windows)]
pub use xmap10_d3d_handoff::{
    xmap10_final_rtv_resource_identity_sha256, NativeXmap10PostFxExecutor,
    NativeXmap10ScreenShadowHandoff, NativeXmap10ShaderResourceView,
};
pub use xmap10_d3d_handoff::{
    xmap10_postfx_assigned_stage_topology, Xmap10AttachedFinalTarget, Xmap10CompletedPostFxFrame,
    Xmap10LuminanceDownsampleChainReceipt, Xmap10LuminanceDownsampleSubpassReceipt,
    Xmap10LuminanceReductionPlan, Xmap10LutExecutionReceipt, Xmap10LutPassExecutionReceipt,
    Xmap10NativeShaderResourceBinding, Xmap10PostFxExecutionInput, Xmap10PostFxFrameReceipt,
    Xmap10ReductionStep, Xmap10ScreenShadowExecutionReceipt, Xmap10TargetBoundPassReceipt,
    Xmap10TemporalExecutionInput, Xmap10TemporalPolicy,
    XMAP10_LUMINANCE_DOWNSAMPLE_CHAIN_RECEIPT_SCHEMA,
    XMAP10_LUMINANCE_DOWNSAMPLE_SUBPASS_RECEIPT_SCHEMA, XMAP10_LUMINANCE_REDUCTION_EVIDENCE_SHA256,
    XMAP10_POSTFX_FRAME_RECEIPT_SCHEMA, XMAP10_REDUCTION_PLAN_SCHEMA,
    XMAP10_SCREEN_SHADOW_RECEIPT_SCHEMA, XMAP10_TARGET_BOUND_PASS_RECEIPT_SCHEMA,
    XMAP10_TEMPORAL_POLICY_SCHEMA,
};
pub use xmap10_scene_cb0::{
    compose_xmap10_scene_cb0, xmap10_main_scene_target_sha256, xmap10_scene_cb0_layout_sha256,
    xmap10_scene_cb0_reflected_layout_sha256, xmap10_scene_cb0_stage_reflected_layout_sha256,
    PreparedXmap10SceneConstantBuffer, Xmap03CameraScenePatch, Xmap05PoliticalFowScenePatch,
    Xmap07SnowMudScenePatch, Xmap09LightingScenePatch, Xmap10PreParticleSceneReceipt,
    Xmap10PresentationRecurrenceStep, Xmap10PresentationScenePatch, Xmap10SceneCompositionInput,
    Xmap10SceneConstantBufferDescriptor, Xmap10SceneFieldCoverage, Xmap10SceneFieldOwner,
    Xmap10SceneFrameIdentity, Xmap10ScenePatchHashes, Xmap10ViewerPresentationState,
    XMAP10_PRE_PARTICLE_OPERATION_ID, XMAP10_PRE_PARTICLE_SCENE_SCHEMA,
    XMAP10_SCENE_CB0_BIND_POINT, XMAP10_SCENE_CB0_BYTE_LEN, XMAP10_SCENE_CB0_LAYOUT_SOURCE,
    XMAP10_SCENE_CB0_SCHEMA, XMAP10_SCENE_FIELD_COVERAGE, XMAP10_VIEWER_PRESENTATION_STATE_SCHEMA,
};
#[cfg(windows)]
pub use xmap10_scene_cb0::{NativeXmap10MapCopySceneCb0View, NativeXmap10SceneConstantBuffer};
#[cfg(windows)]
pub use xmap11_d3d_handoff::{
    execute_xmap11_tree_frame, NativeTreeAssetResources, NativeTreeMainSceneBindings,
    NativeTreePartitionSceneBinding, NativeTreeShaderResource, NativeXmap11LightDepthCaster,
    NativeXmap11ResourceOwner,
};
pub use production_features::xmap11::{
    xmap11_foundation_authorization_sha256, xmap11_foundation_disposition,
};
pub use xmap11_d3d_handoff::{
    TreeGpuResourceIdentity, TreeNativeExecutionAuthority, TreeNativeExecutionReceipt,
    XMAP11_TREE_NATIVE_EXECUTION_SCHEMA,
};
#[cfg(windows)]
pub use xmap16_d3d_handoff::{
    NativeXmap16MainSceneTargetView, NativeXmap16RailDrawInputs,
    NativeXmap16RailExternalResourceView, NativeXmap16RailHandoff,
    NativeXmap16RailSceneConstantBufferView, NativeXmap16RailTargetView,
};
pub use xmap16_d3d_handoff::{
    Xmap16RailComponentClosure, Xmap16RailComponentDisposition, Xmap16RailDiagnosticDisposition,
    Xmap16RailExecutionReceipt, Xmap16RailExternalResourceIdentity, Xmap16RailHandoffReceipt,
    Xmap16RailPipelineStateContract, Xmap16RailSharedExecutionIdentity,
    Xmap16RailTargetBoundDrawReceipt, Xmap16RailTargetIdentity, XMAP16_EXTERNAL_PIXEL_RESOURCES,
    XMAP16_RAIL_DIAGNOSTIC_MANIFEST_SHA256, XMAP16_RAIL_OPERATION_ID, XMAP16_RAIL_PIPELINE_STATE,
    XMAP16_RAIL_STAGE_NAME,
};
pub use xmap16_hub_handoff::{
    prepare_xmap16_hub_gui_packet, D18Xmap16HubViewerSelectionProvider, PreparedXmap16HubGuiPacket,
    Xmap16HubExecutionReceipt, Xmap16HubGuiLayer, Xmap16HubGuiQuad, Xmap16HubGuiVertex,
    Xmap16HubPresentationMode, Xmap16HubPresentationModeProvider, Xmap16HubTextureDescriptor,
    Xmap16HubTextureRole, Xmap16HubUploadReceipt, Xmap16HubViewerSelection,
    Xmap16HubViewerSelectionProvider, XMAP16_HUB_EXECUTION_RECEIPT_SCHEMA,
    XMAP16_HUB_GUI_PACKET_SCHEMA, XMAP16_HUB_UPLOAD_RECEIPT_SCHEMA,
};
#[cfg(windows)]
pub use xmap16_hub_handoff::{
    NativeXmap16HubGuiExecutor, NativeXmap16HubHandoff, NativeXmap16HubTextureSet,
};
pub use xmap16_supply_arrow_handoff::{
    build_xmap16_supply_arrow_cb3, prepare_xmap16_supply_arrow_definitions,
    prepare_xmap16_supply_arrow_program, PreparedXmap16SupplyArrowDefinition,
    PreparedXmap16SupplyArrowProgram, PreparedXmap16SupplyArrowStage,
    Xmap16SupplyArrowBlendContract, Xmap16SupplyArrowCb3SemanticInput, Xmap16SupplyArrowCb3Write,
    Xmap16SupplyArrowDepthStencilContract, Xmap16SupplyArrowDrawReceipt,
    Xmap16SupplyArrowHandoffReceipt, Xmap16SupplyArrowMeshUploadReceipt, Xmap16SupplyArrowPass,
    Xmap16SupplyArrowResourceIdentity, Xmap16SupplyArrowScheduleAssignment,
    Xmap16SupplyArrowTargetIdentity, Xmap16SupplyArrowTextureUploadReceipt,
    XMAP16_SUPPLY_ARROW_CB3_BYTES, XMAP16_SUPPLY_ARROW_CB3_LAYOUT,
    XMAP16_SUPPLY_ARROW_CONSTANT_BUFFERS, XMAP16_SUPPLY_ARROW_D3D11_TOPOLOGY,
    XMAP16_SUPPLY_ARROW_DEFINITION_NAMES, XMAP16_SUPPLY_ARROW_DRAW_BLOCKERS,
    XMAP16_SUPPLY_ARROW_NORMAL_BLEND, XMAP16_SUPPLY_ARROW_NORMAL_DEPTH_STENCIL,
    XMAP16_SUPPLY_ARROW_NORMAL_EFFECT, XMAP16_SUPPLY_ARROW_OPERATION_ID,
    XMAP16_SUPPLY_ARROW_OPERATION_NAME, XMAP16_SUPPLY_ARROW_PRIMITIVE_TOPOLOGY,
    XMAP16_SUPPLY_ARROW_REFLECTED_TEXTURE_RESOURCES, XMAP16_SUPPLY_ARROW_SEETHROUGH_BLEND,
    XMAP16_SUPPLY_ARROW_SEETHROUGH_DEPTH_STENCIL, XMAP16_SUPPLY_ARROW_SEETHROUGH_EFFECT,
    XMAP16_SUPPLY_ARROW_SHADER_SOURCE, XMAP16_SUPPLY_ARROW_TEXTURE_RESOURCES,
    XMAP16_SUPPLY_ARROW_VERTEX_STRIDE_BYTES,
};
#[cfg(windows)]
pub use xmap16_supply_arrow_handoff::{
    NativeXmap16ExternalConstantBufferView, NativeXmap16ExternalResourceView,
    NativeXmap16SceneConstantBufferView, NativeXmap16SupplyArrowDrawInputs,
    NativeXmap16SupplyArrowHandoff,
};
