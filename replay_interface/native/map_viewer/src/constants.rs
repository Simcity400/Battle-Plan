use crate::*;

// Shader constants below are copied by symbol from shipped the game files and are
// validated against the generated map material contract before captured-frame
// replay. They are not tune knobs.

// gfx/FX/constants.fxh:53-56.
pub(crate) const TERRAIN_TILE_FREQ: f32 = 128.0;
pub(crate) const MAP_NUM_TILES: usize = 4;
pub(crate) const TEXELS_PER_TILE: f32 = 512.0;
pub(crate) const ATLAS_TEXEL_POW2_EXPONENT: f32 = 11.0;

// gfx/FX/constants.fxh:93-94.
pub(crate) const COLORMAP_OVERLAY_STRENGTH: f32 = 0.75;
pub(crate) const COLORMAP_MUD_OVERLAY_STRENGTH: f32 = 0.5;

// gfx/FX/constants.fxh:14.
pub(crate) const NIGHT_AMBIENT_BOOST: f32 = 3.0;

// gfx/FX/standardfuncsgfx.fxh:328-329.
pub(crate) const NIGHT_OPACITY: f32 = 0.85;
pub(crate) const NIGHT_DARKNESS: f32 = 0.5;

// common/defines/00_graphics.lua:934; also consumed by CalcGlobeNormal in
// gfx/FX/standardfuncsgfx.fxh.
pub(crate) const GMT_OFFSET: f32 = 2793.0;

// gfx/FX/standardfuncsgfx.fxh:324-325.
pub(crate) const FEATHER_MIN: f32 = -0.01;
pub(crate) const FEATHER_MAX: f32 = 0.01;

// gfx/FX/standardfuncsgfx.fxh:326-327 (moon-light cross-fade feather).
pub(crate) const MOON_FEATHER_MIN: f32 = -0.01;
pub(crate) const MOON_FEATHER_MAX: f32 = 0.01;

// common/defines/00_graphics.lua:936-937; mirrored in the day/night shader at
// gfx/FX/standardfuncsgfx.fxh:330-331.
pub(crate) const SOUTH_POLE_OFFSET: f32 = 0.17;
pub(crate) const NORTH_POLE_OFFSET: f32 = 0.93;

// gfx/FX/constants.fxh:61-63 and 69-70.
pub(crate) const MUD_TILING: f32 = 0.09;
// NOT in constants.fxh: the mud snow-noise tiling is the inline literal
// `tex2D( SnowNoise, vPos.xz * 0.01f )` in GetMudColor,
// gfx/FX/standardfuncsgfx.fxh:241 (the earlier constants.fxh:61-63 citation
// was wrong; corrected 2026-07-09).
pub(crate) const MUD_NOISE_TILING: f32 = 0.01;
pub(crate) const MUD_NORMAL_CUTOFF: f32 = 10.982;
pub(crate) const MUD_STRENGHTEN: f32 = 1.0;
pub(crate) const MUD_CAM_MIN: f32 = 50.0;
pub(crate) const MUD_CAM_MAX: f32 = 300.0;

// gfx/FX/constants.fxh:65-68 and 75-78.
pub(crate) const SNOW_OPACITY_MIN: f32 = 0.95;
pub(crate) const SNOW_OPACITY_MAX: f32 = 0.2;
pub(crate) const SNOW_CAM_MIN: f32 = 50.0;
pub(crate) const SNOW_CAM_MAX: f32 = 300.0;
pub(crate) const SNOW_START_HEIGHT: f32 = 3.0;
pub(crate) const SNOW_RIDGE_START_HEIGHT: f32 = 11.0;
pub(crate) const SNOW_NORMAL_START: f32 = 0.7;
pub(crate) const SNOW_COLOR: Vec3 = Vec3 {
    x: 0.46,
    y: 0.48,
    z: 0.69,
};

