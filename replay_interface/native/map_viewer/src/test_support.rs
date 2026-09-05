
use crate::*;

pub(crate) fn fixture_game_resolver(root: &std::path::Path) -> GameFileResolver {
    let nested = root.join("vanilla");
    GameFileResolver::vanilla(if nested.is_dir() { nested } else { root.to_path_buf() }).unwrap()
}
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use winapi::um::d3d11::{D3D11_DSV_DIMENSION_TEXTURE2D, D3D11_RTV_DIMENSION_TEXTURE2D};
#[cfg(windows)]
use winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;

pub(crate) fn with_define_fixture<T>(source: &str, read: impl FnOnce(&GameFileResolver) -> T) -> T {
    let root = temp_test_dir("effective_define_fixture");
    fs::create_dir_all(root.join("common/defines")).unwrap();
    fs::write(root.join("common/defines/fixture.lua"), source).unwrap();
    let resolver = GameFileResolver::vanilla(&root).unwrap();
    let result = read(&resolver);
    drop(resolver);
    portable_test_fs_retry::remove_dir_all(root).unwrap();
    result
}

pub(crate) fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.0001,
        "expected {actual} to be close to {expected}"
    );
}

pub(crate) fn assert_vec3_close(actual: Vec3, expected: Vec3) {
    assert_close(actual.x, expected.x);
    assert_close(actual.y, expected.y);
    assert_close(actual.z, expected.z);
}

pub(crate) fn assert_unit_vec3(actual: Vec3) {
    assert_close(
        actual.x * actual.x + actual.y * actual.y + actual.z * actual.z,
        1.0,
    );
}

/// A one-hour `WorldFrame` whose raw snapshot is exactly `rest`, for tests that
/// drive a frame-keyed admission from a shape they chose.
pub(crate) fn frame_with_rest(rest: serde_json::Value) -> std::sync::Arc<replay_core::WorldFrame> {
    let rest = rest.as_object().expect("a snapshot object").clone();
    replay_core::WorldTimeline::open(Box::new(RestSource(rest)))
        .expect("timeline")
        .frame(0)
        .expect("frame")
}

/// A viewer-owned map state that validates and selects nothing: no camera, no
/// selection, a mode no family answers to.
pub(crate) fn bare_viewer() -> ViewerOwnedMapState {
    ViewerOwnedMapState {
        viewport_width: 16,
        viewport_height: 16,
        camera: None,
        observer_country_id: None,
        map_mode_id: "not_a_mode".to_string(),
        map_mode_selection_epoch: 0,
        selected_object_ids: Vec::new(),
        input_revision: 0,
        animation_time_f32_bits: 0,
        frame_delta_seconds_f32_bits: 0,
    }
}

struct RestSource(serde_json::Map<String, serde_json::Value>);

impl replay_core::ReplaySource for RestSource {
    fn game_id(&self) -> &str {
        "test-support-rest-frame"
    }
    fn hours(&self) -> &[i64] {
        &[0]
    }
    fn warnings(&self) -> &[String] {
        &[]
    }
    fn frame(
        &self,
        hour_index: i64,
    ) -> std::result::Result<replay_core::RawFrame, replay_core::SourceError> {
        Ok(replay_core::RawFrame {
            hour_index,
            date: None,
            save_version: None,
            schema_version: None,
            rest: self.0.clone().into(),
        })
    }
}

pub(crate) fn identity4() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub(crate) fn single_pixel_rgba_chain(rgba: [u8; 4]) -> RgbaMipChain {
    RgbaMipChain {
        levels: vec![RgbaImage {
            width: 1,
            height: 1,
            pixels: rgba.to_vec(),
        }],
        color_space: RgbaColorSpace::Linear,
    }
}

pub(crate) fn minimal_standard_functions_source() -> String {
    r#"
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
            static const float NIGHT_OPACITY = 0.85f;
            static const float NIGHT_DARKNESS = 0.5f;

            float CalculateOccupationMask( in float2 uv )
            {
                float t = 3.14159 / 8.0;
                float w = SEC_MAP_TILE;

                float stripeVal = cos( ( uv.x * cos( t ) * w ) + ( uv.y * sin( t ) * w ) );
                float camDist = cam_distance( 300.0, 1200.0 );
                stripeVal += camDist * 1.5;

                stripeVal = smoothstep(0.0, 1.0, stripeVal*1.7) * lerp(1.0, 0.3, camDist);
                return stripeVal;
            }
        "#
    .to_string()
}

pub(crate) fn valid_scene_cb0_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; 416];
    write_identity_float4x4(&mut bytes, 0);
    write_f32_le(&mut bytes, 208, 2695.059);
    write_f32_le(&mut bytes, 212, 310.11);
    write_f32_le(&mut bytes, 216, 1569.962);
    write_f32_le(&mut bytes, 220, 1.0);
    write_f32_le(&mut bytes, 128, 0.0);
    write_f32_le(&mut bytes, 132, 0.39);
    write_f32_le(&mut bytes, 136, 0.973);
    write_f32_le(&mut bytes, 140, 0.973);
    write_f32_le(&mut bytes, 240, 0.0);
    write_f32_le(&mut bytes, 244, 457.3188);
    write_f32_le(&mut bytes, 248, 1.0);
    write_f32_le(&mut bytes, 252, 5.0);
    write_f32_le(&mut bytes, 256, 0.75);
    write_f32_le(&mut bytes, 260, 12.0);
    write_f32_le(&mut bytes, 264, -0.2);
    write_f32_le(&mut bytes, 268, 0.9);
    write_f32_le(&mut bytes, 400, 2.0);
    bytes
}

pub(crate) fn test_scene_cb0_with_camera_and_fade(
    cam_y: f32,
    snow_mud_fade: f32,
) -> TerrainSceneCb0Decoded {
    let mut bytes = valid_scene_cb0_bytes();
    write_f32_le(&mut bytes, 212, cam_y);
    write_f32_le(&mut bytes, 248, snow_mud_fade);
    decode_terrain_scene_cb0(
        &bytes,
        &terrain_scene_cb0_layout(&minimal_standard_functions_source()).expect("CB0 layout"),
    )
    .expect("decode TerrainSceneCB0")
}

pub(crate) fn test_post_diffuse_resources(
    scene_cb0: TerrainSceneCb0Decoded,
) -> TerrainPostDiffuseResources {
    TerrainPostDiffuseResources {
        snow_mud_data: RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![0, 255, 0, 255],
        },
        snow_texture: single_pixel_rgba_chain([128, 128, 255, 255]),
        city_lights_snow_noise: single_pixel_rgba_chain([0, 0, 0, 255]),
        mud_diffuse_gloss: single_pixel_rgba_chain([255, 0, 0, 255]),
        mud_normal_spec: single_pixel_rgba_chain([128, 128, 255, 255]),
        province_secondary_color_map: single_pixel_rgba_chain([0, 0, 0, 0]),
        gradient_border_channel1: single_pixel_rgba_chain([0, 0, 0, 0]),
        gradient_border_channel2: single_pixel_rgba_chain([0, 0, 0, 0]),
        light_index_map: RgbaImage {
            width: LIGHT_INDEX_TEXTURE_SIZE,
            height: LIGHT_INDEX_TEXTURE_SIZE,
            pixels: vec![255; LIGHT_INDEX_TEXTURE_SIZE * LIGHT_INDEX_TEXTURE_SIZE * 4],
        },
        light_data_map: Rgba32fImage {
            width: LIGHT_DATA_TEXTURE_SIZE,
            height: 1,
            pixels: vec![[0.0; 4]; LIGHT_DATA_TEXTURE_SIZE],
        },
        shadow_map: TerrainShadowMapState::NullSrv,
        scene_cb0,
        point_light_cb2: TerrainPointLightCb2Decoded {
            grid_start_inv_cell_size: [0.0, 0.0, 1.0, 1.0],
        },
        constants: test_map_frame_shader_constants(),
    }
}

pub(crate) fn test_pixel_input_with_prepos(prepos: Vec3) -> TerrainPixelShaderInput {
    TerrainPixelShaderInput {
        uv: Vec2 { x: 0.5, y: 0.5 },
        uv2: Vec2 { x: 0.5, y: 0.5 },
        prepos,
        v_shadow_proj: Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
        v_screen_coord: Vec4 {
            x: 0.5,
            y: 0.5,
            z: 1.0,
            w: 1.0,
        },
    }
}

pub(crate) fn test_pixel_derivatives() -> TerrainPixelShaderDerivatives {
    TerrainPixelShaderDerivatives {
        prepos_dx: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        prepos_dy: Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    }
}

pub(crate) fn write_identity_float4x4(bytes: &mut [u8], offset: usize) {
    for row in 0..4 {
        for col in 0..4 {
            write_f32_le(
                bytes,
                offset + row * 16 + col * 4,
                if row == col { 1.0 } else { 0.0 },
            );
        }
    }
}

