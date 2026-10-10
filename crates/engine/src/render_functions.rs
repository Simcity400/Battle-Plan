//! Render routine sites and associated globals with unestablished owner types.
//! These are site metadata; separate receivers cannot be assumed to alias.
//! No hook or table is enabled here.

/// Map-effect registration root.
pub const MAP_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0126_1100;
/// Exact map-effect registration entry sentinel.
pub const MAP_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Railroad-effect registration root.
pub const RAILROAD_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0159_B9B0;
/// Exact railroad-effect registration entry sentinel.
pub const RAILROAD_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x4c,
];
/// Map-name effect registration root.
pub const MAP_NAME_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x00B4_0890;
/// Exact map-name effect registration entry sentinel.
pub const MAP_NAME_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x18, 0x48, 0x89, 0x48, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54,
];
/// Bloom-effect registration root.
pub const RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x00FF_1410;
/// Exact bloom-effect registration entry sentinel.
pub const RESTORE_BLOOM_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// LUT-blend effect registration root.
pub const LUT_BLEND_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0126_F590;
/// Exact LUT-blend registration entry sentinel.
pub const LUT_BLEND_EFFECT_REGISTRATION_ROOT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x55,
];
/// Pdxmesh effect registration root.
pub const PDXMESH_EFFECT_REGISTRATION_ROOT_RVA: u32 = 0x0224_BA50;
/// Post-processing render root.
pub const POST_PROCESS_RENDER_ROOT_RVA: u32 = 0x00FF_3450;
/// Exact post-processing entry sentinel.
pub const POST_PROCESS_RENDER_ROOT_SENTINEL: [u8; 16] = [
    0x4c, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54,
];
/// Weather normal update routine.
pub const WEATHER_NORMAL_UPDATE_OWNER_RVA: u32 = 0x0166_CEC0;
/// Exact weather normal update entry sentinel.
pub const WEATHER_NORMAL_UPDATE_OWNER_SENTINEL: [u8; 16] = [
    0x40, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x50, 0x48, 0x89, 0x5c, 0x24, 0x78, 0x4d,
];
/// Weather editor refresh routine.
pub const WEATHER_EDITOR_REFRESH_OWNER_RVA: u32 = 0x01B7_8FB0;
/// Exact weather editor refresh entry sentinel.
pub const WEATHER_EDITOR_REFRESH_OWNER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55,
];
/// Texture resolver trying DDS before TGA.
pub const TEXTURE_EXTENSION_RESOLVER_RVA: usize = 0x023f_3320;
/// GUI 2D tree render routine.
pub const GUI_RENDER_2D_TREE_RVA: usize = 0x0222_84b0;
/// Exact GUI tree-render entry sentinel.
pub const GUI_RENDER_2D_TREE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Particle definition lookup.
pub const PARTICLE_DEFINITION_LOOKUP_RVA: usize = 0x0228_24a0;
/// Particle runtime registry routine.
pub const PARTICLE_RUNTIME_REGISTRY_RVA: usize = 0x023e_8590;
/// Tree instance producer.
pub const TREE_INSTANCE_PRODUCER_RVA: usize = 0x01b1_1a70;
/// Buffer-factory call instruction inside the tree producer.
pub const TREE_BUFFER_FACTORY_CALLSITE_RVA: usize = 0x01b1_21e9;
/// Map preparation flushes registered objects before lights.
pub const MAP_PREPARATION_RVA: usize = 0x0126_89e0;
/// Point-light gather runs last in map preparation.
pub const MAP_POINT_LIGHT_GATHER_RVA: usize = 0x0126_bbd0;
/// Paradrop terrain callback omits ordinary arrow offsets.
pub const PARADROP_HEIGHT_CALLBACK_RVA: usize = 0x0123_fd20;
/// Ordinary arrow terrain sampler applies half-cell and height offsets.
pub const ARROW_TERRAIN_SAMPLER_RVA: usize = 0x0125_2f00;
/// Integer-root helper narrows to i32 before sign extension.
pub const FIXED_POINT_VECTOR_NORM_RVA: usize = 0x024e_6e90;
/// Arrow strip builder fills backward, positive side first.
pub const ARROW_STRIP_BUILDER_RVA: usize = 0x0125_5d70;
/// Rail arrow geometry routine.
pub const RAIL_ARROW_GEOMETRY_BUILDER_RVA: usize = 0x0125_1430;
/// First sine variant; variants differ in rounding.
pub const NATIVE_SINE_VARIANT_A_RVA: usize = 0x0253_e310;
/// Second sine variant; FMA use is not established.
pub const NATIVE_SINE_VARIANT_B_RVA: usize = 0x0254_9390;
/// Tangent for camera half-FOV without argument reduction.
pub const NATIVE_TANGENT_RVA: usize = 0x0255_92c0;
/// Stack-kind validation routine.
pub const STACK_KIND_VALIDATE_RVA: usize = 0x0127_b760;
/// Maximum admitted stack kind.
pub const STACK_KIND_MAX: u32 = 0x25;
/// Mirrored-view update caller using a stack-local record.
pub const MIRRORED_CAMERA_CALLER_RVA: usize = 0x0126_90c0;
/// Map-frame root.
pub const MAP_FRAME_ROOT_RVA: usize = 0x00b2_e730;
/// Map pre-frame preparation.
pub const MAP_PRE_FRAME_RVA: usize = 0x00b4_7710;
/// Map post pass.
pub const MAP_POST_PASS_RVA: usize = 0x00b4_76b0;
/// First scene-manager entity pass.
pub const MAP_ENTITY_PASS_A_RVA: usize = 0x0222_8380;
/// Second scene-manager entity pass.
pub const MAP_ENTITY_PASS_B_RVA: usize = 0x0222_8970;
/// Arrow forwarding routine for pass mode 2.
pub const MAP_ARROW_SHADOW_PASS_RVA: usize = 0x0125_c690;
/// Arrow forwarding routine for pass mode 3.
pub const MAP_ARROW_MODE_THREE_PASS_RVA: usize = 0x0125_c630;
/// Arrow forwarding routine for pass mode 1.
pub const MAP_ARROW_MODE_ONE_PASS_RVA: usize = 0x0125_c5d0;
/// Arrow main pass for mode 0.
pub const MAP_ARROW_MAIN_PASS_RVA: usize = 0x0125_c3c0;
/// Arrow draw consuming pass mode as its tenth argument.
pub const MAP_ARROW_DRAW_RVA: usize = 0x0125_acb0;
/// Scene-constant copy endpoint.
pub const SNOW_MUD_SCENE_COPY_RVA: usize = 0x00ff_3fc0;
/// Occupation secondary-texture raster worker.
pub const OCCUPATION_SECONDARY_RASTER_WORKER_RVA: usize = 0x019c_8a10;
/// Asset-factory loader using the entity-description reader and both definition consumers.
pub const ENTITY_ASSET_LOADER_RVA: usize = 0x0233_8c30;
/// Shader cache reader validating .scache before its paired .bin.
pub const SHADER_CACHE_READER_RVA: usize = 0x023f_9f80;
/// Shader cache writer.
pub const SHADER_CACHE_WRITER_RVA: usize = 0x023f_a950;
/// Shadow-band threshold compared with camera +0x194.
pub const SHADOW_BAND_LIMIT_RVA: usize = 0x0331_99c0;
/// Mirror-plane height global.
pub const MIRROR_PLANE_HEIGHT_RVA: usize = 0x0292_71d4;

