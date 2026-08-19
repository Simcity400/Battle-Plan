
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const TERRAIN_CAPTURE_MOMENT: &str = "terrain_active_pass";
pub const TERRAIN_SCENE_CB0_SYMBOL: &str = "TerrainSceneCB0";
pub const TERRAIN_SCENE_CB0_BIND_POINT: u32 = 0;
pub const TERRAIN_SCENE_CB0_FILE: &str = "terrain_scene_cb0.bin";
pub const TERRAIN_SCENE_CB0_META_FILE: &str = "terrain_scene_cb0_capture.json";
pub const TERRAIN_DRAW_CB1_LAYOUT_SOURCE: &str = "gfx/FX/pdxmap.fxh:ConstantBuffer(1,32)";
pub const TERRAIN_DRAW_CB1_BIND_POINT: u32 = 1;
pub const PDXMAP_FXH_SOURCE: &str = "gfx/FX/pdxmap.fxh";
pub const MAP_FRAME_BUNDLE_FILE: &str = "map_frame_bundle.json";

const DXGI_FORMAT_R16G16B16A16_SINT: u32 = 14;
const DXGI_FORMAT_R16G16_SINT: u32 = 38;
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const D3D11_BIND_CONSTANT_BUFFER: u32 = 0x4;
const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
const D3D11_INPUT_PER_VERTEX_DATA: u32 = 0;
const D3D_REGISTER_COMPONENT_SINT32: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindingSite {
    pub draw_index: u32,
    pub stage: &'static str,
    pub slot: u32,
}

impl BindingSite {
    pub const ALL: [BindingSite; 6] = [
        BindingSite { draw_index: 0, stage: "PS", slot: TERRAIN_SCENE_CB0_BIND_POINT },
        BindingSite { draw_index: 0, stage: "VS", slot: TERRAIN_SCENE_CB0_BIND_POINT },
        BindingSite { draw_index: 0, stage: "VS", slot: TERRAIN_DRAW_CB1_BIND_POINT },
        BindingSite { draw_index: 1, stage: "PS", slot: TERRAIN_SCENE_CB0_BIND_POINT },
        BindingSite { draw_index: 1, stage: "VS", slot: TERRAIN_SCENE_CB0_BIND_POINT },
        BindingSite { draw_index: 1, stage: "VS", slot: TERRAIN_DRAW_CB1_BIND_POINT },
    ];

    pub fn canonical_name(&self) -> &'static str {
        if self.slot == TERRAIN_SCENE_CB0_BIND_POINT {
            TERRAIN_SCENE_CB0_SYMBOL
        } else {
            TERRAIN_DRAW_CB1_LAYOUT_SOURCE
        }
    }

    pub fn stale_name(&self) -> &'static str {
        if self.slot == TERRAIN_SCENE_CB0_BIND_POINT {
            "dx11_cb0"
        } else {
            "dx11_cb1"
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstantBufferNames {
    Stale,
    Canonical,
    CanonicalExcept(BindingSite),
}

impl ConstantBufferNames {
    fn binding_name(self, site: BindingSite) -> &'static str {
        match self {
            Self::Stale => site.stale_name(),
            Self::Canonical => site.canonical_name(),
            Self::CanonicalExcept(stale) => {
                if stale == site {
                    site.stale_name()
                } else {
                    site.canonical_name()
                }
            }
        }
    }

    fn ps_reflection(self) -> Value {
        match self {
            Self::Stale => json!({ "dx11_cb0": TERRAIN_SCENE_CB0_BIND_POINT }),
            _ => json!({ TERRAIN_SCENE_CB0_SYMBOL: TERRAIN_SCENE_CB0_BIND_POINT }),
        }
    }

    fn vs_reflection(self) -> Value {
        match self {
            Self::Stale => json!({
                "dx11_cb0": TERRAIN_SCENE_CB0_BIND_POINT,
                "dx11_cb1": TERRAIN_DRAW_CB1_BIND_POINT,
                TERRAIN_DRAW_CB1_LAYOUT_SOURCE: TERRAIN_DRAW_CB1_BIND_POINT,
            }),
            _ => json!({
                TERRAIN_SCENE_CB0_SYMBOL: TERRAIN_SCENE_CB0_BIND_POINT,
                TERRAIN_DRAW_CB1_LAYOUT_SOURCE: TERRAIN_DRAW_CB1_BIND_POINT,
            }),
        }
    }
}