pub(crate) fn write_f32_le(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn minimal_pdxmap_sampler_source() -> String {
    r#"
            PixelShader =
            {
                Samplers =
                {
                    TerrainDiffuse =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    HeightNormal =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    TerrainColorTint =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    SnowTexture =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    TerrainNormal =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    TerrainIDMap =
                    {
                        MagFilter = "Point"
                        MinFilter = "Point"
                        MipFilter = "None"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    SnowMudData =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    CityLightsAndSnowNoise =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    MudNormalSpec =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    MudDiffuseGloss =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    ProvinceSecondaryColorMap =
                    {
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                }
            }
        "#
    .to_string()
}

pub(crate) fn minimal_pdxwater_sampler_source() -> String {
    r#"
            PixelShader =
            {
                Samplers =
                {
                    HeightTexture =
                    {
                        Index = 0
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    LeanTexture1 =
                    {
                        Index = 1
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    LeanTexture2 =
                    {
                        Index = 2
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    ProvinceSecondaryColorMap =
                    {
                        Index = 3
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    SpecularMap =
                    {
                        Index = 4
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    WaterRefraction =
                    {
                        Index = 5
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    IceDiffuse =
                    {
                        Index = 6
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    IceNoise =
                    {
                        Index = 7
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    ReflectionCubeMap =
                    {
                        Index = 8
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                        Type = "Cube"
                    }
                    SnowMudTexture =
                    {
                        Index = 9
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                    }
                    LightIndexMap =
                    {
                        Index = 10
                        MagFilter = "Point"
                        MinFilter = "Point"
                        MipFilter = "Point"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    LightDataMap =
                    {
                        Index = 11
                        MagFilter = "Point"
                        MinFilter = "Point"
                        MipFilter = "Point"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    GradientBorderChannel1 =
                    {
                        Index = 12
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    GradientBorderChannel2 =
                    {
                        Index = 13
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    GradientBorderChannel3 =
                    {
                        Index = 14
                        MagFilter = "Point"
                        MinFilter = "Point"
                        MipFilter = "Point"
                        AddressU = "Clamp"
                        AddressV = "Clamp"
                    }
                    ShadowMap =
                    {
                        Index = 15
                        MagFilter = "Linear"
                        MinFilter = "Linear"
                        MipFilter = "Linear"
                        AddressU = "Wrap"
                        AddressV = "Wrap"
                        Type = "Shadow"
                    }
                }
            }
        "#
    .to_string()
}

pub(crate) fn minimal_map_source_bytes(source: &str) -> Vec<u8> {
    match source {
        "gfx/FX/pdxmap.shader" => minimal_pdxmap_sampler_source().into_bytes(),
        "gfx/FX/standardfuncsgfx.fxh" => minimal_standard_functions_source().into_bytes(),
        BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE => minimal_graphics_defines_source().into_bytes(),
        "gfx/FX/fow.fxh" => b"
                static const float3 BRIGHT_FOW_COLOR = float3(0.98, 0.93, 0.93);
                static const float3 DARK_FOW_COLOR = float3(0.08, 0.08, 0.1);
            "
        .to_vec(),
        _ => b"shader source".to_vec(),
    }
}

pub(crate) fn temp_test_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "hoi4_renderer_gate_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

pub(crate) fn write_synthetic_terrain_id_map_game_files(root: &Path, terrain_palette: u8) {
    let terrain_bmp = root.join("map").join("terrain.bmp");
    fs::create_dir_all(terrain_bmp.parent().expect("terrain.bmp parent"))
        .expect("create terrain dir");
    write_indexed_bmp(
        &terrain_bmp,
        TERRAIN_ID_MAP_WIDTH,
        TERRAIN_ID_MAP_HEIGHT,
        terrain_palette,
    );

    let terrain_definitions = root.join("common").join("terrain").join("00_terrain.txt");
    fs::create_dir_all(
        terrain_definitions
            .parent()
            .expect("terrain definitions parent"),
    )
    .expect("create terrain definitions dir");
    fs::write(&terrain_definitions, synthetic_terrain_definitions_text())
        .expect("write terrain definitions");

    let default_map = root.join("map").join("default.map");
    fs::write(default_map, "terrain = \"terrain.bmp\"\n").expect("write default.map");
}

/// An indexed BMP whose every pixel is chosen by the caller.
pub(crate) fn write_indexed_bmp_with(
    path: &Path,
    width: usize,
    height: usize,
    value_at: impl Fn(usize, usize) -> u8,
) {
    let row_stride = (width + 3) & !3;
    let palette_bytes = 256usize * 4;
    let pixel_offset = 14usize + 40usize + palette_bytes;
    let image_bytes = row_stride.checked_mul(height).expect("indexed BMP byte count");
    let file_size = pixel_offset + image_bytes;
    let mut file = fs::File::create(path).expect("create indexed BMP");
    file.write_all(b"BM").expect("write BMP signature");
    file.write_all(&(file_size as u32).to_le_bytes()).expect("file size");
    file.write_all(&[0u8; 4]).expect("reserved");
    file.write_all(&(pixel_offset as u32).to_le_bytes()).expect("pixel offset");
    file.write_all(&40u32.to_le_bytes()).expect("header size");
    file.write_all(&(width as i32).to_le_bytes()).expect("width");
    file.write_all(&(height as i32).to_le_bytes()).expect("height");
    file.write_all(&1u16.to_le_bytes()).expect("planes");
    file.write_all(&8u16.to_le_bytes()).expect("bpp");
    file.write_all(&0u32.to_le_bytes()).expect("compression");
    file.write_all(&(image_bytes as u32).to_le_bytes()).expect("image size");
    file.write_all(&2835u32.to_le_bytes()).expect("x ppm");
    file.write_all(&2835u32.to_le_bytes()).expect("y ppm");
    file.write_all(&256u32.to_le_bytes()).expect("colour count");
    file.write_all(&256u32.to_le_bytes()).expect("important colours");
    for palette in 0..=255u8 {
        file.write_all(&[palette, palette, palette, 0]).expect("palette");
    }
    // BMP rows run bottom-up, and the family's decoders undo that, so the
    // callback is given the coordinates the PRODUCERS use: `value_at(x, z)` is
    // what a sampler asked about world (x, z) reads.
    for row in 0..height {
        let mut line = vec![0u8; row_stride];
        for x in 0..width {
            line[x] = value_at(x, row);
        }
        file.write_all(&line).expect("row");
    }
}

pub(crate) fn write_indexed_bmp(path: &Path, width: usize, height: usize, value: u8) {
    let row_stride = (width + 3) & !3;
    let palette_bytes = 256usize * 4;
    let pixel_offset = 14usize + 40usize + palette_bytes;
    let image_bytes = row_stride
        .checked_mul(height)
        .expect("indexed BMP image byte count");
    let file_size = pixel_offset + image_bytes;
    let mut file = fs::File::create(path).expect("create indexed BMP");
    file.write_all(b"BM").expect("write BMP signature");
    file.write_all(&(file_size as u32).to_le_bytes())
        .expect("write BMP file size");
    file.write_all(&[0u8; 4]).expect("write BMP reserved");
    file.write_all(&(pixel_offset as u32).to_le_bytes())
        .expect("write BMP pixel offset");
    file.write_all(&40u32.to_le_bytes())
        .expect("write DIB header size");
    file.write_all(&(width as i32).to_le_bytes())
        .expect("write BMP width");
    file.write_all(&(height as i32).to_le_bytes())
        .expect("write BMP height");
    file.write_all(&1u16.to_le_bytes())
        .expect("write BMP planes");
    file.write_all(&8u16.to_le_bytes()).expect("write BMP bpp");
    file.write_all(&0u32.to_le_bytes())
        .expect("write BMP compression");
    file.write_all(&(image_bytes as u32).to_le_bytes())
        .expect("write BMP image size");
    file.write_all(&2835u32.to_le_bytes())
        .expect("write BMP x ppm");
    file.write_all(&2835u32.to_le_bytes())
        .expect("write BMP y ppm");
    file.write_all(&256u32.to_le_bytes())
        .expect("write BMP color count");
    file.write_all(&256u32.to_le_bytes())
        .expect("write BMP important color count");
    for palette in 0..=255u8 {
        file.write_all(&[palette, palette, palette, 0])
            .expect("write BMP palette");
    }
    let mut row = vec![value; row_stride];
    row[width..].fill(0);
    for _ in 0..height {
        file.write_all(&row).expect("write BMP row");
    }
}

fn synthetic_terrain_definitions_text() -> String {
    [
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
    .into_iter()
    .map(|(palette, texture)| {
        format!("terrain_{palette} = {{ color = {{ {palette} 0 0 }} texture = {texture} }}\n")
    })
    .collect::<Vec<_>>()
    .join("")
}

pub(crate) fn le_i16_bytes(values: &[i16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(crate) fn le_u16_bytes(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(crate) fn le_f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(crate) fn test_position_vertex_bytes() -> Vec<u8> {
    le_i16_bytes(&[
        0, 0, 0, 0,
        1, 0, 0, 0,
        0, 1, 1, 1,
    ])
}

pub(crate) fn test_height_vertex_bytes() -> Vec<u8> {
    le_i16_bytes(&[
        1000, 2000,
        1100, 2100,
        1200, 2200,
    ])
}

pub(crate) fn test_terrain_draw_cb1_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(le_f32_bytes(&[
        1.0, 0.0, 0.0, 0.0,
        0.0, 1.0, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
        10.0, 20.0, 2.0, 0.0,
        1.0, 0.0, 0.0, 0.0,
        0.0, 0.1, 8.0, 0.0,
    ]));
    bytes
}

pub(crate) fn minimal_pdxmap_fxh_source() -> String {
    r#"
            ConstantBuffer( 1, 32 )
            {
                float4x4 ShadowMapTextureMatrix;
                float4 QuadOffset_Scale_IsDetail;
                float4 vSnap;
                float4 vBorderLookup_HeightScale_UseMultisample_SeasonLerp;
            }
        "#
    .to_string()
}

pub(crate) fn test_map_frame_shader_constants() -> MapFrameTerrainShaderConstants {
    MapFrameTerrainShaderConstants {
        map_size_x: 64.0,
        map_size_y: 64.0,
        map_pow2_x: 1.0,
        map_pow2_y: 1.0,
        fow_pow2_x: 1.0,
        fow_pow2_y: 1.0,
    }
}

pub(crate) fn minimal_graphics_defines_source() -> String {
    r#"
            NDefines = NDefines or {}
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
            for k, v in pairs(NDefines_Graphics) do NDefines[k] = v end
        "#
    .to_string()
}

pub(crate) fn vanilla_source_fixture_bytes(source: &str) -> Vec<u8> {
    if source == "gfx/FX/pdxmap.fxh" {
        minimal_pdxmap_fxh_source().into_bytes()
    } else if source == PDXWATER_SHADER_SOURCE {
        minimal_pdxwater_sampler_source().into_bytes()
    } else if source == "gfx/FX/standardfuncsgfx.fxh" {
        minimal_standard_functions_source().into_bytes()
    } else if source == BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE {
        minimal_graphics_defines_source().into_bytes()
    } else {
        format!("// {source}\n").into_bytes()
    }
}

pub(crate) fn write_vanilla_source_fixture(root: &Path, source: &str) {
    let path = source
        .split('/')
        .fold(root.join("vanilla"), |path, part| path.join(part));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create vanilla source evidence dir");
    }
    fs::write(path, vanilla_source_fixture_bytes(source)).expect("write vanilla source evidence");
}

pub(crate) fn write_valid_map_frame_bundle<F>(root: &Path, mutate: F) -> PathBuf
where
    F: FnOnce(MapFrameRuntimeBundle) -> MapFrameRuntimeBundle,
{
    let position_vertices = test_position_vertex_bytes();
    let height_vertices = test_height_vertex_bytes();
    let index_bytes = le_u16_bytes(&[0, 1, 2]);
    let terrain_scene_cb0 = valid_scene_cb0_bytes();
    let terrain_draw_cb1 = test_terrain_draw_cb1_bytes();
    for draw_index in 0..2 {
        fs::write(
            root.join(format!(
                "map_frame_draw{draw_index:03}_ia_vertex_buffer_slot0.bin"
            )),
            &position_vertices,
        )
        .expect("write slot0 vertex buffer");
        fs::write(
            root.join(format!(
                "map_frame_draw{draw_index:03}_ia_vertex_buffer_slot1.bin"
            )),
            &height_vertices,
        )
        .expect("write slot1 vertex buffer");
        fs::write(
            root.join(format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin")),
            &index_bytes,
        )
        .expect("write index buffer");
        fs::write(
            root.join(format!(
                "map_frame_draw{draw_index:03}_vs_constant_buffer_slot0.bin"
            )),
            &terrain_scene_cb0,
        )
        .expect("write VS terrain scene constant buffer");
        fs::write(
            root.join(format!(
                "map_frame_draw{draw_index:03}_vs_constant_buffer_slot1.bin"
            )),
            &terrain_draw_cb1,
        )
        .expect("write VS terrain draw constant buffer");
        fs::write(
            root.join(format!(
                "map_frame_draw{draw_index:03}_ps_constant_buffer_slot0.bin"
            )),
            &terrain_scene_cb0,
        )
        .expect("write PS constant buffer");
    }
    fs::write(root.join("terrain_id_map.rgba"), [4u8; 16]).expect("write TerrainIDMap");
    fs::write(
        root.join("pdxmap_pixel_shader_0123456789abcdef.bin"),
        [9u8; 128],
    )
    .expect("write shader bytecode");
    fs::write(
        root.join("pdxmap_vertex_shader_fedcba9876543210.bin"),
        [7u8; 96],
    )
    .expect("write vertex shader bytecode");
    for source in MAP_FRAME_REQUIRED_VANILLA_SOURCES {
        write_vanilla_source_fixture(root, source);
    }

    let slot0_hash =
        fnv1a64_file(&root.join("map_frame_draw000_ia_vertex_buffer_slot0.bin")).unwrap();
    let slot1_hash =
        fnv1a64_file(&root.join("map_frame_draw000_ia_vertex_buffer_slot1.bin")).unwrap();
    let index_hash = fnv1a64_file(&root.join("map_frame_draw000_ia_index_buffer.bin")).unwrap();
    let terrain_hash = fnv1a64_file(&root.join("terrain_id_map.rgba")).unwrap();
    let shader_hash = fnv1a64_file(&root.join("pdxmap_pixel_shader_0123456789abcdef.bin")).unwrap();
    let vertex_shader_hash =
        fnv1a64_file(&root.join("pdxmap_vertex_shader_fedcba9876543210.bin")).unwrap();
    let ps_cb0_hash =
        fnv1a64_file(&root.join("map_frame_draw000_ps_constant_buffer_slot0.bin")).unwrap();
    let vs_cb0_hash =
        fnv1a64_file(&root.join("map_frame_draw000_vs_constant_buffer_slot0.bin")).unwrap();
    let vs_cb1_hash =
        fnv1a64_file(&root.join("map_frame_draw000_vs_constant_buffer_slot1.bin")).unwrap();
    let reflection_resources = BTreeMap::from([("TerrainIDMap".to_string(), 5)]);
    let vertex_shader_input_parameters = vec![
        MapFrameShaderInputParameter {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            register: 0,
            component_type: D3D_REGISTER_COMPONENT_SINT32,
            mask: 0x0f,
            read_write_mask: 0x0f,
        },
        MapFrameShaderInputParameter {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 0,
            register: 1,
            component_type: D3D_REGISTER_COMPONENT_SINT32,
            mask: 0x03,
            read_write_mask: 0x03,
        },
    ];

    let make_draw = |draw_index: u32| MapFrameDraw {
        draw_index,
        draw_call: "DrawIndexed".to_string(),
        draw_count: 3,
        draw_start: 0,
        base_vertex_location: 0,
        per_instance_count: None,
        instance_count: None,
        start_instance_location: None,
        topology: Some(4),
        shader: MapFrameShader {
            stage: "PS".to_string(),
            pointer: "0x1".to_string(),
            bytecode_file: "pdxmap_pixel_shader_0123456789abcdef.bin".to_string(),
            byte_len: 128,
            fnv1a64: shader_hash.clone(),
            reflection_resources: reflection_resources.clone(),
            input_parameters: Vec::new(),
            constant_buffers: BTreeMap::new(),
            source_evidence: MAP_FRAME_SHADER_SOURCES
                .iter()
                .map(|source| (*source).to_string())
                .collect(),
        },
        vertex_shader: Some(MapFrameShader {
            stage: "VS".to_string(),
            pointer: "0x10".to_string(),
            bytecode_file: "pdxmap_vertex_shader_fedcba9876543210.bin".to_string(),
            byte_len: 96,
            fnv1a64: vertex_shader_hash.clone(),
            reflection_resources: BTreeMap::new(),
            input_parameters: vertex_shader_input_parameters.clone(),
            constant_buffers: BTreeMap::from([(TERRAIN_DRAW_CB1_LAYOUT_SOURCE.to_string(), 1)]),
            source_evidence: MAP_FRAME_SHADER_SOURCES
                .iter()
                .map(|source| (*source).to_string())
                .collect(),
        }),
        input_layout: MapFrameInputLayout {
            pointer: "0x2".to_string(),
            elements: valid_input_elements(),
        },
        vertex_buffers: BTreeMap::from([
            (
                "0".to_string(),
                MapFrameBufferBinding {
                    stage: "IA".to_string(),
                    slot: 0,
                    declared_name: None,
                    file: Some(format!(
                        "map_frame_draw{draw_index:03}_ia_vertex_buffer_slot0.bin"
                    )),
                    byte_len: position_vertices.len(),
                    fnv1a64: Some(slot0_hash.clone()),
                    stride: Some(8),
                    offset: Some(0),
                    dxgi_format: None,
                    bind_flags: 1,
                    usage: 0,
                    cpu_access_flags: 0,
                },
            ),
            (
                "1".to_string(),
                MapFrameBufferBinding {
                    stage: "IA".to_string(),
                    slot: 1,
                    declared_name: None,
                    file: Some(format!(
                        "map_frame_draw{draw_index:03}_ia_vertex_buffer_slot1.bin"
                    )),
                    byte_len: height_vertices.len(),
                    fnv1a64: Some(slot1_hash.clone()),
                    stride: Some(4),
                    offset: Some(0),
                    dxgi_format: None,
                    bind_flags: 1,
                    usage: 0,
                    cpu_access_flags: 0,
                },
            ),
        ]),
        index_buffer: Some(MapFrameIndexBufferBinding {
            file: Some(format!("map_frame_draw{draw_index:03}_ia_index_buffer.bin")),
            byte_len: 6,
            fnv1a64: Some(index_hash.clone()),
            dxgi_format: 57,
            offset: 0,
            bind_flags: 2,
            usage: 0,
            cpu_access_flags: 0,
        }),
        constant_buffers: vec![
            MapFrameBufferBinding {
                stage: "VS".to_string(),
                slot: 0,
                declared_name: Some(TERRAIN_SCENE_CB0_SYMBOL.to_string()),
                file: Some(format!(
                    "map_frame_draw{draw_index:03}_vs_constant_buffer_slot0.bin"
                )),
                byte_len: terrain_scene_cb0.len(),
                fnv1a64: Some(vs_cb0_hash.clone()),
                stride: None,
                offset: None,
                dxgi_format: None,
                bind_flags: D3D11_BIND_CONSTANT_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
            },
            MapFrameBufferBinding {
                stage: "VS".to_string(),
                slot: 1,
                declared_name: Some(TERRAIN_DRAW_CB1_LAYOUT_SOURCE.to_string()),
                file: Some(format!(
                    "map_frame_draw{draw_index:03}_vs_constant_buffer_slot1.bin"
                )),
                byte_len: terrain_draw_cb1.len(),
                fnv1a64: Some(vs_cb1_hash.clone()),
                stride: None,
                offset: None,
                dxgi_format: None,
                bind_flags: D3D11_BIND_CONSTANT_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
            },
            MapFrameBufferBinding {
                stage: "PS".to_string(),
                slot: 0,
                declared_name: Some(TERRAIN_SCENE_CB0_SYMBOL.to_string()),
                file: Some(format!(
                    "map_frame_draw{draw_index:03}_ps_constant_buffer_slot0.bin"
                )),
                byte_len: terrain_scene_cb0.len(),
                fnv1a64: Some(ps_cb0_hash.clone()),
                stride: None,
                offset: None,
                dxgi_format: None,
                bind_flags: D3D11_BIND_CONSTANT_BUFFER,
                usage: 0,
                cpu_access_flags: 0,
            },
        ],
        shader_resources: vec![MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 5,
            declared_name: Some("TerrainIDMap".to_string()),
            is_null: false,
            view_desc: Some(MapFrameShaderResourceViewDesc {
                format: 87,
                view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                most_detailed_mip: 0,
                mip_levels: 1,
                first_array_slice: 0,
                array_size: 1,
                num_cubes: 0,
            }),
            file: Some("terrain_id_map.rgba".to_string()),
            byte_len: 16,
            fnv1a64: Some(terrain_hash.clone()),
            dxgi_format: 87,
            width: 2,
            height: 2,
            mip_count: 1,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string()),
            subresources: vec![MapFrameTextureSubresource {
                mip_slice: 0,
                array_slice: 0,
                file_offset: 0,
                byte_len: 16,
                row_pitch: 8,
                depth_pitch: 16,
                width: 2,
                height: 2,
            }],
            bind_flags: 8,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
        }],
        samplers: vec![MapFrameSamplerBinding {
            stage: "PS".to_string(),
            slot: 5,
            filter: 0,
            address_u: 3,
            address_v: 3,
            address_w: 3,
            mip_lod_bias: 0.0,
            max_anisotropy: 1,
            comparison_func: 0,
            border_color: [0.0; 4],
            min_lod: 0.0,
            max_lod: f32::MAX,
        }],
        draw_state: None,
        post_draw_output_merger: None,
    };
    let draws = vec![make_draw(0), make_draw(1)];
    let vanilla_sources = MAP_FRAME_REQUIRED_VANILLA_SOURCES
        .iter()
        .map(|source| {
            let copied = format!("vanilla/{source}");
            let hash = fnv1a64_file(&root.join(&copied)).unwrap();
            let byte_len = fs::metadata(root.join(&copied)).unwrap().len() as usize;
            (
                (*source).to_string(),
                MapFrameFileEvidence {
                    file: copied,
                    byte_len,
                    fnv1a64: hash,
                    provenance: "unit_test".to_string(),
                    resolved_from: None,
                    effective_vfs_verified: None,
                },
            )
        })
        .collect();
    let post_pass_file = "map_frame_post_pass_render_target_slot000.d3d11";
    fs::write(root.join(post_pass_file), [0u8; 8]).expect("write post-pass target");
    let post_pass_hash = fnv1a64_file(&root.join(post_pass_file)).unwrap();
    let mut post_pass_output_merger = valid_output_merger_state();
    post_pass_output_merger.render_targets[0].post_pass_payload =
        Some(MapFrameOutputMergerTexturePayload {
            file: post_pass_file.to_string(),
            byte_len: 8,
            fnv1a64: post_pass_hash.clone(),
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
            provenance: "unit_test_post_pass_reference".to_string(),
        });

    let frame = MapFrameRuntimeBundle {
        schema: MAP_FRAME_BUNDLE_SCHEMA.to_string(),
        provenance: "unit_test".to_string(),
        capture_moment: TERRAIN_CAPTURE_MOMENT.to_string(),
        session_manifest: MAP_CAPTURE_SESSION_FILE.to_string(),
        shader_cache_constants: Some(MapFrameShaderCacheConstants {
            map_size_x: 2.0,
            map_size_y: 2.0,
            map_pow2_x: 1.0,
            map_pow2_y: 1.0,
            fow_pow2_x: 1.0,
            fow_pow2_y: 1.0,
            terrain_id_srv_slot: 5,
            cache_files: vec!["dx11_win32/ps_5_0/map_t5.scache".to_string()],
            provenance: "unit_test_shader_cache_scan".to_string(),
            extraction: "unit_test_shader_cache_control".to_string(),
        }),
        active_draw: draws[1].clone(),
        draws,
        post_pass_output_merger: Some(post_pass_output_merger),
        vanilla_sources,
        runtime_files: BTreeMap::from([
            (
                "terrain_id_map.rgba".to_string(),
                MapFrameFileEvidence {
                    file: "terrain_id_map.rgba".to_string(),
                    byte_len: 16,
                    fnv1a64: terrain_hash,
                    provenance: "unit_test".to_string(),
                    resolved_from: None,
                    effective_vfs_verified: None,
                },
            ),
            (
                post_pass_file.to_string(),
                MapFrameFileEvidence {
                    file: post_pass_file.to_string(),
                    byte_len: 8,
                    fnv1a64: post_pass_hash,
                    provenance: "unit_test_post_pass_reference".to_string(),
                    resolved_from: None,
                    effective_vfs_verified: None,
                },
            ),
            (
                "pdxmap_pixel_shader_0123456789abcdef.bin".to_string(),
                MapFrameFileEvidence {
                    file: "pdxmap_pixel_shader_0123456789abcdef.bin".to_string(),
                    byte_len: 128,
                    fnv1a64: shader_hash,
                    provenance: "unit_test".to_string(),
                    resolved_from: None,
                    effective_vfs_verified: None,
                },
            ),
            (
                "pdxmap_vertex_shader_fedcba9876543210.bin".to_string(),
                MapFrameFileEvidence {
                    file: "pdxmap_vertex_shader_fedcba9876543210.bin".to_string(),
                    byte_len: 96,
                    fnv1a64: vertex_shader_hash,
                    provenance: "unit_test".to_string(),
                    resolved_from: None,
                    effective_vfs_verified: None,
                },
            ),
        ]),
        blockers: Vec::new(),
        synthetic_payloads: BTreeMap::new(),
        vanilla_source_verification: None,
    };
    let mut frame = mutate(frame);
    if let Some(draw) = frame
        .draws
        .iter_mut()
        .find(|draw| draw.draw_index == frame.active_draw.draw_index)
    {
        *draw = frame.active_draw.clone();
    }
    let session = MapCaptureSession {
            schema: MAP_CAPTURE_SESSION_SCHEMA.to_string(),
            provenance: "unit_test".to_string(),
            required_first_moment: TERRAIN_CAPTURE_MOMENT.to_string(),
            moments: vec![MapCaptureSessionMoment {
                name: TERRAIN_CAPTURE_MOMENT.to_string(),
                status: if frame.blockers.is_empty() {
                    "captured_pending_renderer_validation".to_string()
                } else {
                    "blocked".to_string()
                },
                bundle_file: MAP_FRAME_BUNDLE_FILE.to_string(),
                required_payload: MAP_CAPTURE_REQUIRED_PAYLOAD
                    .iter()
                    .map(|payload| (*payload).to_string())
                    .collect(),
                blockers: frame.blockers.clone(),
            }],
            deferred_moments: [
                "water_pass",
                "rivers_pass",
                "borders_pass",
                "trees_pass",
                "cities_pass",
                "overlays_pass",
            ]
            .iter()
            .map(|name| MapCaptureDeferredMoment {
                name: (*name).to_string(),
                status: "deferred_until_terrain_active_pass_validated".to_string(),
                required_after: TERRAIN_CAPTURE_MOMENT.to_string(),
                blocker:
                    "terrain bundle capture and renderer validation must complete before this layer is captured"
                        .to_string(),
            })
            .collect(),
        };
    write_json(&root.join(MAP_CAPTURE_SESSION_FILE), &session)
        .expect("write map capture session manifest");
    let path = root.join(MAP_FRAME_BUNDLE_FILE);
    write_json(&path, &frame).expect("write map-frame bundle");
    path
}

pub(crate) fn water_refraction_resource_mut(
    draw: &mut MapFrameDraw,
) -> Option<&mut MapFrameTextureBinding> {
    draw.shader_resources.iter_mut().find(|resource| {
        resource.declared_name.as_deref() == Some(WATER_REFRACTION_RESOURCE_SYMBOL)
    })
}

pub(crate) fn mutate_water_sampler<F>(frame: &mut MapFrameRuntimeBundle, name: &str, mut mutate: F)
where
    F: FnMut(&mut MapFrameSamplerBinding),
{
    for draw in &mut frame.draws {
        if let Some(sampler) = water_sampler_mut(draw, name) {
            mutate(sampler);
        }
    }
    if let Some(sampler) = water_sampler_mut(&mut frame.active_draw, name) {
        mutate(sampler);
    }
}

pub(crate) fn move_water_binding_to_slot(
    frame: &mut MapFrameRuntimeBundle,
    name: &str,
    new_slot: u32,
) {
    for draw in &mut frame.draws {
        move_draw_water_binding_to_slot(draw, name, new_slot);
    }
    move_draw_water_binding_to_slot(&mut frame.active_draw, name, new_slot);
}

pub(crate) fn move_draw_water_binding_to_slot(draw: &mut MapFrameDraw, name: &str, new_slot: u32) {
    let Some(old_slot) = draw
        .shader
        .reflection_resources
        .insert(name.to_string(), new_slot)
    else {
        return;
    };
    for resource in &mut draw.shader_resources {
        if resource.declared_name.as_deref() == Some(name) {
            resource.slot = new_slot;
        }
    }
    for sampler in &mut draw.samplers {
        if sampler.stage == "PS" && sampler.slot == old_slot {
            sampler.slot = new_slot;
        }
    }
}

pub(crate) fn add_reflected_water_resource(
    frame: &mut MapFrameRuntimeBundle,
    name: &str,
    view_dimension: u32,
    misc_flags: u32,
) {
    for draw in &mut frame.draws {
        add_draw_reflected_water_resource(draw, name, view_dimension, misc_flags);
    }
    add_draw_reflected_water_resource(&mut frame.active_draw, name, view_dimension, misc_flags);
}

pub(crate) fn add_draw_reflected_water_resource(
    draw: &mut MapFrameDraw,
    name: &str,
    view_dimension: u32,
    misc_flags: u32,
) {
    let expectation =
        pdxwater_sampler_expectation(name).expect("optional water resource has contract");
    draw.shader
        .reflection_resources
        .insert(name.to_string(), expectation.index);

    let mut resource = draw.shader_resources[0].clone();
    resource.slot = expectation.index;
    resource.declared_name = Some(name.to_string());
    resource.misc_flags = misc_flags;
    let view = resource
        .view_desc
        .get_or_insert(MapFrameShaderResourceViewDesc {
            format: resource.dxgi_format,
            view_dimension,
            most_detailed_mip: 0,
            mip_levels: 1,
            first_array_slice: 0,
            array_size: 1,
            num_cubes: 0,
        });
    view.view_dimension = view_dimension;
    draw.shader_resources.push(resource);

    let mut sampler = draw.samplers[0].clone();
    sampler.slot = expectation.index;
    sampler.filter = expectation.mode.filter();
    sampler.address_u = expectation.mode.address_u();
    sampler.address_v = expectation.mode.address_v();
    sampler.address_w = expectation.mode.address_u();
    draw.samplers.push(sampler);
}

pub(crate) fn water_sampler_mut<'a>(
    draw: &'a mut MapFrameDraw,
    name: &str,
) -> Option<&'a mut MapFrameSamplerBinding> {
    let slot = draw.shader.reflection_resources.get(name).copied()?;
    draw.samplers
        .iter_mut()
        .find(|sampler| sampler.stage == "PS" && sampler.slot == slot)
}

pub(crate) fn strip_water_refraction_payloads(frame: &mut MapFrameRuntimeBundle) {
    for draw in &mut frame.draws {
        if let Some(resource) = water_refraction_resource_mut(draw) {
            strip_water_refraction_payload(resource);
        }
    }
    if let Some(resource) = water_refraction_resource_mut(&mut frame.active_draw) {
        strip_water_refraction_payload(resource);
    }
}

pub(crate) fn strip_water_refraction_payload(resource: &mut MapFrameTextureBinding) {
    resource.file = None;
    resource.byte_len = 0;
    resource.fnv1a64 = None;
    resource.payload_format = None;
    resource.subresources.clear();
}

pub(crate) fn write_valid_water_map_frame_bundle<F>(root: &Path, mutate: F) -> PathBuf
where
    F: FnOnce(MapFrameRuntimeBundle) -> MapFrameRuntimeBundle,
{
    let terrain_path = write_valid_map_frame_bundle(root, |frame| frame);
    let mut frame: MapFrameRuntimeBundle =
        serde_json::from_slice(&fs::read(&terrain_path).expect("read terrain frame"))
            .expect("parse terrain frame");
    for source in WATER_FRAME_REQUIRED_VANILLA_SOURCES {
        let path = source
            .split('/')
            .fold(root.join("vanilla"), |path, part| path.join(part));
        if !path.exists() {
            write_vanilla_source_fixture(root, source);
        }
        let copied = format!("vanilla/{source}");
        frame.vanilla_sources.insert(
            source.to_string(),
            MapFrameFileEvidence {
                file: copied.clone(),
                byte_len: fs::metadata(root.join(&copied)).unwrap().len() as usize,
                fnv1a64: fnv1a64_file(&root.join(&copied)).unwrap(),
                provenance: "unit_test_water".to_string(),
                resolved_from: None,
                effective_vfs_verified: None,
            },
        );
    }
    let water_ps = "pdxwater_pixel_shader_1111111111111111.bin";
    let water_vs = "pdxwater_vertex_shader_2222222222222222.bin";
    fs::write(root.join(water_ps), [11u8; 128]).expect("write water PS bytecode");
    fs::write(root.join(water_vs), [22u8; 96]).expect("write water VS bytecode");
    let water_ps_hash = fnv1a64_file(&root.join(water_ps)).unwrap();
    let water_vs_hash = fnv1a64_file(&root.join(water_vs)).unwrap();
    frame.runtime_files.insert(
        water_ps.to_string(),
        MapFrameFileEvidence {
            file: water_ps.to_string(),
            byte_len: 128,
            fnv1a64: water_ps_hash.clone(),
            provenance: "unit_test_water".to_string(),
            resolved_from: None,
            effective_vfs_verified: None,
        },
    );
    frame.runtime_files.insert(
        water_vs.to_string(),
        MapFrameFileEvidence {
            file: water_vs.to_string(),
            byte_len: 96,
            fnv1a64: water_vs_hash.clone(),
            provenance: "unit_test_water".to_string(),
            resolved_from: None,
            effective_vfs_verified: None,
        },
    );
    frame.capture_moment = WATER_CAPTURE_MOMENT.to_string();
    frame.shader_cache_constants = None;
    for draw in &mut frame.draws {
        draw.shader.bytecode_file = water_ps.to_string();
        draw.shader.byte_len = 128;
        draw.shader.fnv1a64 = water_ps_hash.clone();
        draw.shader.reflection_resources = WATER_FRAME_REQUIRED_SHADER_RESOURCES
            .iter()
            .map(|resource| {
                (
                    (*resource).to_string(),
                    pdxwater_sampler_expectation(resource)
                        .expect("water fixture resource has sampler contract")
                        .index,
                )
            })
            .collect();
        draw.shader.constant_buffers = BTreeMap::from([(TERRAIN_SCENE_CB0_SYMBOL.to_string(), 0)]);
        draw.shader.source_evidence = WATER_FRAME_SHADER_SOURCES
            .iter()
            .map(|source| (*source).to_string())
            .collect();
        let vertex_shader = draw.vertex_shader.as_mut().expect("vertex shader");
        vertex_shader.bytecode_file = water_vs.to_string();
        vertex_shader.byte_len = 96;
        vertex_shader.fnv1a64 = water_vs_hash.clone();
        vertex_shader.input_parameters = vec![MapFrameShaderInputParameter {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            register: 0,
            component_type: D3D_REGISTER_COMPONENT_SINT32,
            mask: 0x03,
            read_write_mask: 0x03,
        }];
        vertex_shader.constant_buffers =
            BTreeMap::from([(TERRAIN_SCENE_CB0_SYMBOL.to_string(), 0)]);
        vertex_shader.source_evidence = WATER_FRAME_SHADER_SOURCES
            .iter()
            .map(|source| (*source).to_string())
            .collect();
        draw.input_layout.elements = vec![RuntimeInputElementMeta {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R16G16_SINT,
            input_slot: 0,
            aligned_byte_offset: 0,
            input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
            instance_data_step_rate: 0,
        }];
        let resource_template = draw.shader_resources[0].clone();
        let water_refraction_file = "map_frame_post_pass_render_target_slot000.d3d11";
        let water_refraction_hash =
            fnv1a64_file(&root.join(water_refraction_file)).expect("hash water refraction");
        draw.shader_resources = WATER_FRAME_REQUIRED_SHADER_RESOURCES
            .iter()
            .map(|resource| {
                let expectation = pdxwater_sampler_expectation(resource)
                    .expect("water fixture resource has sampler contract");
                let mut binding = resource_template.clone();
                binding.slot = expectation.index;
                binding.declared_name = Some((*resource).to_string());
                if *resource == WATER_REFRACTION_RESOURCE_SYMBOL {
                    binding.view_desc = Some(MapFrameShaderResourceViewDesc {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        view_dimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                        most_detailed_mip: 0,
                        mip_levels: 1,
                        first_array_slice: 0,
                        array_size: 1,
                        num_cubes: 0,
                    });
                    binding.file = Some(water_refraction_file.to_string());
                    binding.byte_len = 8;
                    binding.fnv1a64 = Some(water_refraction_hash.clone());
                    binding.dxgi_format = DXGI_FORMAT_B8G8R8A8_UNORM;
                    binding.width = 2;
                    binding.height = 1;
                    binding.mip_count = 1;
                    binding.array_size = 1;
                    binding.sample_count = 1;
                    binding.sample_quality = 0;
                    binding.payload_format = Some(NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string());
                    binding.subresources = vec![MapFrameTextureSubresource {
                        mip_slice: 0,
                        array_slice: 0,
                        file_offset: 0,
                        byte_len: 8,
                        row_pitch: 8,
                        depth_pitch: 8,
                        width: 2,
                        height: 1,
                    }];
                    binding.bind_flags = D3D11_BIND_SHADER_RESOURCE;
                    binding.usage = D3D11_USAGE_DEFAULT;
                    binding.cpu_access_flags = 0;
                    binding.misc_flags = 0;
                }
                binding
            })
            .collect();
        let sampler_template = draw.samplers[0].clone();
        draw.samplers = WATER_FRAME_REQUIRED_SHADER_RESOURCES
            .iter()
            .map(|resource| {
                let expectation = pdxwater_sampler_expectation(resource)
                    .expect("water fixture resource has sampler contract");
                let mut sampler = sampler_template.clone();
                sampler.slot = expectation.index;
                let mode = expectation.mode;
                sampler.filter = mode.filter();
                sampler.address_u = mode.address_u();
                sampler.address_v = mode.address_v();
                sampler.address_w = mode.address_u();
                sampler
            })
            .collect();
        let mut state = valid_map_frame_draw_state();
        state.depth_stencil = MapFrameDepthStencilState {
            default_state: false,
            stencil_ref: 0,
            desc: Some(valid_depth_enabled_desc()),
        };
        state.output_merger = Some(valid_output_merger_state());
        draw.draw_state = Some(state);
    }
    frame.active_draw = frame.draws[0].clone();
    let mut frame = mutate(frame);
    if let Some(draw) = frame
        .draws
        .iter_mut()
        .find(|draw| draw.draw_index == frame.active_draw.draw_index)
    {
        *draw = frame.active_draw.clone();
    }
    let session_path = root.join(MAP_CAPTURE_SESSION_FILE);
    let mut session: MapCaptureSession =
        serde_json::from_slice(&fs::read(&session_path).expect("read session"))
            .expect("parse session");
    session.moments.push(MapCaptureSessionMoment {
        name: WATER_CAPTURE_MOMENT.to_string(),
        status: if frame.blockers.is_empty() {
            "captured_pending_renderer_validation".to_string()
        } else {
            "blocked".to_string()
        },
        bundle_file: WATER_FRAME_BUNDLE_FILE.to_string(),
        required_payload: MAP_CAPTURE_REQUIRED_PAYLOAD
            .iter()
            .map(|payload| (*payload).to_string())
            .collect(),
        blockers: frame.blockers.clone(),
    });
    write_json(&session_path, &session).expect("write water session manifest");
    let path = root.join(WATER_FRAME_BUNDLE_FILE);
    write_json(&path, &frame).expect("write water map-frame bundle");
    path
}

pub(crate) fn write_valid_map_layer_frame_bundle<F>(
    root: &Path,
    spec: MapFrameBundleValidationSpec,
    shader_sources: &[&str],
    required_resources: &[&str],
    pixel_shader_file: &str,
    vertex_shader_file: &str,
    mutate: F,
) -> PathBuf
where
    F: FnOnce(MapFrameRuntimeBundle) -> MapFrameRuntimeBundle,
{
    let water_path = write_valid_water_map_frame_bundle(root, |frame| frame);
    let mut frame: MapFrameRuntimeBundle =
        serde_json::from_slice(&fs::read(&water_path).expect("read water frame"))
            .expect("parse water frame");
    for source in spec.required_vanilla_sources {
        let path = source
            .split('/')
            .fold(root.join("vanilla"), |path, part| path.join(part));
        if !path.exists() {
            write_vanilla_source_fixture(root, source);
        }
        let copied = format!("vanilla/{source}");
        frame.vanilla_sources.insert(
            (*source).to_string(),
            MapFrameFileEvidence {
                file: copied.clone(),
                byte_len: fs::metadata(root.join(&copied)).unwrap().len() as usize,
                fnv1a64: fnv1a64_file(&root.join(&copied)).unwrap(),
                provenance: "unit_test_map_layer".to_string(),
                resolved_from: None,
                effective_vfs_verified: None,
            },
        );
    }

    fs::write(root.join(pixel_shader_file), [31u8; 128]).expect("write layer PS bytecode");
    fs::write(root.join(vertex_shader_file), [32u8; 96]).expect("write layer VS bytecode");
    let ps_hash = fnv1a64_file(&root.join(pixel_shader_file)).unwrap();
    let vs_hash = fnv1a64_file(&root.join(vertex_shader_file)).unwrap();
    frame.runtime_files.insert(
        pixel_shader_file.to_string(),
        MapFrameFileEvidence {
            file: pixel_shader_file.to_string(),
            byte_len: 128,
            fnv1a64: ps_hash.clone(),
            provenance: "unit_test_map_layer".to_string(),
            resolved_from: None,
            effective_vfs_verified: None,
        },
    );
    frame.runtime_files.insert(
        vertex_shader_file.to_string(),
        MapFrameFileEvidence {
            file: vertex_shader_file.to_string(),
            byte_len: 96,
            fnv1a64: vs_hash.clone(),
            provenance: "unit_test_map_layer".to_string(),
            resolved_from: None,
            effective_vfs_verified: None,
        },
    );

    frame.capture_moment = spec.capture_moment.to_string();
    frame.shader_cache_constants = None;
    for draw in &mut frame.draws {
        draw.shader.bytecode_file = pixel_shader_file.to_string();
        draw.shader.byte_len = 128;
        draw.shader.fnv1a64 = ps_hash.clone();
        draw.shader.reflection_resources = required_resources
            .iter()
            .enumerate()
            .map(|(slot, resource)| ((*resource).to_string(), slot as u32))
            .collect();
        draw.shader.source_evidence = shader_sources
            .iter()
            .map(|source| (*source).to_string())
            .collect();
        let vertex_shader = draw.vertex_shader.as_mut().expect("vertex shader");
        vertex_shader.bytecode_file = vertex_shader_file.to_string();
        vertex_shader.byte_len = 96;
        vertex_shader.fnv1a64 = vs_hash.clone();
        vertex_shader.source_evidence = shader_sources
            .iter()
            .map(|source| (*source).to_string())
            .collect();

        let resource_template = draw.shader_resources[0].clone();
        draw.shader_resources = required_resources
            .iter()
            .enumerate()
            .map(|(slot, resource)| {
                let mut binding = resource_template.clone();
                binding.slot = slot as u32;
                binding.declared_name = Some((*resource).to_string());
                binding
            })
            .collect();
        let sampler_template = draw.samplers[0].clone();
        draw.samplers = required_resources
            .iter()
            .enumerate()
            .map(|(slot, _)| {
                let mut sampler = sampler_template.clone();
                sampler.slot = slot as u32;
                sampler
            })
            .collect();
        let mut state = valid_map_frame_draw_state();
        state.depth_stencil = MapFrameDepthStencilState {
            default_state: false,
            stencil_ref: 0,
            desc: Some(valid_depth_enabled_desc()),
        };
        state.output_merger = Some(valid_output_merger_state());
        draw.draw_state = Some(state);
    }
    frame.active_draw = frame.draws[0].clone();
    let mut frame = mutate(frame);
    if let Some(draw) = frame
        .draws
        .iter_mut()
        .find(|draw| draw.draw_index == frame.active_draw.draw_index)
    {
        *draw = frame.active_draw.clone();
    }

    let session_path = root.join(MAP_CAPTURE_SESSION_FILE);
    let mut session: MapCaptureSession =
        serde_json::from_slice(&fs::read(&session_path).expect("read session"))
            .expect("parse session");
    session.moments.push(MapCaptureSessionMoment {
        name: spec.capture_moment.to_string(),
        status: if frame.blockers.is_empty() {
            "captured_pending_renderer_validation".to_string()
        } else {
            "blocked".to_string()
        },
        bundle_file: spec.bundle_file.to_string(),
        required_payload: MAP_CAPTURE_REQUIRED_PAYLOAD
            .iter()
            .map(|payload| (*payload).to_string())
            .collect(),
        blockers: frame.blockers.clone(),
    });
    write_json(&session_path, &session).expect("write map-layer session manifest");
    let path = root.join(spec.bundle_file);
    write_json(&path, &frame).expect("write map-layer bundle");
    path
}

pub(crate) fn rewrite_terrain_bundle_with_valid_output_merger_state(path: &Path) {
    let mut frame: MapFrameRuntimeBundle =
        serde_json::from_slice(&fs::read(path).expect("read terrain frame"))
            .expect("parse terrain frame");
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
    write_json(path, &frame).expect("rewrite terrain bundle with OM state");
}

pub(crate) fn add_valid_water_output_merger_state(root: &Path, frame: &mut MapFrameRuntimeBundle) {
    let mut state = valid_map_frame_draw_state();
    state.depth_stencil = MapFrameDepthStencilState {
        default_state: false,
        stencil_ref: 0,
        desc: Some(valid_depth_enabled_desc()),
    };
    let mut output_merger = valid_output_merger_state();
    attach_water_pre_draw_payloads(root, frame, &mut output_merger);
    state.output_merger = Some(output_merger);
    frame.active_draw.draw_state = Some(state.clone());
    for draw in &mut frame.draws {
        draw.draw_state = Some(state.clone());
    }
}

pub(crate) fn attach_water_pre_draw_payloads(
    root: &Path,
    frame: &mut MapFrameRuntimeBundle,
    output_merger: &mut MapFrameOutputMergerState,
) {
    let render_file = "map_frame_water_pre_draw_render_target_slot000.d3d11";
    let depth_file = "map_frame_water_pre_draw_depth_stencil.d3d11";
    fs::write(root.join(render_file), [3u8; 8]).expect("write water pre-draw RT");
    fs::write(root.join(depth_file), [4u8; 8]).expect("write water pre-draw depth");
    let render_hash = fnv1a64_file(&root.join(render_file)).expect("hash water pre-draw RT");
    let depth_hash = fnv1a64_file(&root.join(depth_file)).expect("hash water pre-draw depth");
    output_merger.render_targets[0].pre_draw_payload = Some(water_pre_draw_payload(
        render_file,
        &render_hash,
        &output_merger.render_targets[0].desc,
    ));
    let depth_desc = output_merger
        .depth_stencil
        .as_ref()
        .expect("water depth target")
        .desc
        .clone();
    output_merger
        .depth_stencil
        .as_mut()
        .expect("water depth target")
        .pre_draw_payload = Some(water_pre_draw_payload(depth_file, &depth_hash, &depth_desc));
    for (file, hash) in [(render_file, render_hash), (depth_file, depth_hash)] {
        frame.runtime_files.insert(
            file.to_string(),
            MapFrameFileEvidence {
                file: file.to_string(),
                byte_len: 8,
                fnv1a64: hash,
                provenance: "unit_test_water_pre_draw".to_string(),
                resolved_from: None,
                effective_vfs_verified: None,
            },
        );
    }
}

pub(crate) fn water_pre_draw_payload(
    file: &str,
    fnv1a64: &str,
    desc: &MapFrameTexture2DDesc,
) -> MapFrameOutputMergerTexturePayload {
    MapFrameOutputMergerTexturePayload {
        file: file.to_string(),
        byte_len: 8,
        fnv1a64: fnv1a64.to_string(),
        payload_format: NATIVE_D3D11_TEXTURE_PAYLOAD_FORMAT.to_string(),
        subresources: vec![MapFrameTextureSubresource {
            mip_slice: 0,
            array_slice: 0,
            file_offset: 0,
            byte_len: 8,
            row_pitch: 8,
            depth_pitch: 8,
            width: desc.width,
            height: desc.height,
        }],
        provenance: "unit_test_water_pre_draw".to_string(),
    }
}

pub(crate) fn add_snow_mud_resource(root: &Path, frame: &mut MapFrameRuntimeBundle) {
    let bytes = [1u8; 16];
    fs::write(root.join("snow_mud_data.rgba"), bytes).expect("write SnowMudData");
    let hash = fnv1a64_file(&root.join("snow_mud_data.rgba")).expect("hash SnowMudData");
    frame
        .active_draw
        .shader
        .reflection_resources
        .insert(SNOW_MUD_DATA_SYMBOL.to_string(), 7);
    frame
        .active_draw
        .shader_resources
        .push(MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 7,
            declared_name: Some(SNOW_MUD_DATA_SYMBOL.to_string()),
            is_null: false,
            view_desc: None,
            file: Some("snow_mud_data.rgba".to_string()),
            byte_len: bytes.len(),
            fnv1a64: Some(hash.clone()),
            dxgi_format: DXGI_FORMAT_B8G8R8A8_UNORM,
            width: 2,
            height: 2,
            mip_count: 1,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: 8,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
        });
    frame.active_draw.samplers.push(MapFrameSamplerBinding {
        stage: "PS".to_string(),
        slot: 7,
        filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        address_u: D3D11_TEXTURE_ADDRESS_WRAP,
        address_v: D3D11_TEXTURE_ADDRESS_WRAP,
        address_w: D3D11_TEXTURE_ADDRESS_WRAP,
        mip_lod_bias: 0.0,
        max_anisotropy: 1,
        comparison_func: 0,
        border_color: [0.0; 4],
        min_lod: 0.0,
        max_lod: f32::MAX,
    });
    frame.runtime_files.insert(
        "snow_mud_data.rgba".to_string(),
        MapFrameFileEvidence {
            file: "snow_mud_data.rgba".to_string(),
            byte_len: bytes.len(),
            fnv1a64: hash,
            provenance: "unit_test".to_string(),
            resolved_from: None,
            effective_vfs_verified: None,
        },
    );
    sync_active_shader_bindings_to_draws(frame);
}

pub(crate) fn write_post_diffuse_material_bundle(root: &Path) {
    fs::create_dir_all(root).expect("create post-diffuse material bundle");
    let mut manifest = minimal_test_manifest();
    let mut contract = minimal_test_contract();
    for (symbol, source) in TERRAIN_POST_DIFFUSE_SHIPPED_SHADER_RESOURCES {
        let output = shader_rgba_output_name(source);
        let mip_output = shader_rgba_mip_output_name(source);
        manifest
            .files
            .shader_sources
            .insert(source.to_string(), output.clone());
        contract.shipped_sources.insert(
            source.to_string(),
            MaterialSourceMeta {
                output,
                mip_output: Some(mip_output.clone()),
                role: "unit_test_post_diffuse_material".to_string(),
                shader_samplers: vec![symbol.to_string()],
                dimensions: Dimensions {
                    width: 2,
                    height: 2,
                },
                dds_format: "DXT5".to_string(),
                mip_count: 2,
                preservation: "decoded_rgba_exact_2d_with_mip_chain".to_string(),
            },
        );
        fs::write(root.join(&manifest.files.shader_sources[source]), [3u8; 16])
            .expect("write post-diffuse material base output");
        write_rgba_mip_chain_raw(
            &root.join(mip_output),
            &[
                RgbaImage {
                    width: 2,
                    height: 2,
                    pixels: vec![4u8; 16],
                },
                RgbaImage {
                    width: 1,
                    height: 1,
                    pixels: vec![5u8; 4],
                },
            ],
        )
        .expect("write post-diffuse material mip chain");
    }
    fs::write(root.join(&manifest.files.terrain_id_map), [1u8; 16])
        .expect("write post-diffuse terrain id map");
    let runtime = minimal_test_runtime(&manifest, &root.join(&manifest.files.terrain_id_map));
    write_json(&root.join(&manifest.files.terrain_runtime), &runtime)
        .expect("write post-diffuse terrain runtime contract");
    write_json(&root.join("manifest.json"), &manifest).expect("write material manifest");
    write_json(&root.join("map_material_contract.json"), &contract)
        .expect("write material contract");
}

pub(crate) fn add_post_diffuse_shipped_resources(frame: &mut MapFrameRuntimeBundle) {
    for (symbol, slot, dxgi_format) in [
        (SNOW_TEXTURE_SYMBOL, 3, DXGI_FORMAT_BC3_UNORM),
        (
            CITY_LIGHTS_AND_SNOW_NOISE_SYMBOL,
            8,
            DXGI_FORMAT_BC3_UNORM_SRGB,
        ),
        (MUD_NORMAL_SPEC_SYMBOL, 9, DXGI_FORMAT_BC3_UNORM),
        (MUD_DIFFUSE_GLOSS_SYMBOL, 13, DXGI_FORMAT_BC3_UNORM_SRGB),
    ] {
        add_uncaptured_map_frame_resource(frame, symbol, slot, 2, 2, 2);
        frame
            .active_draw
            .shader_resources
            .iter_mut()
            .find(|resource| resource.slot == slot)
            .expect("added shipped resource")
            .dxgi_format = dxgi_format;
        add_map_frame_sampler(
            frame,
            slot,
            D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            D3D11_TEXTURE_ADDRESS_WRAP,
            D3D11_TEXTURE_ADDRESS_WRAP,
        );
    }
    sync_active_shader_bindings_to_draws(frame);
}

pub(crate) fn add_uncaptured_post_diffuse_runtime_resources(frame: &mut MapFrameRuntimeBundle) {
    for (symbol, slot) in [
        (PROVINCE_SECONDARY_COLOR_MAP_SYMBOL, 6),
        (GRADIENT_BORDER_CHANNEL1_SYMBOL, 14),
        (GRADIENT_BORDER_CHANNEL2_SYMBOL, 15),
        (LIGHT_INDEX_MAP_SYMBOL, 10),
        (LIGHT_DATA_MAP_SYMBOL, 11),
    ] {
        add_uncaptured_map_frame_resource(frame, symbol, slot, 2, 2, 1);
    }
    frame
        .active_draw
        .shader
        .reflection_resources
        .insert(SHADOW_MAP_SYMBOL.to_string(), 12);
    sync_active_shader_bindings_to_draws(frame);
}

pub(crate) fn add_null_shadow_map_resource(frame: &mut MapFrameRuntimeBundle) {
    frame
        .active_draw
        .shader
        .reflection_resources
        .insert(SHADOW_MAP_SYMBOL.to_string(), 12);
    frame
        .active_draw
        .shader_resources
        .push(MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot: 12,
            declared_name: Some(SHADOW_MAP_SYMBOL.to_string()),
            is_null: true,
            view_desc: None,
            file: None,
            byte_len: 0,
            fnv1a64: None,
            dxgi_format: 0,
            width: 0,
            height: 0,
            mip_count: 0,
            array_size: 0,
            sample_count: 0,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: 0,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
        });
    add_map_frame_sampler(
        frame,
        12,
        D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        D3D11_TEXTURE_ADDRESS_WRAP,
        D3D11_TEXTURE_ADDRESS_WRAP,
    );
    sync_active_shader_bindings_to_draws(frame);
}

pub(crate) fn sync_active_shader_bindings_to_draws(frame: &mut MapFrameRuntimeBundle) {
    for draw in &mut frame.draws {
        draw.shader.reflection_resources = frame.active_draw.shader.reflection_resources.clone();
        draw.shader_resources = frame.active_draw.shader_resources.clone();
        draw.samplers = frame.active_draw.samplers.clone();
    }
}

pub(crate) fn add_map_frame_sampler(
    frame: &mut MapFrameRuntimeBundle,
    slot: u32,
    filter: u32,
    address_u: u32,
    address_v: u32,
) {
    frame.active_draw.samplers.push(MapFrameSamplerBinding {
        stage: "PS".to_string(),
        slot,
        filter,
        address_u,
        address_v,
        address_w: address_v,
        mip_lod_bias: 0.0,
        max_anisotropy: 1,
        comparison_func: 0,
        border_color: [0.0; 4],
        min_lod: 0.0,
        max_lod: f32::MAX,
    });
}

pub(crate) fn add_uncaptured_map_frame_resource(
    frame: &mut MapFrameRuntimeBundle,
    symbol: &str,
    slot: u32,
    width: u32,
    height: u32,
    mip_count: u32,
) {
    frame
        .active_draw
        .shader
        .reflection_resources
        .insert(symbol.to_string(), slot);
    frame
        .active_draw
        .shader_resources
        .push(MapFrameTextureBinding {
            stage: "PS".to_string(),
            slot,
            declared_name: Some(symbol.to_string()),
            is_null: false,
            view_desc: None,
            file: None,
            byte_len: 0,
            fnv1a64: None,
            dxgi_format: DXGI_FORMAT_R8G8B8A8_UNORM,
            width,
            height,
            mip_count,
            array_size: 1,
            sample_count: 1,
            sample_quality: 0,
            payload_format: None,
            subresources: Vec::new(),
            bind_flags: 8,
            usage: 0,
            cpu_access_flags: 0,
            misc_flags: 0,
        });
}

pub(crate) fn valid_map_frame_draw_state() -> MapFrameDrawState {
    MapFrameDrawState {
        viewports: vec![MapFrameViewport {
            top_left_x: 0.0,
            top_left_y: 0.0,
            width: 5632.0,
            height: 2048.0,
            min_depth: 0.0,
            max_depth: 1.0,
        }],
        scissor_rects: vec![MapFrameScissorRect {
            left: 0,
            top: 0,
            right: 5632,
            bottom: 2048,
        }],
        rasterizer: MapFrameRasterizerState {
            default_state: false,
            desc: Some(MapFrameRasterizerDesc {
                fill_mode: 3,
                cull_mode: 1,
                front_counter_clockwise: false,
                depth_bias: 0,
                depth_bias_clamp: 0.0,
                slope_scaled_depth_bias: 0.0,
                depth_clip_enable: true,
                scissor_enable: false,
                multisample_enable: false,
                antialiased_line_enable: false,
            }),
        },
        blend: MapFrameBlendState {
            default_state: true,
            blend_factor: [1.0, 1.0, 1.0, 1.0],
            sample_mask: u32::MAX,
            desc: None,
        },
        depth_stencil: MapFrameDepthStencilState {
            default_state: true,
            stencil_ref: 0,
            desc: None,
        },
        output_merger: None,
    }
}

pub(crate) fn valid_depth_enabled_desc() -> MapFrameDepthStencilDesc {
    MapFrameDepthStencilDesc {
        depth_enable: true,
        depth_write_mask: 1,
        depth_func: 4,
        stencil_enable: false,
        stencil_read_mask: 255,
        stencil_write_mask: 255,
        front_face: MapFrameDepthStencilOpDesc {
            stencil_fail_op: 1,
            stencil_depth_fail_op: 1,
            stencil_pass_op: 1,
            stencil_func: 8,
        },
        back_face: MapFrameDepthStencilOpDesc {
            stencil_fail_op: 1,
            stencil_depth_fail_op: 1,
            stencil_pass_op: 1,
            stencil_func: 8,
        },
    }
}

pub(crate) fn valid_output_merger_state() -> MapFrameOutputMergerState {
    MapFrameOutputMergerState {
        render_targets: vec![MapFrameOutputMergerTexture {
            slot: Some(0),
            view_pointer: "0x1000".to_string(),
            resource_pointer: "0x2000".to_string(),
            desc: MapFrameTexture2DDesc {
                width: 2,
                height: 1,
                mip_levels: 1,
                array_size: 1,
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                sample_count: 1,
                sample_quality: 0,
                usage: 0,
                bind_flags: D3D11_BIND_RENDER_TARGET,
                cpu_access_flags: 0,
                misc_flags: 0,
            },
            view_desc: Some(MapFrameOutputMergerViewDesc {
                format: DXGI_FORMAT_B8G8R8A8_UNORM as u32,
                view_dimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                mip_slice: 0,
                first_array_slice: 0,
                array_size: 1,
                flags: 0,
            }),
            pre_draw_payload: None,
            post_pass_payload: None,
            last_render_target_clear: Some(MapFrameRenderTargetClear {
                sequence: 1,
                color: [0.0, 0.0, 0.0, 1.0],
            }),
            last_depth_stencil_clear: None,
        }],
        depth_stencil: Some(MapFrameOutputMergerTexture {
            slot: None,
            view_pointer: "0x3000".to_string(),
            resource_pointer: "0x4000".to_string(),
            desc: MapFrameTexture2DDesc {
                width: 2,
                height: 1,
                mip_levels: 1,
                array_size: 1,
                format: 45,
                sample_count: 1,
                sample_quality: 0,
                usage: 0,
                bind_flags: D3D11_BIND_DEPTH_STENCIL,
                cpu_access_flags: 0,
                misc_flags: 0,
            },
            view_desc: Some(MapFrameOutputMergerViewDesc {
                format: 45,
                view_dimension: D3D11_DSV_DIMENSION_TEXTURE2D,
                mip_slice: 0,
                first_array_slice: 0,
                array_size: 1,
                flags: 0,
            }),
            pre_draw_payload: None,
            post_pass_payload: None,
            last_render_target_clear: None,
            last_depth_stencil_clear: Some(MapFrameDepthStencilClear {
                sequence: 2,
                clear_flags: D3D11_CLEAR_DEPTH,
                depth: 1.0,
                stencil: 0,
            }),
        }),
    }
}

pub(crate) fn minimal_test_manifest() -> Manifest {
    Manifest {
        schema: "battle-plan-rust-map-bundle-v1".to_string(),
        dimensions: Dimensions {
            width: 4,
            height: 4,
        },
        province_id_encoding: "little-endian u32 per pixel".to_string(),
        files: BundleFiles {
            province_ids: "province_ids.r32".to_string(),
            terrain_id_map: "terrain_id_map.rgba".to_string(),
            terrain_runtime: "terrain_runtime.json".to_string(),
            snow_mud_data: "snow_mud_data.rgba".to_string(),
            terrain_scene_cb0: "terrain_scene_cb0.bin".to_string(),
            province_secondary_color_map: "province_secondary_color_map.rgba".to_string(),
            gradient_border_channel1: "gradient_border_channel1.rgba".to_string(),
            gradient_border_channel2: "gradient_border_channel2.rgba".to_string(),
            gradient_border_channel3: "gradient_border_channel3.rgba".to_string(),
            light_index_map: "light_index_map.rgba".to_string(),
            light_data_map: "light_data_map.rgba".to_string(),
            shadow_map: "shadow_map.rgba".to_string(),
            terrain_indices: "terrain_indices.r8".to_string(),
            terrain_texture_ids: "terrain_texture_ids.r8".to_string(),
            heightmap: "heightmap.r8".to_string(),
            rivers: "rivers.r8".to_string(),
            height_normal: "height_normal.rgbx".to_string(),
            cities: "cities.r8".to_string(),
            trees: "trees.r8".to_string(),
            tree_tint: "tree_tint.rgbx".to_string(),
            tree_season: "tree_season.rgbx".to_string(),
            terrain_diffuse_atlas: "terrain_diffuse_atlas0.rgbx".to_string(),
            terrain_normal_atlas: "terrain_normal_atlas0.rgbx".to_string(),
            terrain_diffuse_atlas_mips: "terrain_diffuse_atlas0_mips.rgba".to_string(),
            terrain_normal_atlas_mips: "terrain_normal_atlas0_mips.rgba".to_string(),
            shader_sources: BTreeMap::new(),
            map_sources: BTreeMap::new(),
            map_object_meta: "map_object_meta.json".to_string(),
            map_material_contract: "map_material_contract.json".to_string(),
            province_to_state: "province_to_state.json".to_string(),
            state_meta: "state_meta.json".to_string(),
            state_owner_1936: "state_owner_1936.json".to_string(),
            country_colors: "country_colors.json".to_string(),
            province_definitions: "province_definitions.json".to_string(),
            province_to_strategic_region: "province_to_strategic_region.json".to_string(),
            strategic_region_to_provinces: "strategic_region_to_provinces.json".to_string(),
            strategic_region_meta: "strategic_region_meta.json".to_string(),
        },
        counts: BundleCounts {
            definition_rows: 0,
            distinct_ids_in_provinces_bmp: 0,
            states: 0,
            state_owners_1936: 0,
            province_to_state: 0,
            country_colors: 0,
            strategic_regions: 0,
            province_to_strategic_region: 0,
            unknown_pixels: 0,
        },
        source: BundleSource {
            game_dir_name: "the game".to_string(),
            default_map: "map/default.map".to_string(),
            definition_csv_fnv1a64: String::new(),
            provinces_bmp_fnv1a64: String::new(),
            visual_input_fnv1a64: BTreeMap::new(),
            visual_input_dimensions: BTreeMap::new(),
        },
        modset: None,
        viewer_note: String::new(),
    }
}

pub(crate) fn minimal_test_contract() -> MapMaterialContract {
    MapMaterialContract {
        implemented_visible_level: "production_terrain_renderer_gate".to_string(),
        shader_constants: BTreeMap::from([
            (
                "TERRAIN_TILE_FREQ".to_string(),
                test_shader_constant(TERRAIN_TILE_FREQ),
            ),
            (
                "TEXELS_PER_TILE".to_string(),
                test_shader_constant(TEXELS_PER_TILE),
            ),
            (
                "MAP_NUM_TILES".to_string(),
                test_shader_constant(MAP_NUM_TILES as f32),
            ),
            (
                "ATLAS_TEXEL_POW2_EXPONENT".to_string(),
                test_shader_constant(ATLAS_TEXEL_POW2_EXPONENT),
            ),
            (
                "COLORMAP_OVERLAY_STRENGTH".to_string(),
                test_shader_constant(COLORMAP_OVERLAY_STRENGTH),
            ),
            (
                "COLORMAP_MUD_OVERLAY_STRENGTH".to_string(),
                test_shader_constant(COLORMAP_MUD_OVERLAY_STRENGTH),
            ),
            ("MUD_TILING".to_string(), test_shader_constant(MUD_TILING)),
            (
                "MUD_NORMAL_CUTOFF".to_string(),
                test_shader_constant(MUD_NORMAL_CUTOFF),
            ),
            (
                "MUD_STRENGHTEN".to_string(),
                test_shader_constant(MUD_STRENGHTEN),
            ),
            ("MUD_CAM_MIN".to_string(), test_shader_constant(MUD_CAM_MIN)),
            ("MUD_CAM_MAX".to_string(), test_shader_constant(MUD_CAM_MAX)),
            (
                "SNOW_OPACITY_MIN".to_string(),
                test_shader_constant(SNOW_OPACITY_MIN),
            ),
            (
                "SNOW_OPACITY_MAX".to_string(),
                test_shader_constant(SNOW_OPACITY_MAX),
            ),
            (
                "SNOW_CAM_MIN".to_string(),
                test_shader_constant(SNOW_CAM_MIN),
            ),
            (
                "SNOW_CAM_MAX".to_string(),
                test_shader_constant(SNOW_CAM_MAX),
            ),
            (
                "SNOW_START_HEIGHT".to_string(),
                test_shader_constant(SNOW_START_HEIGHT),
            ),
            (
                "SNOW_RIDGE_START_HEIGHT".to_string(),
                test_shader_constant(SNOW_RIDGE_START_HEIGHT),
            ),
            (
                "SNOW_NORMAL_START".to_string(),
                test_shader_constant(SNOW_NORMAL_START),
            ),
            ("SNOW_CLIFFS".to_string(), test_shader_constant(SNOW_CLIFFS)),
            (
                "SNOW_SPEC_GLOSS_MULT".to_string(),
                test_shader_constant(SNOW_SPEC_GLOSS_MULT),
            ),
            ("SNOW_TILING".to_string(), test_shader_constant(SNOW_TILING)),
            (
                "SNOW_NOISE_TILING".to_string(),
                test_shader_constant(SNOW_NOISE_TILING),
            ),
            (
                "SNOW_FROST_MIN_EFFECT".to_string(),
                test_shader_constant(SNOW_FROST_MIN_EFFECT),
            ),
            (
                "CITY_LIGHTS_TILING".to_string(),
                test_shader_constant(CITY_LIGHTS_TILING),
            ),
            (
                "CITY_LIGHTS_INTENSITY".to_string(),
                test_shader_constant(CITY_LIGHTS_INTENSITY),
            ),
            ("FOG_BEGIN".to_string(), test_shader_constant(FOG_BEGIN)),
            ("FOG_END".to_string(), test_shader_constant(FOG_END)),
            ("FOG_MAX".to_string(), test_shader_constant(FOG_MAX)),
            ("GB_CAM_MIN".to_string(), test_shader_constant(GB_CAM_MIN)),
            ("GB_CAM_MAX".to_string(), test_shader_constant(GB_CAM_MAX)),
            (
                "GB_CAM_MAX_FILLING_CLAMP".to_string(),
                test_shader_constant(GB_CAM_MAX_FILLING_CLAMP),
            ),
            (
                "GB_THRESHOLD".to_string(),
                test_shader_constant(GB_THRESHOLD),
            ),
            (
                "GB_THRESHOLD2".to_string(),
                test_shader_constant(GB_THRESHOLD2),
            ),
            (
                "GB_OPACITY_NEAR".to_string(),
                test_shader_constant(GB_OPACITY_NEAR),
            ),
            (
                "GB_OPACITY_FAR".to_string(),
                test_shader_constant(GB_OPACITY_FAR),
            ),
            (
                "GB_STRENGTH_CH1".to_string(),
                test_shader_constant(GB_STRENGTH_CH1),
            ),
            (
                "GB_STRENGTH_CH2".to_string(),
                test_shader_constant(GB_STRENGTH_CH2),
            ),
            (
                "GB_FIRST_LAYER_PRIORITY".to_string(),
                test_shader_constant(GB_FIRST_LAYER_PRIORITY),
            ),
            (
                "SEC_MAP_TILE".to_string(),
                test_shader_constant(SEC_MAP_TILE),
            ),
            (
                "BORDER_NIGHT_DESATURATION_MAX".to_string(),
                test_shader_constant(BORDER_NIGHT_DESATURATION_MAX),
            ),
            (
                "BORDER_FOW_REMOVAL_FACTOR".to_string(),
                test_shader_constant(BORDER_FOW_REMOVAL_FACTOR),
            ),
            (
                "BORDER_LIGHT_REMOVAL_FACTOR".to_string(),
                test_shader_constant(BORDER_LIGHT_REMOVAL_FACTOR),
            ),
        ]),
        shader_vector3_constants: BTreeMap::from([
            (
                "SNOW_COLOR".to_string(),
                test_shader_vector3_constant(SNOW_COLOR),
            ),
            (
                "FOG_COLOR".to_string(),
                test_shader_vector3_constant(FOG_COLOR),
            ),
        ]),
        shipped_sources: BTreeMap::new(),
        generated_runtime_inputs: generated_runtime_material_inputs(),
    }
}

pub(crate) fn test_material_source_meta(
    mip_count: u32,
    preservation: &str,
    mip_output: Option<String>,
) -> MaterialSourceMeta {
    MaterialSourceMeta {
        output: "shader_test_material.rgba".to_string(),
        mip_output,
        role: "unit_test_material".to_string(),
        shader_samplers: Vec::new(),
        dimensions: Dimensions {
            width: 2,
            height: 2,
        },
        dds_format: "DXT5".to_string(),
        mip_count,
        preservation: preservation.to_string(),
    }
}

pub(crate) fn test_shader_constant(value: f32) -> ShaderConstantMeta {
    ShaderConstantMeta {
        value,
        source: "gfx/FX/constants.fxh".to_string(),
    }
}

pub(crate) fn test_shader_vector3_constant(value: Vec3) -> ShaderVector3Meta {
    ShaderVector3Meta {
        value: [value.x, value.y, value.z],
        source: "gfx/FX/constants.fxh".to_string(),
    }
}

pub(crate) fn minimal_test_runtime(
    manifest: &Manifest,
    terrain_id_map: &Path,
) -> TerrainRuntimeInputs {
    TerrainRuntimeInputs {
        schema: TERRAIN_RUNTIME_SCHEMA.to_string(),
        terrain_id_map: TerrainIdMapRuntimeMeta {
            file: manifest.files.terrain_id_map.clone(),
            width: 2,
            height: 2,
            byte_len: 16,
            fnv1a64: fnv1a64_file(terrain_id_map).expect("hash terrain id map"),
            format: TERRAIN_ID_MAP_FORMAT.to_string(),
            channel_order: terrain_id_map_channel_order(),
            sampler: terrain_id_map_sampler(),
            source: RuntimeInputSource {
                provenance: "unit_test_capture".to_string(),
                extraction: "runtime_extracted_texture_no_static_fallback".to_string(),
            },
        },
        map_pow2: MapPow2RuntimeMeta {
            x: 2.75,
            y: 1.0,
            source: RuntimeInputSource {
                provenance: "unit_test_capture".to_string(),
                extraction: "runtime_extracted_shader_constant_no_static_fallback".to_string(),
            },
        },
        fow_pow2: MapPow2RuntimeMeta {
            x: 1.0,
            y: 1.0,
            source: RuntimeInputSource {
                provenance: "unit_test_capture".to_string(),
                extraction: "runtime_extracted_shader_constant_no_static_fallback".to_string(),
            },
        },
        runtime_textures: BTreeMap::new(),
        runtime_constant_buffers: BTreeMap::new(),
        runtime_vertex_buffers: BTreeMap::new(),
        runtime_input_layouts: BTreeMap::new(),
    }
}

pub(crate) fn write_test_terrain_vertex_buffer_capture(root: &Path, stride: u32) -> PathBuf {
    let capture_dir = root.join("capture");
    fs::create_dir_all(&capture_dir).expect("create vertex capture dir");
    let sample = capture_dir.join("terrain_vertex_buffer_sample.bin");
    fs::write(&sample, (0u8..64).collect::<Vec<_>>()).expect("write vertex sample");
    let capture = TerrainVertexBufferCaptureFile {
        schema: TERRAIN_VERTEX_BUFFER_CAPTURE_SCHEMA.to_string(),
        symbol: TERRAIN_VERTEX_BUFFER_SYMBOL.to_string(),
        file: "terrain_vertex_buffer_sample.bin".to_string(),
        source_byte_width: 4096,
        captured_byte_len: 64,
        fnv1a64: fnv1a64_file(&sample).expect("hash vertex sample"),
        stride,
        offset: 0,
        minimum_stride_source:
            "gfx/FX/pdxmap.shader:VS_INPUT_TERRAIN_NOTEXTURE:int4 position + int2 height"
                .to_string(),
        draw_call: "DrawIndexed".to_string(),
        draw_count: 1470,
        draw_start: 0,
        base_vertex_location: 90246,
        terrain_id_srv_slot: 5,
        source: "ia_get_vertex_buffers_slot0_during_active_pdxmap_draw".to_string(),
    };
    let capture_json = capture_dir.join("terrain_vertex_buffer_capture.json");
    write_json(&capture_json, &capture).expect("write vertex capture json");
    capture_json
}

pub(crate) fn write_test_terrain_input_layout_capture(
    root: &Path,
    vertex_stride: u32,
    elements: Vec<RuntimeInputElementMeta>,
) -> PathBuf {
    let capture_dir = root.join("capture");
    fs::create_dir_all(&capture_dir).expect("create input layout capture dir");
    let capture = TerrainInputLayoutCaptureFile {
        schema: TERRAIN_INPUT_LAYOUT_CAPTURE_SCHEMA.to_string(),
        symbol: TERRAIN_INPUT_LAYOUT_SYMBOL.to_string(),
        element_count: elements.len(),
        elements,
        vertex_stride,
        vertex_offset: 0,
        shader_input_source: TERRAIN_INPUT_LAYOUT_SHADER_SOURCE.to_string(),
        draw_call: "DrawIndexed".to_string(),
        draw_count: 1470,
        draw_start: 0,
        base_vertex_location: 90246,
        terrain_id_srv_slot: 5,
        source: "create_input_layout_descriptor_bound_during_active_pdxmap_draw".to_string(),
    };
    let capture_json = capture_dir.join("terrain_input_layout_capture.json");
    write_json(&capture_json, &capture).expect("write input layout capture json");
    capture_json
}

pub(crate) fn add_valid_vertex_and_input_layout_runtime(
    root: &Path,
    runtime: &mut TerrainRuntimeInputs,
) {
    let sample = root.join("terrain_vertex_buffer_sample.bin");
    fs::write(&sample, (0u8..64).collect::<Vec<_>>()).expect("write vertex sample");
    runtime.runtime_vertex_buffers.insert(
        TERRAIN_VERTEX_BUFFER_SYMBOL.to_string(),
        RuntimeVertexBufferRuntimeMeta {
            file: "terrain_vertex_buffer_sample.bin".to_string(),
            source_byte_width: 4096,
            captured_byte_len: 64,
            fnv1a64: fnv1a64_file(&sample).expect("hash vertex sample"),
            format: RUNTIME_VERTEX_BUFFER_FORMAT.to_string(),
            stride: 8,
            offset: 0,
            minimum_stride_source: TERRAIN_INPUT_LAYOUT_SHADER_SOURCE.to_string(),
            draw_call: "DrawIndexed".to_string(),
            draw_count: 1470,
            draw_start: 0,
            base_vertex_location: 90246,
            terrain_id_srv_slot: 5,
            source: RuntimeInputSource {
                provenance: "unit_test_vertex_capture".to_string(),
                extraction: "ia_get_vertex_buffers_slot0_during_active_pdxmap_draw".to_string(),
            },
        },
    );
    runtime.runtime_input_layouts.insert(
        TERRAIN_INPUT_LAYOUT_SYMBOL.to_string(),
        RuntimeInputLayoutRuntimeMeta {
            element_count: 2,
            elements: valid_input_elements(),
            format: RUNTIME_INPUT_LAYOUT_FORMAT.to_string(),
            vertex_stride: 8,
            vertex_offset: 0,
            shader_input_source: TERRAIN_INPUT_LAYOUT_SHADER_SOURCE.to_string(),
            draw_call: "DrawIndexed".to_string(),
            draw_count: 1470,
            draw_start: 0,
            base_vertex_location: 90246,
            terrain_id_srv_slot: 5,
            source: RuntimeInputSource {
                provenance: "unit_test_input_layout_capture".to_string(),
                extraction: "create_input_layout_descriptor_bound_during_active_pdxmap_draw"
                    .to_string(),
            },
        },
    );
}

pub(crate) fn valid_input_elements() -> Vec<RuntimeInputElementMeta> {
    vec![
        RuntimeInputElementMeta {
            semantic_name: "POSITION".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R16G16B16A16_SINT,
            input_slot: 0,
            aligned_byte_offset: 0,
            input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
            instance_data_step_rate: 0,
        },
        RuntimeInputElementMeta {
            semantic_name: "TEXCOORD".to_string(),
            semantic_index: 0,
            format: DXGI_FORMAT_R16G16_SINT,
            input_slot: 1,
            aligned_byte_offset: 0,
            input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
            instance_data_step_rate: 0,
        },
    ]
}

pub(crate) fn write_minimal_runtime_import_bundle(root: &Path) -> Manifest {
    let mut manifest = minimal_test_manifest();
    manifest.files.shader_sources = BTreeMap::from([
        (
            "map/terrain/atlas0.dds".to_string(),
            "shader_map_terrain_atlas0.rgba".to_string(),
        ),
        (
            "map/terrain/atlas_normal0.dds".to_string(),
            "shader_map_terrain_atlas_normal0.rgba".to_string(),
        ),
        (
            "map/terrain/colormap_rgb_cityemissivemask_a.dds".to_string(),
            "shader_map_terrain_colormap_rgb_cityemissivemask_a.rgba".to_string(),
        ),
    ]);
    manifest.files.map_sources = BTreeMap::from([
        (
            "gfx/FX/constants.fxh".to_string(),
            "source_gfx_fx_constants_fxh".to_string(),
        ),
        (
            "gfx/FX/pdxmap.shader".to_string(),
            "source_gfx_fx_pdxmap_shader".to_string(),
        ),
        (
            "gfx/FX/pdxmap.fxh".to_string(),
            "source_gfx_fx_pdxmap_fxh".to_string(),
        ),
        (
            "gfx/FX/standardfuncsgfx.fxh".to_string(),
            "source_gfx_fx_standardfuncsgfx_fxh".to_string(),
        ),
        (
            "gfx/FX/shadow.fxh".to_string(),
            "source_gfx_fx_shadow_fxh".to_string(),
        ),
        (
            "gfx/FX/fow.fxh".to_string(),
            "source_gfx_fx_fow_fxh".to_string(),
        ),
    ]);
    write_json(&root.join("manifest.json"), &manifest).expect("write manifest");
    write_json(
        &root.join("map_material_contract.json"),
        &minimal_test_contract(),
    )
    .expect("write contract");
    for output in manifest.files.shader_sources.values() {
        fs::write(root.join(output), [7u8]).expect("write shader source file");
    }
    for (source, output) in &manifest.files.map_sources {
        let bytes = minimal_map_source_bytes(source);
        fs::write(root.join(output), &bytes).expect("write map source file");
            crate::test_support::write_modvfs_fixture(&root.join("vanilla"), source, &bytes);
    }
    fs::write(root.join(&manifest.files.terrain_id_map), [1u8; 16]).expect("write terrain id map");
    let runtime = minimal_test_runtime(&manifest, &root.join(&manifest.files.terrain_id_map));
    write_json(&root.join(&manifest.files.terrain_runtime), &runtime)
        .expect("write terrain runtime contract");
    manifest
}

/// A vanilla-install resolver carrying a validated content admission over
/// `paths`, for the install gates whose production entry point calls
/// `require_complete_admitted_dir`.
pub(crate) fn admitted_vanilla_resolver(
    resolver: &GameFileResolver,
    paths: &std::collections::BTreeSet<String>,
    label: &str,
) -> (GameFileResolver, replay_core::ValidatedContentAdmission) {
    use replay_core::{
        ContentManifest, LayerStorageKind, RecordedContentLayer, RequiredContent,
        SourceGameIdentity, SourceProfileIdentity, UsedContentKey,
    };

    let effective_vfs_manifest_sha256 = "ab".repeat(32);
    let required_content = paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let actual = resolver
                .resolve_required_with_provenance(path, label)
                .unwrap_or_else(|error| panic!("{label} resolves {path}: {error}"));
            RequiredContent {
                content_key: format!("{label}-{index}"),
                kind: admitted_content_kind(path),
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.clone(),
                    normalized_game_relative_path: actual.logical_path.to_ascii_lowercase(),
                    winning_layer_ordinal: actual.source.ordinal as u32,
                    layer_kind: admitted_layer_kind(&actual.source.kind),
                    layer_id: actual.source.source_id,
                    source_sha256: actual
                        .source
                        .archive_sha256
                        .unwrap_or_else(|| actual.sha256.clone()),
                    payload_sha256: actual.sha256,
                    payload_bytes: actual.byte_len,
                },
                observation_count: 1,
                consumers: vec![label.to_string()],
            }
        })
        .collect::<Vec<_>>();
    let used_ordinals = required_content
        .iter()
        .map(|row| row.content.winning_layer_ordinal as usize)
        .collect::<std::collections::BTreeSet<_>>();
    let ordered_layers = resolver
        .active_sources()
        .into_iter()
        .map(|source| RecordedContentLayer {
            ordinal: source.ordinal as u32,
            layer_kind: admitted_layer_kind(&source.kind),
            layer_id: source.source_id.clone(),
            display_name: source.source_id,
            descriptor_sha256: source.descriptor_sha256,
            source_container_sha256: source.archive_sha256.clone(),
            storage_kind: if source.archive_sha256.is_some() {
                LayerStorageKind::ZipArchive
            } else {
                LayerStorageKind::Directory
            },
            replace_paths: source.replace_path,
            required_for_playback: used_ordinals.contains(&source.ordinal),
        })
        .collect();
    let source_game = SourceGameIdentity {
        version: "1.19.2.0".into(),
        build: "a729".into(),
        executable_sha256: "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
            .into(),
        build_profile: label.to_string(),
    };
    let mut manifest = ContentManifest {
        schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.into(),
        product_modset_sha256: "99".repeat(32),
        source_game: source_game.clone(),
        source_profile: SourceProfileIdentity {
            profile_id: label.to_string(),
            playset_id: "vanilla".into(),
            selection_sha256: "00".repeat(32),
        },
        effective_vfs_manifest_sha256,
        ordered_layers,
        dlc_selection: Vec::new(),
        localisation_language: "l_english".into(),
        required_settings: BTreeMap::new(),
        recorder_local_settings: BTreeMap::new(),
        required_content,
        manifest_digest_sha256: None,
    };
    manifest.source_profile.selection_sha256 = manifest.computed_selection_sha256().unwrap();
    manifest = manifest.with_computed_digest().unwrap();
    let inventory = resolver
        .playback_inventory(
            &manifest,
            crate::PlaybackEnvironment {
                source_game,
                source_profile: manifest.source_profile.clone(),
                dlc_selection: Vec::new(),
                localisation_language: "l_english".into(),
                settings: BTreeMap::new(),
            },
        )
        .unwrap();
    let admission = manifest.admit_playback(&inventory).unwrap();
    (
        resolver.clone().with_content_admission(&admission).unwrap(),
        admission,
    )
}

pub(crate) fn admitted_layer_kind(kind: &str) -> replay_core::ContentLayerKind {
    use replay_core::ContentLayerKind;
    match kind {
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => ContentLayerKind::Base,
    }
}

fn admitted_content_kind(path: &str) -> replay_core::RequiredContentKind {
    use replay_core::RequiredContentKind;
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "dds" | "tga" | "png" | "bmp" => RequiredContentKind::Texture,
        "fx" | "fxh" | "shader" => RequiredContentKind::ShaderSource,
        "gui" => RequiredContentKind::GuiDefinition,
        "gfx" => RequiredContentKind::Catalog,
        "yml" => RequiredContentKind::Localisation,
        "fnt" => RequiredContentKind::Font,
        "mesh" => RequiredContentKind::Mesh,
        "txt" | "lua" | "csv" | "asset" => RequiredContentKind::Definition,
        _ => RequiredContentKind::Other,
    }
}

pub(crate) fn write_modvfs_fixture(root: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

pub(crate) fn admit_modvfs_fixture(root: &Path) -> (GameFileResolver, replay_core::ValidatedContentAdmission) {
    let raw = GameFileResolver::with_mod_paths(root.join("vanilla"), vec![root.join("mod")]).unwrap();
    let paths = raw.audit_effective_dir_files_recursive("", None).unwrap().into_iter().collect();
    admitted_vanilla_resolver(&raw, &paths, "effective-fixture")
}

pub(crate) fn modvfs_shader(effect: &str, samplers: &str) -> String {
    format!(r#"
VertexShader = {{
    MainCode VS [[ float4 main(float4 p : POSITION, float4 t : TEXCOORD0) : SV_Position {{ return p + t; }} ]]
}}
PixelShader = {{
    {samplers}
    MainCode PS [[ float4 main() : SV_Target {{ return float4(1,1,1,1); }} ]]
}}
Effect {effect} {{ VertexShader = "VS" PixelShader = "PS" }}
"#)
}

pub(crate) fn write_modvfs_provinces(root: &Path, width: usize, height: usize) {
    let stride = (width * 3 + 3) & !3;
    let mut bytes = vec![0; 54 + stride * height];
    bytes[..2].copy_from_slice(b"BM");
    for (offset, value) in [(2, bytes.len() as u32), (10, 54), (14, 40), (18, width as u32), (22, height as u32)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height { for x in 0..width { bytes[54+y*stride+x*3..57+y*stride+x*3].copy_from_slice(&[3,2,1]); } }
    write_modvfs_fixture(root, "map/provinces.bmp", bytes);
    write_modvfs_fixture(root, "map/definition.csv", "0;0;0;0;land;false;plains;0\n1;1;2;3;land;false;plains;0\n");
    write_indexed_bmp(&root.join("map/heightmap.bmp"), width, height, 100);
}