/// Connected-region border builder.
pub const BUILD_CONNECTED_REGION_BORDERS_RVA: usize = 0x01b0_b700;
/// Height-volume reader ordering helper, sorting heights ascending.
pub const POST_EFFECT_HEIGHT_ORDER_A_RVA: usize = 0x0126_ec30;
/// Companion height-volume ordering helper in the same initialization path.
pub const POST_EFFECT_HEIGHT_ORDER_B_RVA: usize = 0x0126_f030;
/// Grading-volume selector using the first positive region factor.
pub const POST_EFFECT_VOLUME_SELECT_RVA: usize = 0x0127_48e0;
/// Grade resolution blending the height pair before optional region blending.
pub const POST_EFFECT_GRADE_RESOLVE_RVA: usize = 0x0127_1150;
/// Grade endpoint and factor composition.
pub const POST_EFFECT_GRADE_COMPOSE_RVA: usize = 0x0127_0d90;
/// Region coordinates and weight-row construction.
pub const POST_EFFECT_REGION_WEIGHTS_RVA: usize = 0x0127_2380;
/// Region snow weighting; zero-row regions perform no province lookup.
pub const POST_EFFECT_REGION_SNOW_WEIGHT_RVA: usize = 0x0127_4af0;
/// Float define loader accepting Lua numeric type 3 and converting its value.
pub const LUA_FLOAT_DEFINE_LOADER_RVA: usize = 0x0205_b9e0;
/// Define initialization for HDR adjustment speed.
pub const INITIALIZE_HDR_ADJUSTMENT_DEFINE_RVA: usize = 0x0084_b080;
/// Luminance gather setup passing tau times delta to its constants.
pub const LUMINANCE_GATHER_SETUP_RVA: usize = 0x00ff_2890;
/// Post-effect allocation clearing both current and history targets.
pub const ALLOCATE_POST_EFFECT_TARGETS_RVA: usize = 0x00ff_2410;
/// Frame cadence converting seconds to elapsed frames at 30 Hz.
pub const FRAME_CADENCE_CONSUMER_RVA: usize = 0x0123_b4f0;
/// Seconds-to-milliseconds f32 global.
pub const FRAME_SECONDS_TO_MILLISECONDS_RVA: usize = 0x0272_0bb0;
/// Thirty-Hz frame-period f32 global.
pub const FRAME_PERIOD_THIRTY_HZ_RVA: usize = 0x0298_e854;
/// Strategic-air map-mode producer using observer-owned air-map data.
pub const BUILD_STRATEGIC_AIR_MAP_MODE_RVA: usize = 0x00df_f6e0;
/// River mesh generation helper producing the retained vertex buffer.
pub const RIVER_MESH_BUILD_A_RVA: usize = 0x01b0_f440;
/// Companion river mesh generation helper.
pub const RIVER_MESH_BUILD_B_RVA: usize = 0x01b0_fc70;
/// Map shader-constant derivation.
pub const DERIVE_MAP_SHADER_CONSTANTS_RVA: usize = 0x00a4_dde0;
/// Supply-arrow color construction caller initializing every color component to one.
pub const CONSTRUCT_SUPPLY_ARROW_COLOR_RVA: usize = 0x0125_55a0;