pub fn write_map_frame_bundle_fixture(root: &Path, names: ConstantBufferNames) -> PathBuf {
    let scene_cb0 = vec![0u8; 416];
    let draw_cb1 = vec![0u8; 112];
    write_file(root, TERRAIN_SCENE_CB0_FILE, &scene_cb0);
    write_json_value(
        &root.join(TERRAIN_SCENE_CB0_META_FILE),
        json!({
            "symbol": TERRAIN_SCENE_CB0_SYMBOL,
            "file": TERRAIN_SCENE_CB0_FILE,
            "byte_len": scene_cb0.len(),
            "fnv1a64": fnv1a64_bytes(&scene_cb0),
            "shader_constant_buffer_slot": TERRAIN_SCENE_CB0_BIND_POINT,
            "source": "portable_terrain_fixture"
        }),
    );
    for draw_index in 0..2u32 {
        write_file(
            root,
            &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot0.bin"),
            &le_i16_bytes(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1]),
        );
        write_file(
            root,
            &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot1.bin"),
            &le_i16_bytes(&[1000, 2000, 1100, 2100, 1200, 2200]),
        );
        write_file(
            root,
            &format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin"),
            &le_u16_bytes(&[0, 1, 2]),
        );
        write_file(
            root,
            &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot0.bin"),
            &scene_cb0,
        );
        write_file(
            root,
            &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot1.bin"),
            &draw_cb1,
        );
        write_file(
            root,
            &format!("map_frame_draw{draw_index:03}_ps_constant_buffer_slot0.bin"),
            &scene_cb0,
        );
    }
    write_file(root, "terrain_id_map.rgba", &[4u8; 16]);
    write_file(
        root,
        "pdxmap_pixel_shader_0123456789abcdef.bin",
        &[9u8; 128],
    );
    write_file(
        root,
        "pdxmap_vertex_shader_fedcba9876543210.bin",
        &[7u8; 96],
    );
    write_vanilla_source(root, "gfx/FX/pdxmap.shader", "// gfx/FX/pdxmap.shader\n");
    write_vanilla_source(root, PDXMAP_FXH_SOURCE, MINIMAL_PDXMAP_FXH_SOURCE);
    write_vanilla_source(root, "gfx/FX/constants.fxh", "// gfx/FX/constants.fxh\n");
    write_vanilla_source(
        root,
        "gfx/FX/standardfuncsgfx.fxh",
        MINIMAL_STANDARD_FUNCTIONS_SOURCE,
    );
    write_vanilla_source(root, "gfx/FX/tiled_pointlights.fxh", "// tiled\n");
    write_vanilla_source(root, "gfx/FX/shadow.fxh", "// shadow\n");
    write_vanilla_source(root, "gfx/FX/fow.fxh", "// fow\n");
    write_vanilla_source(
        root,
        "common/defines/00_graphics.lua",
        MINIMAL_GRAPHICS_DEFINES_SOURCE,
    );

    let draw0 = contract_draw(root, 0, names);
    let draw1 = contract_draw(root, 1, names);
    let active_draw = match names {
        ConstantBufferNames::CanonicalExcept(_) => {
            contract_draw(root, 1, ConstantBufferNames::Canonical)
        }
        _ => draw1.clone(),
    };
    let frame = json!({
        "schema": "battle-plan-map-frame-bundle-v1",
        "provenance": "portable_terrain_fixture",
        "capture_moment": TERRAIN_CAPTURE_MOMENT,
        "session_manifest": "map_capture_session.json",
        "shader_cache_constants": {
            "map_size_x": 2.0,
            "map_size_y": 2.0,
            "map_pow2_x": 1.0,
            "map_pow2_y": 1.0,
            "fow_pow2_x": 1.0,
            "fow_pow2_y": 1.0,
            "terrain_id_srv_slot": 5,
            "cache_files": ["dx11_win32/ps_5_0/map_t5.scache"],
            "provenance": "portable_terrain_fixture_shader_cache_scan",
            "extraction": "portable_terrain_fixture_shader_cache_control"
        },
        "active_draw": active_draw,
        "draws": [draw0, draw1],
        "post_pass_output_merger": null,
        "vanilla_sources": vanilla_sources_evidence(root),
        "runtime_files": runtime_files_evidence(root),
        "blockers": []
    });
    let session = json!({
        "schema": "battle-plan-map-capture-session-v1",
        "provenance": "portable_terrain_fixture",
        "required_first_moment": TERRAIN_CAPTURE_MOMENT,
        "moments": [{
            "name": TERRAIN_CAPTURE_MOMENT,
            "status": "captured_pending_renderer_validation",
            "bundle_file": MAP_FRAME_BUNDLE_FILE,
            "required_payload": [
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
                "provenance"
            ],
            "blockers": []
        }],
        "deferred_moments": []
    });
    write_json_value(&root.join("map_capture_session.json"), session);
    let path = root.join(MAP_FRAME_BUNDLE_FILE);
    write_json_value(&path, frame);
    path
}