// gfx/FX/constants.fxh:17-22. These are shader ambient-map constants, distinct
// from 00_graphics.lua's virtual-light rig.
pub(crate) const DAY_AMBIENT_MAP_POS_X: Vec3 = Vec3 {
    x: 0.1,
    y: 0.1,
    z: 0.05,
};
pub(crate) const DAY_AMBIENT_MAP_NEG_X: Vec3 = Vec3 {
    x: 0.15,
    y: 0.15,
    z: 0.15,
};
pub(crate) const DAY_AMBIENT_MAP_POS_Y: Vec3 = Vec3 {
    x: 0.03,
    y: 0.03,
    z: 0.06,
};
pub(crate) const DAY_AMBIENT_MAP_NEG_Y: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};
pub(crate) const DAY_AMBIENT_MAP_POS_Z: Vec3 = Vec3 {
    x: 0.0502,
    y: 0.05023,
    z: 0.1023,
};
pub(crate) const DAY_AMBIENT_MAP_NEG_Z: Vec3 = Vec3 {
    x: 0.03,
    y: 0.033,
    z: 0.033,
};

// gfx/FX/constants.fxh:24-29. These are the NightAmbientMap* constants, not the
// stronger NightAmbient* rig at constants.fxh:32-37.
pub(crate) const NIGHT_AMBIENT_MAP_POS_X: Vec3 = Vec3 {
    x: 0.2,
    y: 0.2,
    z: 0.2,
};
pub(crate) const NIGHT_AMBIENT_MAP_NEG_X: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: 0.0,
};
pub(crate) const NIGHT_AMBIENT_MAP_POS_Y: Vec3 = Vec3 {
    x: 0.01,
    y: 0.01,
    z: 0.01,
};
pub(crate) const NIGHT_AMBIENT_MAP_NEG_Y: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: 0.1,
};
pub(crate) const NIGHT_AMBIENT_MAP_POS_Z: Vec3 = Vec3 {
    x: 0.06,
    y: 0.1,
    z: 0.15,
};
pub(crate) const NIGHT_AMBIENT_MAP_NEG_Z: Vec3 = Vec3 {
    x: 0.14,
    y: 0.14,
    z: 0.2,
};

// gfx/FX/constants.fxh:80-85.
pub(crate) const SNOW_CLIFFS: f32 = 5.0;
pub(crate) const SNOW_SPEC_GLOSS_MULT: f32 = 0.2;
pub(crate) const SNOW_TILING: f32 = 0.05;
pub(crate) const SNOW_NOISE_TILING: f32 = 0.06;
pub(crate) const SNOW_FROST_MIN_EFFECT: f32 = 0.4;

// gfx/FX/constants.fxh:49-50.
pub(crate) const CITY_LIGHTS_TILING: f32 = 0.09103;
pub(crate) const CITY_LIGHTS_INTENSITY: f32 = 5.5;

// gfx/FX/constants.fxh:152-155.
pub(crate) const FOG_COLOR: Vec3 = Vec3 {
    x: 0.12,
    y: 0.28,
    z: 0.6,
};

// gfx/FX/fow.fxh:20-21.
pub(crate) const BRIGHT_FOW_COLOR: Vec3 = Vec3 {
    x: 0.98,
    y: 0.93,
    z: 0.93,
};
pub(crate) const DARK_FOW_COLOR: Vec3 = Vec3 {
    x: 0.08,
    y: 0.08,
    z: 0.1,
};
// gfx/FX/constants.fxh:153-155 (distance fog; FOG_COLOR above is :152).
pub(crate) const FOG_BEGIN: f32 = 1.0;
pub(crate) const FOG_END: f32 = 150.0;
pub(crate) const FOG_MAX: f32 = 0.35;

// gfx/FX/constants.fxh:195-209.
pub(crate) const GB_CAM_MIN: f32 = 100.0;
pub(crate) const GB_CAM_MAX: f32 = 350.0;
pub(crate) const GB_CAM_MAX_FILLING_CLAMP: f32 = 0.8;
pub(crate) const GB_THRESHOLD: f32 = 0.05;
pub(crate) const GB_THRESHOLD2: f32 = 0.25;
pub(crate) const GB_OPACITY_NEAR: f32 = 1.0;
pub(crate) const GB_OPACITY_FAR: f32 = 0.85;
pub(crate) const GB_STRENGTH_CH1: f32 = 1.0;
pub(crate) const GB_STRENGTH_CH2: f32 = 1.0;
pub(crate) const GB_FIRST_LAYER_PRIORITY: f32 = 0.4;

// SEC_MAP_TILE lives in occupation_stripe_law.rs (the lib.rs re-export wins
// over this module's glob, so a copy here would be dead and could drift).