/// Shadow matrix fit.
pub const SHADOW_FIT_RVA: usize = 0x022c_4f30;
/// Shadow screen chain.
pub const SHADOW_SCREEN_CHAIN_RVA: usize = 0x022c_5d30;
/// Shadow blur-factor routine.
pub const SHADOW_BLUR_FACTOR_RVA: usize = 0x022c_6bc0;
/// Vertex-buffer allocation callback receiving element count and byte stride.
pub const VERTEX_BUFFER_ALLOCATE_CALLBACK_RVA: usize = 0x030a_77b8;
/// Vertex-buffer constructor with byte width equal to count times stride.
pub const CREATE_VERTEX_BUFFER_RVA: usize = 0x023f_d780;
/// Air-base anchor resolver using building-position match or fallback.
pub const AIR_BASE_ANCHOR_RESOLVER_RVA: usize = 0x018d_a830;
/// Counter define normalization f32 global, also used by whole-percent
/// building-health thresholds.
pub const COUNTER_FIXED_POINT_SCALE_RVA: usize = 0x0270_9c48;

/// Model local-ray matrix decomposition yielding translation, rotation
/// and reciprocal scale.
pub const MODEL_LOCAL_RAY_DECOMPOSITION_RVA: usize = 0x0226_1a90;
/// Ray/axis-aligned-bounds slab-interval predicate.
pub const RAY_AXIS_ALIGNED_BOUNDS_RVA: usize = 0x024e_6270;
/// Direct model picker skipping the model test for class one.
pub const DIRECT_MODEL_PICKER_RVA: usize = 0x00b4_50e0;