fn contract_draw(root: &Path, draw_index: u32, names: ConstantBufferNames) -> Value {
    let scene_cb0_file = format!("map_frame_draw{draw_index:03}_ps_constant_buffer_slot0.bin");
    json!({
        "draw_index": draw_index,
        "draw_call": "DrawIndexed",
        "draw_count": 3,
        "draw_start": 0,
        "base_vertex_location": 0,
        "topology": 4,
        "shader": {
            "stage": "PS",
            "pointer": "0x1",
            "bytecode_file": "pdxmap_pixel_shader_0123456789abcdef.bin",
            "byte_len": 128,
            "fnv1a64": file_hash(root, "pdxmap_pixel_shader_0123456789abcdef.bin"),
            "reflection_resources": { "TerrainIDMap": 5 },
            "input_parameters": [],
            "constant_buffers": names.ps_reflection(),
            "source_evidence": MAP_FRAME_SHADER_SOURCES
        },
        "vertex_shader": {
            "stage": "VS",
            "pointer": "0x10",
            "bytecode_file": "pdxmap_vertex_shader_fedcba9876543210.bin",
            "byte_len": 96,
            "fnv1a64": file_hash(root, "pdxmap_vertex_shader_fedcba9876543210.bin"),
            "reflection_resources": {},
            "input_parameters": [
                {
                    "semantic_name": "POSITION",
                    "semantic_index": 0,
                    "register": 0,
                    "component_type": D3D_REGISTER_COMPONENT_SINT32,
                    "mask": 15,
                    "read_write_mask": 15
                },
                {
                    "semantic_name": "TEXCOORD",
                    "semantic_index": 0,
                    "register": 1,
                    "component_type": D3D_REGISTER_COMPONENT_SINT32,
                    "mask": 3,
                    "read_write_mask": 3
                }
            ],
            "constant_buffers": names.vs_reflection(),
            "source_evidence": MAP_FRAME_SHADER_SOURCES
        },
        "input_layout": {
            "pointer": "0x2",
            "elements": [
                {
                    "semantic_name": "POSITION",
                    "semantic_index": 0,
                    "format": DXGI_FORMAT_R16G16B16A16_SINT,
                    "input_slot": 0,
                    "aligned_byte_offset": 0,
                    "input_slot_class": D3D11_INPUT_PER_VERTEX_DATA,
                    "instance_data_step_rate": 0
                },
                {
                    "semantic_name": "TEXCOORD",
                    "semantic_index": 0,
                    "format": DXGI_FORMAT_R16G16_SINT,
                    "input_slot": 1,
                    "aligned_byte_offset": 0,
                    "input_slot_class": D3D11_INPUT_PER_VERTEX_DATA,
                    "instance_data_step_rate": 0
                }
            ]
        },
        "vertex_buffers": {
            "0": buffer_binding(root, "IA", 0, None, &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot0.bin"), 1, Some(8), Some(0)),
            "1": buffer_binding(root, "IA", 1, None, &format!("map_frame_draw{draw_index:03}_ia_vertex_buffer_slot1.bin"), 1, Some(4), Some(0))
        },
        "index_buffer": {
            "file": format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin"),
            "byte_len": file_len(root, &format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin")),
            "fnv1a64": file_hash(root, &format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin")),
            "dxgi_format": 57,
            "offset": 0,
            "bind_flags": 2,
            "usage": 0,
            "cpu_access_flags": 0
        },
        "constant_buffers": [
            buffer_binding(root, "VS", 0, Some(names.binding_name(BindingSite { draw_index, stage: "VS", slot: 0 })), &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot0.bin"), D3D11_BIND_CONSTANT_BUFFER, None, None),
            buffer_binding(root, "VS", 1, Some(names.binding_name(BindingSite { draw_index, stage: "VS", slot: 1 })), &format!("map_frame_draw{draw_index:03}_vs_constant_buffer_slot1.bin"), D3D11_BIND_CONSTANT_BUFFER, None, None),
            buffer_binding(root, "PS", 0, Some(names.binding_name(BindingSite { draw_index, stage: "PS", slot: 0 })), &scene_cb0_file, D3D11_BIND_CONSTANT_BUFFER, None, None)
        ],
        "shader_resources": [{
            "stage": "PS",
            "slot": 5,
            "declared_name": "TerrainIDMap",
            "is_null": false,
            "view_desc": {
                "format": DXGI_FORMAT_B8G8R8A8_UNORM,
                "view_dimension": 4,
                "most_detailed_mip": 0,
                "mip_levels": 1,
                "first_array_slice": 0,
                "array_size": 1,
                "num_cubes": 0
            },
            "file": "terrain_id_map.rgba",
            "byte_len": file_len(root, "terrain_id_map.rgba"),
            "fnv1a64": file_hash(root, "terrain_id_map.rgba"),
            "dxgi_format": DXGI_FORMAT_B8G8R8A8_UNORM,
            "width": 2,
            "height": 2,
            "mip_count": 1,
            "array_size": 1,
            "sample_count": 1,
            "sample_quality": 0,
            "payload_format": "d3d11_mapped_subresources",
            "subresources": [{
                "mip_slice": 0,
                "array_slice": 0,
                "file_offset": 0,
                "byte_len": 16,
                "row_pitch": 8,
                "depth_pitch": 16,
                "width": 2,
                "height": 2
            }],
            "bind_flags": D3D11_BIND_SHADER_RESOURCE,
            "usage": 0,
            "cpu_access_flags": 0,
            "misc_flags": 0
        }],
        "samplers": [{
            "stage": "PS",
            "slot": 5,
            "filter": 0,
            "address_u": 3,
            "address_v": 3,
            "address_w": 3,
            "mip_lod_bias": 0.0,
            "max_anisotropy": 1,
            "comparison_func": 0,
            "border_color": [0.0, 0.0, 0.0, 0.0],
            "min_lod": 0.0,
            "max_lod": 340282346638528859811704183484516925440.0
        }],
        "draw_state": null,
        "post_draw_output_merger": null
    })
}

#[allow(clippy::too_many_arguments)]
fn buffer_binding(
    root: &Path,
    stage: &str,
    slot: u32,
    declared_name: Option<&str>,
    file: &str,
    bind_flags: u32,
    stride: Option<u32>,
    offset: Option<u32>,
) -> Value {
    json!({
        "stage": stage,
        "slot": slot,
        "declared_name": declared_name,
        "file": file,
        "byte_len": file_len(root, file),
        "fnv1a64": file_hash(root, file),
        "stride": stride,
        "offset": offset,
        "dxgi_format": null,
        "bind_flags": bind_flags,
        "usage": 0,
        "cpu_access_flags": 0
    })
}

fn runtime_files_evidence(root: &Path) -> Value {
    let files = [
        "terrain_id_map.rgba",
        TERRAIN_SCENE_CB0_FILE,
        "pdxmap_pixel_shader_0123456789abcdef.bin",
        "pdxmap_vertex_shader_fedcba9876543210.bin",
    ];
    let mut object = Map::new();
    for file in files {
        object.insert(file.to_string(), file_evidence(root, file));
    }
    Value::Object(object)
}

fn vanilla_sources_evidence(root: &Path) -> Value {
    let mut object = Map::new();
    for source in MAP_FRAME_SHADER_SOURCES
        .iter()
        .copied()
        .chain(std::iter::once("common/defines/00_graphics.lua"))
    {
        let file = format!("vanilla/{source}");
        object.insert(source.to_string(), file_evidence(root, &file));
    }
    Value::Object(object)
}

fn file_evidence(root: &Path, file: &str) -> Value {
    json!({
        "file": file,
        "byte_len": file_len(root, file),
        "fnv1a64": file_hash(root, file),
        "provenance": "portable_terrain_fixture"
    })
}

fn write_vanilla_source(root: &Path, source: &str, contents: &str) {
    write_file(root, &format!("vanilla/{source}"), contents.as_bytes());
}

fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent directory");
    }
    fs::write(path, bytes).expect("write fixture file");
}

fn write_json_value(path: &Path, value: Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture parent directory");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(&value).expect("serialize fixture json"),
    )
    .expect("write fixture json");
}

fn file_len(root: &Path, file: &str) -> usize {
    fs::metadata(root.join(file))
        .expect("fixture file metadata")
        .len() as usize
}

fn file_hash(root: &Path, file: &str) -> String {
    fnv1a64_bytes(&fs::read(root.join(file)).expect("read fixture file"))
}

pub fn fnv1a64_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn le_i16_bytes(values: &[i16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn le_u16_bytes(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

const MAP_FRAME_SHADER_SOURCES: [&str; 7] = [
    "gfx/FX/pdxmap.shader",
    PDXMAP_FXH_SOURCE,
    "gfx/FX/constants.fxh",
    "gfx/FX/standardfuncsgfx.fxh",
    "gfx/FX/tiled_pointlights.fxh",
    "gfx/FX/shadow.fxh",
    "gfx/FX/fow.fxh",
];

const MINIMAL_STANDARD_FUNCTIONS_SOURCE: &str = r#"
            ConstantBuffer( 0, 0 )
            {
                float4x4    ViewProjectionMatrix;
                float4x4    InvViewProjMatrix;
                float4      vGBCamDistOverride_GBOutlineCutoff;
                float4      vVirtualSunPos;
                float4      vVirtualMoonPos;
                float4      vSecondVirtualSunPos;
                float4      vSecondVirtualMoonPos;
                float3      vCamPos;
                float       HdrRange;
                float3      vCamLookAtDir;
                float       vGlobalTime;
                float4      vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed;
                float4      DayNight_Hour_SunDir;
                float3      AmbientPosX;
                float       ShadowFadeFactor;
                float3      AmbientNegX;
                float       FOWFadeFactor;
                float3      AmbientPosY;
                float       MinMeshAlpha;
                float3      AmbientNegY;
                float       NegFogMultiplier;
                float3      AmbientPosZ;
                float3      AmbientNegZ;
                float       CubemapIntensity;
                float4      SunDiffuseIntensity;
                float4      MoonDiffuseIntensity;
                float       GB_TextureHeight;
                float       SunSpecularIntensity;
            };
        "#;

const MINIMAL_PDXMAP_FXH_SOURCE: &str = r#"
            ConstantBuffer( 1, 32 )
            {
                float4x4 ShadowMapTextureMatrix;
                float4 QuadOffset_Scale_IsDetail;
                float4 vSnap;
                float4 vBorderLookup_HeightScale_UseMultisample_SeasonLerp;
            }
        "#;

const MINIMAL_GRAPHICS_DEFINES_SOURCE: &str = r#"
            NDefines_Graphics = {
                NGraphics = {
                    CAMERA_OUTSIDE_MAP_DISTANCE_TOP = 200.0,
                    CAMERA_OUTSIDE_MAP_DISTANCE_BOTTOM = 200.0,
                    CAMERA_ZOOM_SPEED = 50,
                    CAMERA_ZOOM_KEY_SCALE = 0.02,
                    CAMERA_ZOOM_SPEED_DISTANCE_MULT = 6.0,
                },
                NFrontend = {
                    CAMERA_MIN_HEIGHT = 50.0,
                    CAMERA_MAX_HEIGHT = 3000.0,
                },
            }
        "#;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "portable_terrain_fixture_{}_{}_{}",
            std::process::id(),
            name,
            nonce
        ));
        fs::create_dir_all(&path).expect("create temp fixture root");
        path
    }

    #[test]
    fn the_two_variants_differ_only_in_constant_buffer_names() {
        let stale_root = temp_root("stale");
        let canonical_root = temp_root("canonical");
        let stale_path = write_map_frame_bundle_fixture(&stale_root, ConstantBufferNames::Stale);
        let canonical_path =
            write_map_frame_bundle_fixture(&canonical_root, ConstantBufferNames::Canonical);

        let mut stale: Value =
            serde_json::from_slice(&fs::read(&stale_path).expect("read stale bundle"))
                .expect("parse stale bundle");
        let canonical: Value =
            serde_json::from_slice(&fs::read(&canonical_path).expect("read canonical bundle"))
                .expect("parse canonical bundle");
        assert_ne!(stale, canonical, "the variants must not be identical");

        for draw in draws_mut(&mut stale) {
            draw["shader"]["constant_buffers"] = ConstantBufferNames::Canonical.ps_reflection();
            draw["vertex_shader"]["constant_buffers"] =
                ConstantBufferNames::Canonical.vs_reflection();
            for binding in draw["constant_buffers"]
                .as_array_mut()
                .expect("draw constant_buffers array")
            {
                let stage = binding["stage"].as_str().expect("binding stage").to_string();
                let slot = binding["slot"].as_u64().expect("binding slot");
                let canonical_name = if slot == u64::from(TERRAIN_SCENE_CB0_BIND_POINT) {
                    TERRAIN_SCENE_CB0_SYMBOL
                } else {
                    TERRAIN_DRAW_CB1_LAYOUT_SOURCE
                };
                assert!(stage == "PS" || stage == "VS", "unexpected stage {stage}");
                binding["declared_name"] = Value::String(canonical_name.to_string());
            }
        }
        assert_eq!(
            stale, canonical,
            "renaming the constant buffers is the ONLY difference between the variants"
        );

        let _ = fs::remove_dir_all(stale_root);
        let _ = fs::remove_dir_all(canonical_root);
    }

    fn draws_mut(bundle: &mut Value) -> Vec<&mut Value> {
        let object = bundle.as_object_mut().expect("bundle object");
        let mut draws = Vec::new();
        for (key, value) in object.iter_mut() {
            match key.as_str() {
                "active_draw" => draws.push(value),
                "draws" => draws.extend(value.as_array_mut().expect("draws array").iter_mut()),
                _ => {}
            }
        }
        draws
    }

    #[test]
    fn each_single_site_variant_differs_from_canonical_in_exactly_one_place() {
        let canonical_root = temp_root("one_place_canonical");
        let canonical: Value = serde_json::from_slice(
            &fs::read(write_map_frame_bundle_fixture(
                &canonical_root,
                ConstantBufferNames::Canonical,
            ))
            .expect("read canonical bundle"),
        )
        .expect("parse canonical bundle");

        for site in BindingSite::ALL {
            let root = temp_root(&format!(
                "one_place_{}_{}_{}",
                site.draw_index, site.stage, site.slot
            ));
            let variant: Value = serde_json::from_slice(
                &fs::read(write_map_frame_bundle_fixture(
                    &root,
                    ConstantBufferNames::CanonicalExcept(site),
                ))
                .expect("read variant bundle"),
            )
            .expect("parse variant bundle");

            let differences = json_differences("", &canonical, &variant);
            assert_eq!(
                differences,
                vec![format!(
                    "/draws/{}/constant_buffers/{}/declared_name",
                    site.draw_index,
                    binding_position(site)
                )],
                "staling {site:?} changed the wrong set of locations"
            );
            let _ = fs::remove_dir_all(root);
        }
        let _ = fs::remove_dir_all(canonical_root);
    }

    fn binding_position(site: BindingSite) -> usize {
        match (site.stage, site.slot) {
            ("VS", 0) => 0,
            ("VS", 1) => 1,
            ("PS", 0) => 2,
            other => panic!("unexpected binding site {other:?}"),
        }
    }

    fn json_differences(pointer: &str, left: &Value, right: &Value) -> Vec<String> {
        match (left, right) {
            (Value::Object(a), Value::Object(b)) => {
                let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
                keys.sort();
                keys.dedup();
                keys.into_iter()
                    .flat_map(|key| {
                        let child = format!("{pointer}/{key}");
                        match (a.get(key), b.get(key)) {
                            (Some(x), Some(y)) => json_differences(&child, x, y),
                            _ => vec![child],
                        }
                    })
                    .collect()
            }
            (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
                .iter()
                .zip(b.iter())
                .enumerate()
                .flat_map(|(index, (x, y))| json_differences(&format!("{pointer}/{index}"), x, y))
                .collect(),
            _ if left == right => Vec::new(),
            _ => vec![pointer.to_string()],
        }
    }

    #[test]
    fn the_fixture_is_byte_identical_across_roots() {
        let first_root = temp_root("determinism_a");
        let second_root = temp_root("determinism_b");
        let first = write_map_frame_bundle_fixture(&first_root, ConstantBufferNames::Stale);
        let second = write_map_frame_bundle_fixture(&second_root, ConstantBufferNames::Stale);
        assert_eq!(
            fs::read(&first).expect("read first bundle"),
            fs::read(&second).expect("read second bundle"),
            "the bundle must not depend on where it was written"
        );
        let _ = fs::remove_dir_all(first_root);
        let _ = fs::remove_dir_all(second_root);
    }
}