// gfx/FX/standardfuncsgfx.fxh:965-976, CalculateOccupationMask camera fade.
pub(crate) const SEC_CAM_MIN: f32 = 300.0;
pub(crate) const SEC_CAM_MAX: f32 = 1200.0;

// gfx/FX/constants.fxh:204-206.
pub(crate) const BORDER_NIGHT_DESATURATION_MAX: f32 = 0.2;
pub(crate) const BORDER_FOW_REMOVAL_FACTOR: f32 = 0.8;
pub(crate) const BORDER_LIGHT_REMOVAL_FACTOR: f32 = 0.8;

// SHADOW_WEIGHT_TERRAIN lives in shadow_law.rs (the lib.rs re-export wins
// over this module's glob, so a copy here would be dead and could drift).

// gfx/FX/tiled_pointlights.fxh:11-12.
pub(crate) const LIGHT_INDEX_TEXTURE_SIZE: usize = 64;
pub(crate) const LIGHT_DATA_TEXTURE_SIZE: usize = 128;
pub(crate) const TERRAIN_RUNTIME_SCHEMA: &str = "battle-plan-terrain-runtime-inputs-v2";
pub(crate) const MAP_FRAME_BUNDLE_SCHEMA: &str = "battle-plan-map-frame-bundle-v1";
pub(crate) const MAP_FRAME_BUNDLE_FILE: &str = "map_frame_bundle.json";
pub(crate) const TERRAIN_CHUNK_LIBRARY_SCHEMA: &str = "battle-plan-terrain-chunk-library-v1";
pub(crate) const TERRAIN_CHUNK_LIBRARY_FILE: &str = "terrain_chunk_library.json";
pub(crate) const MAP_CAPTURE_SESSION_SCHEMA: &str = "battle-plan-map-capture-session-v1";
pub(crate) const MAP_CAPTURE_SESSION_FILE: &str = "map_capture_session.json";
pub(crate) const TERRAIN_D3D11_ACCEPTANCE_REPORT_SCHEMA: &str =
    "battle-plan-terrain-d3d11-acceptance-v1";
pub(crate) const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
pub(crate) const TERRAIN_CHUNK_LIBRARY_MOMENT: &str = "terrain_chunk_library";
pub(crate) const WATER_CAPTURE_MOMENT: &str = "water_pass";
pub(crate) const RIVERS_CAPTURE_MOMENT: &str = "rivers_pass";
pub(crate) const BORDERS_CAPTURE_MOMENT: &str = "borders_pass";
pub(crate) const TREES_CAPTURE_MOMENT: &str = "trees_pass";
pub(crate) const CITIES_CAPTURE_MOMENT: &str = "cities_pass";
pub(crate) const OVERLAYS_CAPTURE_MOMENT: &str = "overlays_pass";
pub(crate) const WATER_FRAME_BUNDLE_FILE: &str = "map_frame_water_pass_bundle.json";
pub(crate) const RIVERS_FRAME_BUNDLE_FILE: &str = "map_frame_rivers_pass_bundle.json";
pub(crate) const BORDERS_FRAME_BUNDLE_FILE: &str = "map_frame_borders_pass_bundle.json";
pub(crate) const TREES_FRAME_BUNDLE_FILE: &str = "map_frame_trees_pass_bundle.json";
pub(crate) const TERRAIN_ID_MAP_FORMAT: &str = "rgba8_unorm_engine_packed_terrain_ids";
pub(crate) const RUNTIME_RGBA8_FORMAT: &str = "rgba8_unorm_runtime_captured";
pub(crate) const RUNTIME_CONSTANT_BUFFER_FORMAT: &str = "d3d11_constant_buffer_raw_bytes";
pub(crate) const RUNTIME_VERTEX_BUFFER_FORMAT: &str = "d3d11_vertex_buffer_sample";
pub(crate) const RUNTIME_INPUT_LAYOUT_FORMAT: &str = "d3d11_input_layout_descriptor";
pub(crate) const NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT: &str = "d3d11_mapped_subresources";
pub(crate) const TERRAIN_ID_RESOURCE_SYMBOL: &str = "TerrainIDMap";
pub(crate) const CANONICAL_TERRAIN_ID_MAP_FILE: &str = "terrain_id_map.rgba";
pub(crate) const SYNTHESIZED_TERRAIN_ID_MAP_FILE: &str = "__synthesized__/terrain_id_map.rgba";
pub(crate) const WATER_REFRACTION_RESOURCE_SYMBOL: &str = "WaterRefraction";
pub(crate) const PDXWATER_SHADER_SOURCE: &str = "gfx/FX/pdxwater.shader";
pub(crate) const TERRAIN_VERTEX_BUFFER_SYMBOL: &str = "TerrainVertexBuffer";
pub(crate) const TERRAIN_INPUT_LAYOUT_SYMBOL: &str = "TerrainInputLayout";
pub(crate) const TERRAIN_VERTEX_BUFFER_CAPTURE_SCHEMA: &str =
    "battle-plan-terrain-vertex-buffer-capture-v1";