/// Shared geometry f32 unity global, also used to clamp building-slot
/// popup scale at +0x40.
pub const GEOMETRY_UNITY_RVA: usize = 0x0270_e270;
/// Negative determinant-epsilon f32 global.
pub const GEOMETRY_NEGATIVE_DETERMINANT_EPSILON_RVA: usize = 0x02b2_9ea0;
/// Positive determinant-epsilon f32 global.
pub const GEOMETRY_POSITIVE_DETERMINANT_EPSILON_RVA: usize = 0x0293_4a78;

/// Joint loader retaining source order and scaling only translation.
pub const SKELETON_SOURCE_LOADER_RVA: usize = 0x0225_0010;
/// Bind-transform expansion before skinning.
pub const ANIMATION_PALETTE_EVALUATE_RVA: usize = 0x0239_9990;
/// Inverse helper following three copies of the first transform row,
/// including the singular radius-center path.
pub const BIND_RADIUS_MATRIX_INVERSE_RVA: usize = 0x024e_7ba0;
/// Animation-object constructor; allocation does not establish a rest palette.
pub const ANIMATION_OBJECT_CONSTRUCTOR_RVA: usize = 0x0239_9610;
/// Clip playback evaluates the first clip immediately; replacement of an
/// active clip retains bounds until update.
pub const ANIMATION_PLAY_RVA: usize = 0x0239_b250;
/// Instruction site in the blended-translation scale branch.
pub const ANIMATION_BLEND_TRANSLATION_SCALE_SITE_RVA: usize = 0x0239_a015;
/// Arccosine path after slerp sign correction.
pub const NATIVE_POSITIVE_ACOS_RVA: usize = 0x0254_e038;
/// Appends initial tracks before update tracks to one evaluator list.
pub const PARTICLE_EVALUATOR_TRACK_APPEND_RVA: usize = 0x024e_e010;
/// Decimal conversion retaining fractional digits through wrapping integer
/// intermediates before fixed-1e5 conversion.
pub const VARIED_FIXED_DECIMAL_PARSE_RVA: usize = 0x024b_8600;
/// Attachment-weight conversion sign-extending the total before unsigned
/// i64-to-double conversion.
pub const ENTITY_ATTACHMENT_WEIGHTED_SELECT_RVA: usize = 0x024d_b730;
/// First state-weight selector using signed-positive 64-bit sums and
/// a uniform zero-total fallback.
pub const ENTITY_STATE_WEIGHTED_SELECT_A_RVA: usize = 0x024d_b310;
/// Second state-weight selector using signed-positive 64-bit sums and
/// a uniform zero-total fallback.
pub const ENTITY_STATE_WEIGHTED_SELECT_B_RVA: usize = 0x024d_b8f0;
/// A positive requested range replaces the authored range under bit 4.
pub const ENTITY_STATE_ENTER_RVA: usize = 0x0227_55a0;
/// Initializes both shared random words once.
pub const ENTITY_MANAGER_CONSTRUCTOR_RVA: usize = 0x0226_c380;
/// Enqueue supplies camera index as pass; scene zero has no camera and
/// cannot enqueue.
pub const ENTITY_MANAGER_CAMERA_DISPATCH_RVA: usize = 0x0227_ce00;
/// The update mask does not suppress enqueue. This routine also creates
/// animation-event lights.
pub const ENTITY_EVENT_ENQUEUE_RVA: usize = 0x0226_ef30;
/// Keeps time-slice remainder in f64 with an f32 cap and callback.
pub const PARTICLE_PHASE_SLICE_RVA: usize = 0x023e_9500;
/// Compacts particle registry survivors without a tail swap.
pub const LIGHT_INSTANCE_DESTROY_RVA: usize = 0x023c_9c90;
/// Evaluates end before delay and flips the delay sign bit.
pub const PARTICLE_SUBSYSTEM_INITIALIZE_RVA: usize = 0x023d_cc80;
/// Runs two gap passes with a strict end bound; a full sort is not established.
pub const PARTICLE_GAP_ORDER_RVA: usize = 0x023d_fd30;
/// Returns before initializing an overlapping trail-birth batch while
/// the caller still advances counts.
pub const PARTICLE_TRAIL_BIRTH_RVA: usize = 0x023e_5cd0;
/// Advances instance noise tables once per retained subsystem.
pub const PARTICLE_SUBSYSTEM_UPDATE_RVA: usize = 0x023d_daa0;
/// Reads completion before global particle time slices advance. This
/// routine also creates animation-event lights.
pub const ENTITY_PARTICLE_FINISHED_READ_RVA: usize = 0x0227_6040;
/// Refreshes eligibility after animation playback.
pub const ENTITY_ANIMATION_ELIGIBILITY_REFRESH_RVA: usize = 0x0226_77f0;
/// Forced state exit precedes static-child destruction.
pub const ENTITY_RECURSIVE_DESTRUCTION_RVA: usize = 0x0226_c910;
/// Requested-state setter follows complete recursive construction on
/// the same random cursor.
pub const PLACED_ENTITY_REQUESTED_STATE_RVA: usize = 0x0127_6c20;
/// Instruction site passing a null state packet.
pub const ENTITY_AUTOMATIC_NEXT_STATE_SITE_RVA: usize = 0x0227_074a;
/// Null-packet automatic next-state selection bypasses the pointer/hash
/// equality skip.
pub const ENTITY_STATE_PACKET_SETTER_RVA: usize = 0x0227_b5e0;
/// Returns a render-frame stamp even if a later camera rejects the sphere.
pub const RETAINED_ENTITY_CULL_RVA: usize = 0x022c_24d0;
/// Updates animation objects before entity dispatch can advance entities
/// or play automatic next-state clips.
pub const ANIMATION_MANAGER_UPDATE_RVA: usize = 0x0221_9c10;
/// Advances clip playhead at authored speed times render delta; also
/// creates animation-event lights.
pub const ENTITY_ANIMATION_EVENT_ADVANCE_RVA: usize = 0x0227_87f0;
/// Enabled-light update advances clocks, evaluates nine value tracks
/// and applies optional attachment transforms.
pub const LIGHT_INSTANCES_UPDATE_RVA: usize = 0x023c_a2c0;
/// Prepares scalar-curve inputs and a fallback base value, then delegates
/// evaluation.
pub const VARIED_VALUE_PREPARE_INPUTS_AND_EVALUATE_RVA: usize = 0x024e_e400;
/// Initializes strided outputs, holds or interpolates samples, then applies
/// addition, multiplication or replacement.
pub const VARIED_VALUE_APPLY_SAMPLED_CURVES_RVA: usize = 0x024e_fc80;
/// Compiles 64 cubic samples with extrapolated endpoints and min/max remapping.
pub const ANIMATION_CURVE_TABLE_BUILD_RVA: usize = 0x024f_0720;
/// Read-only cell-size f32 global.
pub const TILED_POINT_LIGHT_CELL_SIZE_RVA: usize = 0x0299_0128;
/// Read-only inverse-cell-size f32 global.
pub const TILED_POINT_LIGHT_INVERSE_CELL_SIZE_RVA: usize = 0x0299_011c;
/// Read-only forward-anchor distance f32 global.
pub const TILED_POINT_LIGHT_ANCHOR_FORWARD_DISTANCE_RVA: usize = 0x0299_012c;
/// Global city-light gate byte read by the light constructor.
pub const CITY_POINT_LIGHT_GATE_RVA: usize = 0x0337_1b32;
/// Instruction site reading the city-light gate byte.
pub const CITY_POINT_LIGHT_GATE_READ_SITE_RVA: usize = 0x0164_03d4;
/// Light constructor receiving the animation creator entity scene mask.
pub const POINT_LIGHT_INSTANCE_CONSTRUCTOR_RVA: usize = 0x023c_95b0;
/// High-resolution timing helper in the weather variant seed chain.
pub const WEATHER_VARIANT_CLOCK_SAMPLE_RVA: usize = 0x020f_4690;

/// Straits draw endpoint accepting the +0x4968 pointee.
pub const STRAITS_DRAW_RVA: usize = 0x022c_0140;
/// Supply-flow province-color update endpoint whose receiver is resolved indirectly.
pub const SUPPLY_FLOW_COLOUR_UPDATE_RVA: usize = 0x0164_c300;
/// Order-edit preview after checking the accessor object chain.
pub const ORDER_EDIT_PREVIEW_DRAW_RVA: usize = 0x0101_afa0;
/// Airplane path/tangent diagnostic using map +0x718.
pub const AIRPLANE_DIAGNOSTIC_DRAW_RVA: usize = 0x0123_5a70;
/// Pathfinder projected debug-line endpoint.
pub const PATHFINDER_DIAGNOSTIC_DRAW_RVA: usize = 0x0123_f3c0;
/// Player-supply debug endpoint.
pub const PLAYER_SUPPLY_DIAGNOSTIC_DRAW_RVA: usize = 0x0166_7600;
/// Debug bounding-box endpoint accepting camera and transient geometry.
pub const MAP_BOUNDING_BOX_DRAW_RVA: usize = 0x0117_acd0;
/// Scene object-list debug endpoint using the map +0x10 pointee's +0xb8 object.
pub const SCENE_OBJECT_LIST_DIAGNOSTIC_DRAW_RVA: usize = 0x0127_1ab0;
/// Shadow-enable byte combined with rendering-options +0x13 and camera height.
pub const MAP_SHADOW_ENABLED_RVA: usize = 0x0306_e9d3;
/// Unlit-pass camera-height threshold f32, compared with camera +0x194.
pub const UNLIT_SHADOW_HEIGHT_LIMIT_RVA: usize = 0x0331_9af4;
/// Unlit shadow-pass gate byte.
pub const UNLIT_SHADOW_ENABLED_RVA: usize = 0x0306_e9d4;
/// Registered-object diagnostic byte A, combined with B using OR and also checked separately.
pub const REGISTERED_OBJECT_DIAGNOSTIC_A_RVA: usize = 0x0331_7391;
/// Registered-object diagnostic byte B, combined with A using OR and also checked separately.
pub const REGISTERED_OBJECT_DIAGNOSTIC_B_RVA: usize = 0x0331_7392;
/// Pathfinder diagnostic byte gating clearing of map +0x710, separate from the later draw.
pub const PATHFINDER_DIAGNOSTIC_ENABLED_RVA: usize = 0x0331_723f;
/// Player-supply diagnostic byte preceding count-gated processing.
pub const PLAYER_SUPPLY_DIAGNOSTIC_ENABLED_RVA: usize = 0x0331_7394;
/// Player-supply secondary diagnostic byte with additional runtime gates.
pub const PLAYER_SUPPLY_SECONDARY_DIAGNOSTIC_ENABLED_RVA: usize = 0x0331_7393;
/// Bounding-box draw byte.
pub const MAP_BOUNDING_BOX_ENABLED_RVA: usize = 0x0331_738e;
/// Partition-frustum byte checked independently for each partition.
pub const PARTITION_FRUSTUM_ENABLED_RVA: usize = 0x0306_e9d1;
/// Second-form partition-frustum byte also requiring the first-form toggle.
pub const PARTITION_FRUSTUM_SECOND_FORM_ENABLED_RVA: usize = 0x0306_e9d2;
/// Light-frustum diagnostic byte.
pub const LIGHT_FRUSTUM_DIAGNOSTIC_ENABLED_RVA: usize = 0x0332_32bd;
/// Scene object-list diagnostic byte.
pub const SCENE_OBJECT_LIST_DIAGNOSTIC_ENABLED_RVA: usize = 0x0331_738d;
/// Registered-map-object draw using the tree/map owner +0x5ce0 pointee.
pub const REGISTERED_MAP_OBJECT_DRAW_RVA: usize = 0x0159_afb0;
/// Registered-map-object debug draw using the same +0x5ce0 pointee.
pub const REGISTERED_MAP_OBJECT_DIAGNOSTIC_DRAW_RVA: usize = 0x0159_b430;