pub(crate) const TERRAIN_INPUT_LAYOUT_CAPTURE_SCHEMA: &str =
    "battle-plan-terrain-input-layout-capture-v1";
pub(crate) const TERRAIN_INPUT_LAYOUT_SHADER_SOURCE: &str =
    "gfx/FX/pdxmap.shader:VS_INPUT_TERRAIN_NOTEXTURE:int4 position + int2 height";
pub(crate) const DXGI_FORMAT_R32G32B32A32_FLOAT: u32 = 2;
pub(crate) const DXGI_FORMAT_R32G32B32A32_SINT: u32 = 4;
pub(crate) const DXGI_FORMAT_R32G32B32_FLOAT: u32 = 6;
pub(crate) const DXGI_FORMAT_R32G32_FLOAT: u32 = 16;
pub(crate) const DXGI_FORMAT_R32_FLOAT: u32 = 41;
pub(crate) const DXGI_FORMAT_R16G16B16A16_FLOAT: u32 = 10;
pub(crate) const DXGI_FORMAT_R16G16B16A16_SINT: u32 = 14;
pub(crate) const DXGI_FORMAT_R32G32_SINT: u32 = 18;
pub(crate) const DXGI_FORMAT_R8G8B8A8_SINT: u32 = 32;
pub(crate) const DXGI_FORMAT_R16G16_SNORM: u32 = 37;
pub(crate) const DXGI_FORMAT_R16G16_SINT: u32 = 38;
pub(crate) const DXGI_FORMAT_R32_UINT: u32 = 42;
pub(crate) const DXGI_FORMAT_R8G8_SINT: u32 = 52;
pub(crate) const DXGI_FORMAT_R16_FLOAT: u32 = 54;
pub(crate) const DXGI_FORMAT_R16_UINT: u32 = 57;
pub(crate) const DXGI_FORMAT_A8_UNORM: u32 = 65;
pub(crate) const DXGI_FORMAT_BC1_UNORM: u32 = 71;
pub(crate) const DXGI_FORMAT_BC1_UNORM_SRGB: u32 = 72;
pub(crate) const DXGI_FORMAT_BC3_UNORM: u32 = 77;
pub(crate) const DXGI_FORMAT_BC3_UNORM_SRGB: u32 = 78;
pub(crate) const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
pub(crate) const DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: u32 = 29;
pub(crate) const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
pub(crate) const DXGI_FORMAT_B8G8R8X8_UNORM: u32 = 88;
pub(crate) const DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: u32 = 91;
pub(crate) const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
pub(crate) const D3D11_BIND_RENDER_TARGET: u32 = 0x20;
pub(crate) const D3D11_BIND_DEPTH_STENCIL: u32 = 0x40;
pub(crate) const D3D11_CLEAR_DEPTH: u32 = 0x1;
pub(crate) const D3D11_CLEAR_STENCIL: u32 = 0x2;
pub(crate) const D3D11_USAGE_DEFAULT: u32 = 0;
pub(crate) const D3D11_USAGE_IMMUTABLE_CAPTURED: u32 = 1;
pub(crate) const D3D11_USAGE_DYNAMIC_CAPTURED: u32 = 2;
pub(crate) const D3D11_USAGE_STAGING_CAPTURED: u32 = 3;
pub(crate) const D3D11_CPU_ACCESS_WRITE_CAPTURED: u32 = 0x10000;
pub(crate) const D3D11_INPUT_PER_VERTEX_DATA: u32 = 0;
pub(crate) const D3D11_INPUT_PER_INSTANCE_DATA: u32 = 1;
pub(crate) const D3D11_APPEND_ALIGNED_ELEMENT: u32 = 0xFFFF_FFFF;
pub(crate) const D3D11_BIND_VERTEX_BUFFER: u32 = 0x1;
pub(crate) const D3D11_BIND_INDEX_BUFFER: u32 = 0x2;
pub(crate) const D3D11_BIND_CONSTANT_BUFFER: u32 = 0x4;
pub(crate) const D3D11_RESOURCE_MISC_TEXTURECUBE: u32 = 0x4;
pub(crate) const D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST: u32 = 4;
pub(crate) const D3D11_TEXTURE_ADDRESS_WRAP: u32 = 1;
pub(crate) const D3D11_TEXTURE_ADDRESS_CLAMP: u32 = 3;
pub(crate) const D3D11_FILTER_MIN_MAG_MIP_POINT: u32 = 0;
pub(crate) const D3D11_FILTER_MIN_MAG_MIP_LINEAR: u32 = 21;
pub(crate) const D3D_REGISTER_COMPONENT_SINT32: u32 = 2;
pub(crate) const MAP_FRAME_MIN_TERRAIN_DRAWS: usize = 2;
pub(crate) const MAP_CAPTURE_REQUIRED_PAYLOAD: [&str; 14] = [
    "shader_bytecode_reflection",
    "input_layouts",
    "vertex_buffers_all_slots",
    "index_buffers",
    "constant_buffers_all_stages",
    "shader_resource_views_textures",
    "sampler_states",
    "draw_args",
    "resource_descs",
    "output_merger_clear_provenance",
    "pre_draw_output_merger_payload",
    "post_pass_render_target_payload",
    "hashes",
    "provenance",
];
pub(crate) const SNOW_MUD_DATA_SYMBOL: &str = "SnowMudData";
pub(crate) const TERRAIN_SCENE_CB0_SYMBOL: &str = "TerrainSceneCB0";
pub(crate) const TERRAIN_SCENE_CB0_BIND_POINT: u32 = 0;
#[cfg(test)]
pub(crate) const MAP_FRAME_SHADER_SOURCES: [&str; 7] = [
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
pub(crate) const MAP_FRAME_REQUIRED_VANILLA_SOURCES: [&str; 8] = [
    "gfx/FX/pdxmap.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
#[cfg(test)]
pub(crate) const WATER_FRAME_SHADER_SOURCES: [&str; 7] = [
    PDXWATER_SHADER_SOURCE,
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
pub(crate) const WATER_FRAME_REQUIRED_VANILLA_SOURCES: [&str; 8] = [
    PDXWATER_SHADER_SOURCE,
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
pub(crate) const EMPTY_REQUIRED_SHADER_RESOURCES: [&str; 0] = [];
pub(crate) const WATER_FRAME_REQUIRED_SHADER_RESOURCES: [&str; 5] = [
    "HeightTexture",
    "LeanTexture1",
    "LeanTexture2",
    "SpecularMap",
    WATER_REFRACTION_RESOURCE_SYMBOL,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderSamplerMode {
    LinearWrap,
    LinearClamp,
    PointClamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderSamplerResourceType {
    Texture2D,
    Cube,
    Shadow,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShaderSamplerExpectation {
    pub(crate) name: &'static str,
    pub(crate) index: u32,
    pub(crate) mode: ShaderSamplerMode,
    pub(crate) resource_type: ShaderSamplerResourceType,
}

pub(crate) const PDXWATER_SAMPLER_EXPECTATIONS: [ShaderSamplerExpectation; 16] = [
    ShaderSamplerExpectation {
        name: "HeightTexture",
        index: 0,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: "LeanTexture1",
        index: 1,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: "LeanTexture2",
        index: 2,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
        index: 3,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: "SpecularMap",
        index: 4,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: WATER_REFRACTION_RESOURCE_SYMBOL,
        index: 5,
        mode: ShaderSamplerMode::LinearClamp,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: "IceDiffuse",
        index: 6,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: "IceNoise",
        index: 7,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: "ReflectionCubeMap",
        index: 8,
        mode: ShaderSamplerMode::LinearClamp,
        resource_type: ShaderSamplerResourceType::Cube,
    },
    ShaderSamplerExpectation {
        name: "SnowMudTexture",
        index: 9,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: LIGHT_INDEX_MAP_SYMBOL,
        index: 10,
        mode: ShaderSamplerMode::PointClamp,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: LIGHT_DATA_MAP_SYMBOL,
        index: 11,
        mode: ShaderSamplerMode::PointClamp,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: GRADIENT_BORDER_CHANNEL1_SYMBOL,
        index: 12,
        mode: ShaderSamplerMode::LinearClamp,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: GRADIENT_BORDER_CHANNEL2_SYMBOL,
        index: 13,
        mode: ShaderSamplerMode::LinearClamp,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: GRADIENT_BORDER_CHANNEL3_SYMBOL,
        index: 14,
        mode: ShaderSamplerMode::PointClamp,
        resource_type: ShaderSamplerResourceType::Texture2D,
    },
    ShaderSamplerExpectation {
        name: SHADOW_MAP_SYMBOL,
        index: 15,
        mode: ShaderSamplerMode::LinearWrap,
        resource_type: ShaderSamplerResourceType::Shadow,
    },
];
#[cfg(test)]
pub(crate) const RIVER_FRAME_SHADER_SOURCES: [&str; 6] = [
    "gfx/FX/river.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
pub(crate) const RIVER_FRAME_REQUIRED_VANILLA_SOURCES: [&str; 7] = [
    "gfx/FX/river.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
pub(crate) const RIVER_FRAME_REQUIRED_SHADER_RESOURCES: [&str; 6] = [
    "RiverData",
    "LeanTexture1",
    "LeanTexture2",
    "SnowMudTexture",
    "GradientBorderChannel1",
    "GradientBorderChannel2",
];
#[cfg(test)]
pub(crate) const BORDER_FRAME_SHADER_SOURCES: [&str; 6] = [
    "gfx/FX/border.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
pub(crate) const BORDER_FRAME_REQUIRED_VANILLA_SOURCES: [&str; 7] = [
    "gfx/FX/border.shader",
    "gfx/FX/pdxmap.fxh",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
pub(crate) const BORDER_FRAME_REQUIRED_SHADER_RESOURCES: [&str; 4] = [
    "BorderDiffuse",
    SNOW_MUD_DATA_SYMBOL,
    "IntelMap",
    SHADOW_MAP_SYMBOL,
];
#[cfg(test)]
pub(crate) const TREE_FRAME_SHADER_SOURCES: [&str; 6] = [
    "gfx/FX/tree.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];
pub(crate) const TREE_FRAME_REQUIRED_VANILLA_SOURCES: [&str; 7] = [
    "gfx/FX/tree.shader",
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
];
pub(crate) const TREE_FRAME_REQUIRED_SHADER_RESOURCES: [&str; 7] = [
    "DiffuseMap",
    "NormalMap",
    "TintMap",
    "SeasonMap",
    "TreeMaskTexture",
    "GradientBorderChannel1",
    "GradientBorderChannel2",
];
pub(crate) const TERRAIN_SCENE_CB0_LAYOUT_SOURCE: &str =
    "gfx/FX/standardfuncsgfx.fxh:ConstantBuffer(0,0)";
pub(crate) const BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub(crate) const TERRAIN_DRAW_CB1_BIND_POINT: u32 = 1;
pub(crate) const TERRAIN_DRAW_CB1_FXH_SOURCE: &str = "gfx/FX/pdxmap.fxh";
pub(crate) const TERRAIN_DRAW_CB1_LAYOUT_SOURCE: &str = "gfx/FX/pdxmap.fxh:ConstantBuffer(1,32)";
pub(crate) const TERRAIN_DRAW_CB1_QUAD_OFFSET_SCALE_BYTE_OFFSET: usize = 64;
pub(crate) const TERRAIN_DRAW_CB1_DETAIL_EDGE_FLAGS_BYTE_OFFSET: usize = 80;
pub(crate) const TERRAIN_POINT_LIGHT_CB2_BIND_POINT: u32 = 2;
pub(crate) const TERRAIN_POINT_LIGHT_CB2_FXH_SOURCE: &str = "gfx/FX/tiled_pointlights.fxh";
pub(crate) const TERRAIN_POINT_LIGHT_CB2_LAYOUT_SOURCE: &str =
    "gfx/FX/tiled_pointlights.fxh:ConstantBuffer(2,62)";
pub(crate) const PROVINCE_SECONDARY_COLOR_MAP_SYMBOL: &str = "ProvinceSecondaryColorMap";
pub(crate) const GRADIENT_BORDER_CHANNEL1_SYMBOL: &str = "GradientBorderChannel1";
pub(crate) const GRADIENT_BORDER_CHANNEL2_SYMBOL: &str = "GradientBorderChannel2";
pub(crate) const GRADIENT_BORDER_CHANNEL3_SYMBOL: &str = "GradientBorderChannel3";
pub(crate) const LIGHT_INDEX_MAP_SYMBOL: &str = "LightIndexMap";
pub(crate) const LIGHT_DATA_MAP_SYMBOL: &str = "LightDataMap";
pub(crate) const SHADOW_MAP_SYMBOL: &str = "ShadowMap";
pub(crate) const TERRAIN_DIFFUSE_SYMBOL: &str = "TerrainDiffuse";
pub(crate) const TERRAIN_COLOR_TINT_SYMBOL: &str = "TerrainColorTint";
pub(crate) const TERRAIN_NORMAL_SYMBOL: &str = "TerrainNormal";
pub(crate) const SNOW_TEXTURE_SYMBOL: &str = "SnowTexture";
pub(crate) const CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL: &str = "CityLightsAndSnowNoise";
pub(crate) const MUD_DIFFUSE_GLOSS_SYMBOL: &str = "MudDiffuseGloss";
pub(crate) const MUD_NORMAL_SPEC_SYMBOL: &str = "MudNormalSpec";
pub(crate) const TERRAIN_BASE_SHIPPED_SHADER_RESOURCES: [(&str, &str); 3] = [
    (TERRAIN_DIFFUSE_SYMBOL, "map/terrain/atlas0.dds"),
    (
        TERRAIN_COLOR_TINT_SYMBOL,
        "map/terrain/colormap_rgb_cityemissivemask_a.dds",
    ),
    (TERRAIN_NORMAL_SYMBOL, "map/terrain/atlas_normal0.dds"),
];

#[derive(Clone, Copy)]
pub(crate) struct MapFrameBundleValidationSpec {
    pub(crate) renderer_label: &'static str,
    pub(crate) capture_moment: &'static str,
    pub(crate) bundle_file: &'static str,
    pub(crate) primary_shader_source: &'static str,
    pub(crate) required_vanilla_sources: &'static [&'static str],
    pub(crate) required_reflected_resources: &'static [&'static str],
    pub(crate) min_draws: usize,
    pub(crate) terrain_strict: bool,
    pub(crate) ignore_dependent_pre_draw_output_blockers: bool,
}

pub(crate) const TERRAIN_MAP_FRAME_SPEC: MapFrameBundleValidationSpec =
    MapFrameBundleValidationSpec {
        renderer_label: "production terrain renderer",
        capture_moment: TERRAIN_CAPTURE_MOMENT,
        bundle_file: MAP_FRAME_BUNDLE_FILE,
        primary_shader_source: "gfx/FX/pdxmap.shader",
        required_vanilla_sources: &MAP_FRAME_REQUIRED_VANILLA_SOURCES,
        required_reflected_resources: &EMPTY_REQUIRED_SHADER_RESOURCES,
        min_draws: MAP_FRAME_MIN_TERRAIN_DRAWS,
        terrain_strict: true,
        ignore_dependent_pre_draw_output_blockers: false,
    };

pub(crate) const WATER_MAP_FRAME_SPEC: MapFrameBundleValidationSpec =
    MapFrameBundleValidationSpec {
        renderer_label: "production water renderer",
        capture_moment: WATER_CAPTURE_MOMENT,
        bundle_file: WATER_FRAME_BUNDLE_FILE,
        primary_shader_source: "gfx/FX/pdxwater.shader",
        required_vanilla_sources: &WATER_FRAME_REQUIRED_VANILLA_SOURCES,
        required_reflected_resources: &WATER_FRAME_REQUIRED_SHADER_RESOURCES,
        min_draws: 1,
        terrain_strict: false,
        ignore_dependent_pre_draw_output_blockers: true,
    };

pub(crate) const RIVERS_MAP_FRAME_SPEC: MapFrameBundleValidationSpec =
    MapFrameBundleValidationSpec {
        renderer_label: "production rivers renderer",
        capture_moment: RIVERS_CAPTURE_MOMENT,
        bundle_file: RIVERS_FRAME_BUNDLE_FILE,
        primary_shader_source: "gfx/FX/river.shader",
        required_vanilla_sources: &RIVER_FRAME_REQUIRED_VANILLA_SOURCES,
        required_reflected_resources: &RIVER_FRAME_REQUIRED_SHADER_RESOURCES,
        min_draws: 1,
        terrain_strict: false,
        ignore_dependent_pre_draw_output_blockers: true,
    };

pub(crate) const BORDERS_MAP_FRAME_SPEC: MapFrameBundleValidationSpec =
    MapFrameBundleValidationSpec {
        renderer_label: "production borders renderer",
        capture_moment: BORDERS_CAPTURE_MOMENT,
        bundle_file: BORDERS_FRAME_BUNDLE_FILE,
        primary_shader_source: "gfx/FX/border.shader",
        required_vanilla_sources: &BORDER_FRAME_REQUIRED_VANILLA_SOURCES,
        required_reflected_resources: &BORDER_FRAME_REQUIRED_SHADER_RESOURCES,
        min_draws: 1,
        terrain_strict: false,
        ignore_dependent_pre_draw_output_blockers: true,
    };

pub(crate) const TREES_MAP_FRAME_SPEC: MapFrameBundleValidationSpec =
    MapFrameBundleValidationSpec {
        renderer_label: "production trees renderer",
        capture_moment: TREES_CAPTURE_MOMENT,
        bundle_file: TREES_FRAME_BUNDLE_FILE,
        primary_shader_source: "gfx/FX/tree.shader",
        required_vanilla_sources: &TREE_FRAME_REQUIRED_VANILLA_SOURCES,
        required_reflected_resources: &TREE_FRAME_REQUIRED_SHADER_RESOURCES,
        min_draws: 1,
        terrain_strict: false,
        ignore_dependent_pre_draw_output_blockers: true,
    };
pub(crate) const TERRAIN_POST_DIFFUSE_SHIPPED_SHADER_RESOURCES: [(&str, &str); 4] = [
    (
        SNOW_TEXTURE_SYMBOL,
        "map/terrain/snow_normal_rgb_diffuse_a.dds",
    ),
    (
        CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL,
        "map/terrain/citylights_rgb_snowmask_a_0.dds",
    ),
    (
        MUD_DIFFUSE_GLOSS_SYMBOL,
        "map/terrain/mud_diffuse_rgb_gloss_a_0.dds",
    ),
    (
        MUD_NORMAL_SPEC_SYMBOL,
        "map/terrain/mud_normal_rgb_spec_a_0.dds",
    ),
];
pub(crate) const TERRAIN_POST_DIFFUSE_RUNTIME_SHADER_RESOURCES: [&str; 7] = [
    SNOW_MUD_DATA_SYMBOL,
    PROVINCE_SECONDARY_COLOR_MAP_SYMBOL,
    GRADIENT_BORDER_CHANNEL1_SYMBOL,
    GRADIENT_BORDER_CHANNEL2_SYMBOL,
    LIGHT_INDEX_MAP_SYMBOL,
    LIGHT_DATA_MAP_SYMBOL,
    SHADOW_MAP_SYMBOL,
];

impl ShaderSamplerMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::LinearWrap => "linear/wrap",
            Self::LinearClamp => "linear/clamp",
            Self::PointClamp => "point/clamp",
        }
    }

    pub(crate) fn filter(self) -> u32 {
        match self {
            Self::LinearWrap | Self::LinearClamp => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            Self::PointClamp => D3D11_FILTER_MIN_MAG_MIP_POINT,
        }
    }

    pub(crate) fn address_u(self) -> u32 {
        match self {
            Self::LinearWrap => D3D11_TEXTURE_ADDRESS_WRAP,
            Self::LinearClamp | Self::PointClamp => D3D11_TEXTURE_ADDRESS_CLAMP,
        }
    }

    pub(crate) fn address_v(self) -> u32 {
        self.address_u()
    }
}

impl ShaderSamplerResourceType {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Texture2D => "Texture2D",
            Self::Cube => "Cube",
            Self::Shadow => "Shadow",
        }
    }
}