/// Tree-batch ordering helper accepting byte-range endpoints and comparing
/// the byte key before f32 distance.
pub const TREE_BATCH_ORDERING_RVA: usize = 0x0126_0a40;
/// Pdxmesh shader-effect lookup by name and suffix, with -1 failure handling.
pub const PDXMESH_EFFECT_LOOKUP_RVA: usize = 0x0224_fee0;

/// Tree-render enable byte; zero returns early. Its relationship to
/// `cmap_rendering_options::TREES` remains unestablished.
pub const TREE_RENDER_ENABLED_RVA: usize = 0x0306_e9d6;
/// Maximum-f32 global used for tree partition-distance exclusion.
/// `tree_caster_owner_layout::EXCLUDED_DISTANCE_F32_BITS` retains the
/// value; this constant locates the separate global.
pub const TREE_PARTITION_EXCLUDED_DISTANCE_RVA: usize = 0x027c_d248;
/// Fallback effect-handle global read on the non-skinned fallback path.
/// A -1 handle fails; the live handle's validity is not inferred.
pub const PDXMESH_FALLBACK_EFFECT_HANDLE_RVA: usize = 0x030a_5e64;

/// Border-bucket introsort compares the unsigned reason byte at row +2;
/// equal-reason order is not promised.
pub const SORT_BORDER_BUCKET_ROWS_RVA: usize = 0x0126_0670;

/// Province-selection border color define loader.
pub const BORDER_SELECTION_PROVINCE_DEFINE_REGISTRAR_RVA: u32 = 0x0083_b0c0;

/// State-selection border color define loader.
pub const BORDER_SELECTION_STATE_DEFINE_REGISTRAR_RVA: u32 = 0x0083_b900;

/// Supply-area selection border color define loader.
pub const BORDER_SELECTION_SUPPLY_AREA_DEFINE_REGISTRAR_RVA: u32 = 0x0083_c140;

/// Adjacency-rule area border color define loader.
pub const BORDER_SELECTION_ADJACENCY_RULE_AREA_DEFINE_REGISTRAR_RVA: u32 = 0x0083_a040;

/// Building-area selection border color define loader.
pub const BORDER_SELECTION_BUILDING_AREA_DEFINE_REGISTRAR_RVA: u32 = 0x0083_a880;

/// Tutorial-highlight border color define loader.
pub const BORDER_TUTORIAL_HIGHLIGHT_DEFINE_REGISTRAR_RVA: u32 = 0x0083_c980;

/// Demilitarized border color define loader.
pub const BORDER_DEMILITARIZED_DEFINE_REGISTRAR_RVA: u32 = 0x0083_9800;

/// Edge border-conflict color define loader.
pub const BORDER_CONFLICT_EDGE_DEFINE_REGISTRAR_RVA: u32 = 0x0083_8570;

/// Non-edge border-conflict color define loader.
pub const BORDER_CONFLICT_NON_EDGE_DEFINE_REGISTRAR_RVA: u32 = 0x0083_8db0;

/// Custom-highlight border color define loader.
pub const BORDER_CUSTOM_HIGHLIGHTS_DEFINE_REGISTRAR_RVA: u32 = 0x0083_8fc0;
