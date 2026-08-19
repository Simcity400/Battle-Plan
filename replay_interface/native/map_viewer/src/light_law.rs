//! LIGHT1 — the map light-rig law (files-first, capture-validated).
//!
//! This module encodes the scene lighting rig the game's map shaders consume from
//! scene constant-buffer 0 (CB0, `ConstantBuffer(0,0)` in
//! `gfx/FX/standardfuncsgfx.fxh`): the virtual sun/moon positions, the
//! day-night terminator driver, the ambient rig, and the diffuse/specular
//! intensities. Every field is either (a) a shipped define we reproduce
//! exactly, (b) a fitted law with residuals against the mined captures, or
//! (c) an engine-runtime value we read from CB0 and cannot derive from files
//! (typed unavailable when the required runtime value is absent).
//!
//! Sibling of [`crate::water_law`], which owns the water sun/moon *packing*
//! (`.xwz` swizzle) law. This module owns the *rig* (positions, day/night,
//! colors) and does not touch the water packing law.
//!
//! # Where the law comes from (shipped files, source of truth)
//!
//! CB0 is declared in `gfx/FX/standardfuncsgfx.fxh:2-33`. The HLSL cbuffer
//! packing rules pin every field to the byte offsets in [`offsets`]. The
//! lighting constants are shipped in `common/defines/00_graphics.lua`
//! (~lines 1043-1066) and `gfx/FX/constants.fxh` (ambient maps, night boost).
//!
//! # The day-night mechanism (per-pixel terminator, no texture)
//!
//! Night darkness is applied per-pixel, entirely from the single CB0 uniform
//! `DayNight_Hour_SunDir` (offset 256) — there is no shadow/terminator texture.
//! `standardfuncsgfx.fxh`:
//! * `CalcGlobeNormal(worldXZ)` (fxh:347) maps a pixel's world XZ onto a virtual
//!   globe, adding `DayNight_Hour_SunDir.x` as an east/west longitude phase:
//!   `fmod_loop( (worldXZ.x - GMT_OFFSET)/MAP_SIZE_X + DayNight_Hour_SunDir.x, 1)`.
//!   So `.x` is the hour-of-day fraction that rotates the terminator.
//! * `DayNightFactor(globeNormal)` (fxh:358) = `saturate((dot(globeNormal,
//!   DayNight_Hour_SunDir.yzw) - vMin)/(vMax-vMin)) * FoW.w`. So `.yzw` is the
//!   sun-direction unit vector; the dot with the per-pixel globe normal is the
//!   terminator, feathered by FEATHER_MIN/MAX = -/+0.01.
//! * `DayNightWithBlend`/`NightifyColor` (fxh:370-399) darken by
//!   `NIGHT_DARKNESS = 0.5` and desaturate, lerped by
//!   `DayNightFactor * NIGHT_OPACITY (0.85)`.
//!
//! The whole day-night blend is gated by `FoW.w`
//! (`vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.w`, offset 252). In EVERY
//! mined capture `FoW.w == 0`, i.e. the corpus is full-daylight only; see the
//! capture-spec gap in the doc.

/// Byte offsets of every CB0 lighting field, derived from the HLSL packing of
/// `ConstantBuffer(0,0)` in `gfx/FX/standardfuncsgfx.fxh`. Total CB0 size is
/// 416 bytes (26 float4 registers), which matches every captured scene CB0.
pub mod offsets {
    /// `vGBCamDistOverride_GBOutlineCutoff` float4 (reg 8).
    pub const GB_CAM_DIST_OVERRIDE: usize = 128;
    /// `vVirtualSunPos` float4 (reg 9).
    pub const VIRTUAL_SUN_POS: usize = 144;
    /// `vVirtualMoonPos` float4 (reg 10).
    pub const VIRTUAL_MOON_POS: usize = 160;
    /// `vSecondVirtualSunPos` float4 (reg 11).
    pub const SECOND_VIRTUAL_SUN_POS: usize = 176;
    /// `vSecondVirtualMoonPos` float4 (reg 12).
    pub const SECOND_VIRTUAL_MOON_POS: usize = 192;
    /// `vCamPos` float3 (reg 13 .xyz).
    pub const CAM_POS: usize = 208;
    /// `HdrRange` float (reg 13 .w).
    pub const HDR_RANGE: usize = 220;
    /// `vCamLookAtDir` float3 (reg 14 .xyz).
    pub const CAM_LOOK_AT_DIR: usize = 224;
    /// `vGlobalTime` float (reg 14 .w).
    pub const GLOBAL_TIME: usize = 236;
    /// `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed` float4 (reg 15).
    pub const FOW_OPACITY_TIME_SNOWMUD_MAXSPEED: usize = 240;
    /// `DayNight_Hour_SunDir` float4 (reg 16). The whole day-night driver.
    pub const DAY_NIGHT_HOUR_SUN_DIR: usize = 256;
    /// `AmbientPosX` float3 (reg 17 .xyz) — engine-composed per-frame ambient.
    pub const AMBIENT_POS_X: usize = 272;
    /// `ShadowFadeFactor` float (reg 17 .w).
    pub const SHADOW_FADE_FACTOR: usize = 284;
    /// `AmbientNegX` float3 (reg 18 .xyz).
    pub const AMBIENT_NEG_X: usize = 288;
    /// `FOWFadeFactor` float (reg 18 .w).
    pub const FOW_FADE_FACTOR: usize = 300;
    /// `AmbientPosY` float3 (reg 19 .xyz).
    pub const AMBIENT_POS_Y: usize = 304;
    /// `MinMeshAlpha` float (reg 19 .w).
    pub const MIN_MESH_ALPHA: usize = 316;
    /// `AmbientNegY` float3 (reg 20 .xyz).
    pub const AMBIENT_NEG_Y: usize = 320;
    /// `NegFogMultiplier` float (reg 20 .w).
    pub const NEG_FOG_MULTIPLIER: usize = 332;
    /// `AmbientPosZ` float3 (reg 21 .xyz).
    pub const AMBIENT_POS_Z: usize = 336;
    /// `AmbientNegZ` float3 (reg 22 .xyz — a fresh register: a float3 cannot
    /// share the .w left by the previous float3 under HLSL packing).
    pub const AMBIENT_NEG_Z: usize = 352;
    /// `CubemapIntensity` float (reg 22 .w).
    pub const CUBEMAP_INTENSITY: usize = 364;
    /// `SunDiffuseIntensity` float4 (reg 23) — rgb color, a = intensity.
    pub const SUN_DIFFUSE_INTENSITY: usize = 368;
    /// `MoonDiffuseIntensity` float4 (reg 24) — rgb color, a = intensity.
    pub const MOON_DIFFUSE_INTENSITY: usize = 384;
    /// `GB_TextureHeight` float (reg 25 .x).
    pub const GB_TEXTURE_HEIGHT: usize = 400;
    /// `SunSpecularIntensity` float (reg 25 .y).
    pub const SUN_SPECULAR_INTENSITY: usize = 404;
    /// Total CB0 byte length (416 = 26 * 16).
    pub const CB0_LEN: usize = 416;
}

/// Shipped lighting defines, byte-for-byte from the vanilla installed content. Kept as
/// constants so a validation test fails loudly if a mod or patch retunes them;
/// the resolver-based readers elsewhere parse the live files, these mirror the
/// vanilla values the laws were fitted against.
pub mod defines {
    // --- common/defines/00_graphics.lua ---
    /// `SUN_HEIGHT` (line 1044), packed in `vVirtualSunPos.y`.
    pub const SUN_HEIGHT: f32 = 600.0;
    /// `MOON_HEIGHT` (line 1043), packed in `vVirtualMoonPos.y`.
    pub const MOON_HEIGHT: f32 = 600.0;
    /// `SUN_LATITUDE` (line 1048), packed in `vVirtualSunPos.z`.
    pub const SUN_LATITUDE: f32 = 848.0;
    /// `MOON_LATITUDE` (line 1047), commented "NOT USED"; the captured
    /// `vVirtualMoonPos.z` is NOT this value (see [`MOON_Z_HYPOTHESIS`]).
    pub const MOON_LATITUDE_NOT_USED: f32 = 0.0;
    /// `SECOND_SUN_LATITUDE` (line 1050), packed in `vSecondVirtualSunPos.z`.
    pub const SECOND_SUN_LATITUDE: f32 = 100.0;
    /// `SECOND_MOON_LATITUDE` (line 1049), packed in `vSecondVirtualMoonPos.z`.
    pub const SECOND_MOON_LATITUDE: f32 = 100.0;
    /// `GMT_OFFSET` (line 934) — map X of the Greenwich/GMT+0 meridian, the
    /// longitude reference used by `CalcGlobeNormal`. NOTE: this, not the
    /// camera, is the sun's longitude anchor (camera-follow is refuted).
    pub const GMT_OFFSET: f32 = 2793.0;
    /// `SUN_DIFFUSE_COLOR` HSV (line 1060). H is on the 0..1 dial the engine
    /// scales by 6 before `HSVtoRGBPost`.
    pub const SUN_DIFFUSE_COLOR_HSV: [f32; 3] = [0.14, 0.0, 1.0];
    /// `SUN_INTENSITY` (line 1061) — `SunDiffuseIntensity.a`.
    pub const SUN_INTENSITY: f32 = 1.0;
    /// `SUN_SPECULAR_INTENSITY` (line 1062) — CB0 `SunSpecularIntensity`.
    pub const SUN_SPECULAR_INTENSITY: f32 = 1.0;
    /// `MOON_DIFFUSE_COLOR` HSV (line 1063).
    pub const MOON_DIFFUSE_COLOR_HSV: [f32; 3] = [0.58, 0.5, 1.0];
    /// `MOON_INTENSITY` (line 1064) — `MoonDiffuseIntensity.a`.
    pub const MOON_INTENSITY: f32 = 2.5;
    /// `CUBEMAP_INTENSITY` (line 1066) — CB0 `CubemapIntensity`.
    pub const CUBEMAP_INTENSITY: f32 = 1.0;
    /// Map width in world units, `MAP_SIZE_X`. Shipped as the `MAP_SIZE_X`
    /// shader define (parsed from the scache by
    /// `shader_meta::find_shader_cache_map_constants`) and equal to the
    /// vanilla `map/heightmap.bmp` width (5632; the shipped heightmap is
    /// 5632x2048). This is the wrap period of the virtual sun/moon longitude
    /// law (`virtual_sun_longitude`): the rig completes one full east->west
    /// pass across `MAP_SIZE_X` per game day. Mirrors `PRODUCT_MAP_WIDTH` in
    /// the replay product surface; a validation test fails loudly if the two
    /// disagree.
    pub const MAP_SIZE_X: f32 = 5632.0;

    // --- gfx/FX/standardfuncsgfx.fxh ---
    /// `NIGHT_OPACITY` (fxh:328) — max night blend applied to day color.
    pub const NIGHT_OPACITY: f32 = 0.85;
    /// `NIGHT_DARKNESS` (fxh:329) — the night color multiplier.
    pub const NIGHT_DARKNESS: f32 = 0.5;
    /// `FEATHER_MIN` (fxh:324) — terminator lower feather.
    pub const FEATHER_MIN: f32 = -0.01;
    /// `FEATHER_MAX` (fxh:325) — terminator upper feather.
    pub const FEATHER_MAX: f32 = 0.01;

    // --- gfx/FX/constants.fxh ---
    /// `NIGHT_AMBIENT_BOOST` (constants.fxh:14) — night ambient multiplier
    /// before saturate.
    pub const NIGHT_AMBIENT_BOOST: f32 = 3.0;

    // --- common/defines/00_graphics.lua:1052-1058 ("hsv color ambient light") ---
    // The CB0 `Ambient{Pos,Neg}{X,Y,Z}` compose inputs (AMBIENT1, 2026-07-09).
    // These HSV triples, pushed through the shader's own manual-input HSV->RGB
    // (`HSVtoRGB`, standardfuncsgfx.fxh:145-152, which ends in `ToLinear` =
    // pow 2.2), reproduce the captured CB0 ambient rig BIT-EXACTLY (residual
    // 0.0 on all 18 components across the hour-swept corpus samples). See
    // `cb0_ambient_rig_from_hsv_defines`. NOTE: these are NOT the shader-side
    // `DayAmbientMap*` constants (constants.fxh:17-22) — those are a separate
    // per-pixel map-shader rig; constants.fxh:31 says it directly: "regular
    // unit ambient colors set from defines. this is night".
    /// `AMBIENT_LIGHT_POS_X` (00_graphics.lua:1053) HSV — right face.
    pub const AMBIENT_LIGHT_POS_X_HSV: [f32; 3] = [0.2, 0.2, 0.2];
    /// `AMBIENT_LIGHT_NEG_X` (00_graphics.lua:1054) HSV — left face.
    pub const AMBIENT_LIGHT_NEG_X_HSV: [f32; 3] = [0.4, 0.1, 0.6];
    /// `AMBIENT_LIGHT_POS_Y` (00_graphics.lua:1055) HSV — "kills everything".
    pub const AMBIENT_LIGHT_POS_Y_HSV: [f32; 3] = [0.0, 0.0, 0.0];
    /// `AMBIENT_LIGHT_NEG_Y` (00_graphics.lua:1056) HSV — from under (V=0, so
    /// it composes to black exactly like the captured field).
    pub const AMBIENT_LIGHT_NEG_Y_HSV: [f32; 3] = [0.35, 0.2, 0.0];
    /// `AMBIENT_LIGHT_POS_Z` (00_graphics.lua:1057) HSV — top face.
    pub const AMBIENT_LIGHT_POS_Z_HSV: [f32; 3] = [0.6, 0.2, 0.924];
    /// `AMBIENT_LIGHT_NEG_Z` (00_graphics.lua:1058) HSV — bottom face.
    pub const AMBIENT_LIGHT_NEG_Z_HSV: [f32; 3] = [0.55, 0.1, 0.9];

    /// `NightAmbient*` (gfx/FX/constants.fxh:32-37) — the NIGHT counterpart of
    /// the CB0 day ambient rig for the unit/mesh path: `standardfuncsgfx.fxh`
    /// pairs the CB0 `Ambient*` fields (day) with these constants (night) in
    /// `AmbientLight(..., DayAmbientColors_, NightAmbientColors_)` (fxh:712-728),
    /// blending per pixel by the day/night factor with `NIGHT_AMBIENT_BOOST`
    /// (fxh:468-470). Order matches the CB0 field order
    /// [PosX, NegX, PosY, NegY, PosZ, NegZ].
    pub const NIGHT_AMBIENT: [[f32; 3]; 6] = [
        [2.0, 2.0, 2.0], // NightAmbientPosX (constants.fxh:32)
        [0.2, 0.2, 0.2], // NightAmbientNegX (constants.fxh:33)
        [0.1, 0.1, 0.1], // NightAmbientPosY (constants.fxh:34)
        [0.0, 0.0, 0.0], // NightAmbientNegY (constants.fxh:35)
        [3.0, 3.0, 3.0], // NightAmbientPosZ (constants.fxh:36)
        [0.8, 0.8, 0.8], // NightAmbientNegZ (constants.fxh:37)
    ];

    /// The captured `vVirtualMoonPos.z` is a constant 1448 in every mined
    /// capture. It is UNEXPLAINED by any single shipped define (`MOON_LATITUDE`
    /// is 0/"NOT USED"). Two file-grounded arithmetic identities both hit 1448
    /// exactly, and the corpus (one unvarying value) cannot choose between them:
    ///   * `SUN_LATITUDE + MOON_HEIGHT = 848 + 600 = 1448`
    ///   * `MAP_SIZE_Y - MOON_HEIGHT   = 2048 - 600 = 1448` (map height 2048,
    ///     from the shipped 5632x2048 heightmap.bmp)
    /// Both are HYPOTHESES, not proven source assignments — see the doc's
    /// capture spec for what would disambiguate them. This constant carries the
    /// first identity purely so a validation residual can be reported; it is
    /// numerically equal to the second.
    pub const MOON_Z_HYPOTHESIS: f32 = SUN_LATITUDE + MOON_HEIGHT;
}

/// A packed CB0 float4 read little-endian from raw bytes.
fn read_f4(cb0: &[u8], off: usize) -> Option<[f32; 4]> {
    let s = cb0.get(off..off.checked_add(16)?)?;
    let f = |i: usize| f32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    Some([f(0), f(4), f(8), f(12)])
}

/// A packed CB0 float3 read little-endian from raw bytes.
fn read_f3(cb0: &[u8], off: usize) -> Option<[f32; 3]> {
    let s = cb0.get(off..off.checked_add(12)?)?;
    let f = |i: usize| f32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
    Some([f(0), f(4), f(8)])
}

/// A single float read little-endian from raw bytes.
fn read_f1(cb0: &[u8], off: usize) -> Option<f32> {
    let s = cb0.get(off..off.checked_add(4)?)?;
    Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The lighting-relevant fields of a captured scene CB0, decoded from the
/// `standardfuncsgfx.fxh` layout. Every field maps to a named cbuffer member.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLighting {
    /// `vVirtualSunPos` (x, terrain-height y, latitude z, water-height w).
    pub virtual_sun_pos: [f32; 4],
    /// `vVirtualMoonPos`.
    pub virtual_moon_pos: [f32; 4],
    /// `vSecondVirtualSunPos` (seam-hiding fake).
    pub second_virtual_sun_pos: [f32; 4],
    /// `vSecondVirtualMoonPos` (seam-hiding fake).
    pub second_virtual_moon_pos: [f32; 4],
    /// `vCamPos`.
    pub cam_pos: [f32; 3],
    /// `vCamLookAtDir`.
    pub cam_look_at_dir: [f32; 3],
    /// `vGlobalTime` — continuous animation clock (NOT the day-night hour).
    pub global_time: f32,
    /// `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed`. `.w` gates the whole
    /// day-night blend; observed 0 (full daylight) in the entire corpus.
    pub fow_opacity_time_snowmud_maxspeed: [f32; 4],
    /// `DayNight_Hour_SunDir`: `.x` hour-of-day fraction (longitude phase),
    /// `.yzw` sun-direction unit vector. The day-night driver.
    pub day_night_hour_sun_dir: [f32; 4],
    /// `SunDiffuseIntensity`: rgb color, a = intensity.
    pub sun_diffuse_intensity: [f32; 4],
    /// `MoonDiffuseIntensity`: rgb color, a = intensity.
    pub moon_diffuse_intensity: [f32; 4],
    /// `SunSpecularIntensity`.
    pub sun_specular_intensity: f32,
    /// `CubemapIntensity`.
    pub cubemap_intensity: f32,
    /// The engine-composed CB0 ambient rig, in CB0 field order
    /// `[AmbientPosX, AmbientNegX, AmbientPosY, AmbientNegY, AmbientPosZ,
    /// AmbientNegZ]`. Composed from the `AMBIENT_LIGHT_*` HSV defines — see
    /// [`cb0_ambient_rig_from_hsv_defines`].
    pub ambient_rig: [[f32; 3]; 6],
}

impl SceneLighting {
    /// Decode the lighting rig from a raw scene CB0 byte buffer. Returns `None`
    /// (fails closed) if the buffer is shorter than the fields it must read.
    pub fn from_cb0_bytes(cb0: &[u8]) -> Option<Self> {
        use offsets::*;
        Some(Self {
            virtual_sun_pos: read_f4(cb0, VIRTUAL_SUN_POS)?,
            virtual_moon_pos: read_f4(cb0, VIRTUAL_MOON_POS)?,
            second_virtual_sun_pos: read_f4(cb0, SECOND_VIRTUAL_SUN_POS)?,
            second_virtual_moon_pos: read_f4(cb0, SECOND_VIRTUAL_MOON_POS)?,
            cam_pos: read_f3(cb0, CAM_POS)?,
            cam_look_at_dir: read_f3(cb0, CAM_LOOK_AT_DIR)?,
            global_time: read_f1(cb0, GLOBAL_TIME)?,
            fow_opacity_time_snowmud_maxspeed: read_f4(cb0, FOW_OPACITY_TIME_SNOWMUD_MAXSPEED)?,
            day_night_hour_sun_dir: read_f4(cb0, DAY_NIGHT_HOUR_SUN_DIR)?,
            sun_diffuse_intensity: read_f4(cb0, SUN_DIFFUSE_INTENSITY)?,
            moon_diffuse_intensity: read_f4(cb0, MOON_DIFFUSE_INTENSITY)?,
            sun_specular_intensity: read_f1(cb0, SUN_SPECULAR_INTENSITY)?,
            cubemap_intensity: read_f1(cb0, CUBEMAP_INTENSITY)?,
            ambient_rig: [
                read_f3(cb0, AMBIENT_POS_X)?,
                read_f3(cb0, AMBIENT_NEG_X)?,
                read_f3(cb0, AMBIENT_POS_Y)?,
                read_f3(cb0, AMBIENT_NEG_Y)?,
                read_f3(cb0, AMBIENT_POS_Z)?,
                read_f3(cb0, AMBIENT_NEG_Z)?,
            ],
        })
    }

    /// The in-game hour-of-day this frame's terminator is set for, derived from
    /// `DayNight_Hour_SunDir.x` (which is `hour / 24`). Files-first: this is the
    /// engine's own `+ DayNight_Hour_SunDir.x` longitude phase in
    /// `CalcGlobeNormal` (fxh:349), read back as `x * 24`.
    pub fn day_night_hour(self) -> f32 {
        self.day_night_hour_sun_dir[0] * 24.0
    }

    /// The sun-direction unit vector `DayNight_Hour_SunDir.yzw` used by
    /// `DayNightFactor`'s dot product (fxh:360).
    pub fn sun_dir_yzw(self) -> [f32; 3] {
        [
            self.day_night_hour_sun_dir[1],
            self.day_night_hour_sun_dir[2],
            self.day_night_hour_sun_dir[3],
        ]
    }

    /// Whether this frame's day-night blend is fully disabled, i.e. the capture
    /// is unconditional full daylight because `FoW.w (MaxGameSpeed) == 0` gates
    /// `DayNightFactor` to 0 (fxh:361). True for the entire mined corpus.
    pub fn is_full_daylight_capture(self) -> bool {
        self.fow_opacity_time_snowmud_maxspeed[3] == 0.0
    }
}

/// the game's `HuePost` (standardfuncsgfx.fxh:91) — H on the 0..6 sextant dial.
fn hue_post(h: f32) -> [f32; 3] {
    let x = 1.0 - ((h % 2.0) - 1.0).abs();
    if h < 1.0 {
        [1.0, x, 0.0]
    } else if h < 2.0 {
        [x, 1.0, 0.0]
    } else if h < 3.0 {
        [0.0, 1.0, x]
    } else if h < 4.0 {
        [0.0, x, 1.0]
    } else if h < 5.0 {
        [x, 0.0, 1.0]
    } else {
        [1.0, 0.0, x]
    }
}

/// the game's `HSVtoRGBPost` (standardfuncsgfx.fxh:102) with H already scaled to
/// the 0..6 dial. Matches the shader's clamp semantics.
fn hsv_to_rgb_post(hsv6: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv6;
    if s != 0.0 {
        let c = s * v;
        let hue = hue_post(h);
        [
            (hue[0] * c + (v - c)).clamp(0.0, 1.0),
            (hue[1] * c + (v - c)).clamp(0.0, 1.0),
            (hue[2] * c + (v - c)).clamp(0.0, 1.0),
        ]
    } else {
        [v.clamp(0.0, 1.0); 3]
    }
}

/// The diffuse-color law: a shipped `*_DIFFUSE_COLOR` HSV define (H on the 0..1
/// dial) becomes the CB0 `*DiffuseIntensity.rgb` via
/// `ToLinear(HSVtoRGBPost(H*6, S, V))` — i.e. HSV->RGB then a gamma-2.2 lift to
/// linear (`ToLinear`, fxh:48). Reproduces the captured moon color to <1e-4 and
/// the sun color exactly. `.a` of the CB0 field is the matching `*_INTENSITY`.
pub fn diffuse_color_rgb_from_hsv_define(hsv_dial01: [f32; 3]) -> [f32; 3] {
    let rgb = hsv_to_rgb_post([hsv_dial01[0] * 6.0, hsv_dial01[1], hsv_dial01[2]]);
    [rgb[0].powf(2.2), rgb[1].powf(2.2), rgb[2].powf(2.2)]
}

/// the game's `Hue` (standardfuncsgfx.fxh:136-142) — the *manual-input* hue ramp
/// (H on the 0..1 dial, scaled by 6 inside), distinct from `HuePost`
/// (fxh:91) used by the diffuse-color path. Saturated per the shader.
fn hue_manual(h01: f32) -> [f32; 3] {
    let h6 = h01 * 6.0;
    let r = (h6 - 3.0).abs() - 1.0;
    let g = 2.0 - (h6 - 2.0).abs();
    let b = 2.0 - (h6 - 4.0).abs();
    [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
}

/// the game's manual-input `HSVtoRGB` (standardfuncsgfx.fxh:145-152):
/// `ToLinear(((Hue(H) - 1) * S + 1) * V)` with `ToLinear = pow(rgb, 2.2)`
/// (fxh:48). The shader's own comment: "used for manual input, converts to
/// linear". This is the transform the engine applies to the `AMBIENT_LIGHT_*`
/// HSV defines when it writes the CB0 `Ambient{Pos,Neg}{X,Y,Z}` fields.
pub fn hsv_to_rgb_manual_linear(hsv_dial01: [f32; 3]) -> [f32; 3] {
    let [h, s, v] = hsv_dial01;
    let hue = hue_manual(h);
    let val = |c: f32| (((c - 1.0) * s + 1.0) * v).powf(2.2);
    [val(hue[0]), val(hue[1]), val(hue[2])]
}

/// The vanilla `AMBIENT_LIGHT_*` HSV defines (00_graphics.lua:1053-1058) in
/// CB0 ambient field order `[PosX, NegX, PosY, NegY, PosZ, NegZ]`.
pub fn vanilla_ambient_light_hsv_defines() -> [[f32; 3]; 6] {
    [
        defines::AMBIENT_LIGHT_POS_X_HSV,
        defines::AMBIENT_LIGHT_NEG_X_HSV,
        defines::AMBIENT_LIGHT_POS_Y_HSV,
        defines::AMBIENT_LIGHT_NEG_Y_HSV,
        defines::AMBIENT_LIGHT_POS_Z_HSV,
        defines::AMBIENT_LIGHT_NEG_Z_HSV,
    ]
}

/// AMBIENT1 — the CB0 ambient-rig COMPOSE law (files-first, capture-validated).
///
/// The engine writes the scene CB0 `Ambient{Pos,Neg}{X,Y,Z}` fields (offsets
/// 272..364) as `HSVtoRGB(AMBIENT_LIGHT_*)` — the manual-input HSV→linear
/// transform ([`hsv_to_rgb_manual_linear`]) applied to the six
/// `AMBIENT_LIGHT_*` HSV defines (00_graphics.lua:1053-1058). Validated
/// BIT-EXACT (residual 0.0 on all 18 components) against the real hour-swept
/// scene CB0 corpus (session_CAPFIX2_FULL_2 samples embedded in the tests);
/// the captured rig is also HOUR-INVARIANT (hour 1 and hour 6 samples carry
/// byte-identical ambient), consistent with the compose reading no day/night
/// state.
///
/// IMPORTANT provenance correction (vs the earlier ledger reading): the CB0
/// field is NOT an engine-side day/night-blended ambient. The day/night
/// ambient blend + `NIGHT_AMBIENT_BOOST` happen PER PIXEL inside the shaders
/// (`AmbientLight`, standardfuncsgfx.fxh:456-470), which pair the CB0 fields
/// (day) with the `NightAmbient*` constants (night, constants.fxh:32-37,
/// [`defines::NIGHT_AMBIENT`]) on the unit/mesh path — or pair the
/// `DayAmbientMap*`/`NightAmbientMap*` constants.fxh rigs on the map path,
/// with no CB0 involvement at all. At the captured day state
/// (`DayNightFactor == 0` because `FoW.w == 0`) the in-shader blend returns
/// the day array unchanged, so this compose IS the whole captured value.
pub fn cb0_ambient_rig_from_hsv_defines(hsv6: [[f32; 3]; 6]) -> [[f32; 3]; 6] {
    [
        hsv_to_rgb_manual_linear(hsv6[0]),
        hsv_to_rgb_manual_linear(hsv6[1]),
        hsv_to_rgb_manual_linear(hsv6[2]),
        hsv_to_rgb_manual_linear(hsv6[3]),
        hsv_to_rgb_manual_linear(hsv6[4]),
        hsv_to_rgb_manual_linear(hsv6[5]),
    ]
}

/// Max absolute per-component residual between a captured CB0 ambient rig and
/// a composed one.
pub fn ambient_rig_residual(captured: &[[f32; 3]; 6], composed: &[[f32; 3]; 6]) -> f32 {
    let mut worst = 0.0f32;
    for (cap, law) in captured.iter().zip(composed.iter()) {
        for (c, l) in cap.iter().zip(law.iter()) {
            worst = worst.max((c - l).abs());
        }
    }
    worst
}

/// Parse the six `AMBIENT_LIGHT_*` HSV defines from `00_graphics.lua` source
/// text (mod-aware callers resolve the file through `GameFileResolver` and pass
/// the text here; this module stays dependency-free). Returns them in CB0
/// ambient field order. Fails closed if any key is missing or malformed.
pub fn ambient_light_hsv_defines_from_graphics_source(
    source: &str,
) -> Result<[[f32; 3]; 6], String> {
    const KEYS: [&str; 6] = [
        "AMBIENT_LIGHT_POS_X",
        "AMBIENT_LIGHT_NEG_X",
        "AMBIENT_LIGHT_POS_Y",
        "AMBIENT_LIGHT_NEG_Y",
        "AMBIENT_LIGHT_POS_Z",
        "AMBIENT_LIGHT_NEG_Z",
    ];
    let mut out = [[0.0f32; 3]; 6];
    let mut found = [false; 6];
    for raw in source.lines() {
        // Strip lua comments; keys sit at top level as `KEY = { h, s, v },`.
        let code = raw.split("--").next().unwrap_or(raw).trim();
        let Some((lhs, rhs)) = code.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let Some(slot) = KEYS.iter().position(|k| *k == key) else {
            continue;
        };
        let open = rhs.find('{').ok_or_else(|| {
            format!("00_graphics.lua {key} has no {{ h, s, v }} table on its line")
        })?;
        let close = rhs[open + 1..]
            .find('}')
            .ok_or_else(|| format!("00_graphics.lua {key} table is unterminated on its line"))?;
        let inner = &rhs[open + 1..open + 1 + close];
        let mut triple = [0.0f32; 3];
        let mut count = 0usize;
        for token in inner.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if count >= 3 {
                count += 1;
                break;
            }
            triple[count] = token
                .parse::<f32>()
                .map_err(|err| format!("00_graphics.lua {key} component {count}: {err}"))?;
            count += 1;
        }
        if count != 3 {
            return Err(format!(
                "00_graphics.lua {key} must have exactly 3 HSV components, found {count}"
            ));
        }
        if found[slot] {
            return Err(format!("00_graphics.lua defines {key} more than once"));
        }
        out[slot] = triple;
        found[slot] = true;
    }
    if let Some(missing) = KEYS.iter().zip(found.iter()).find(|(_, f)| !**f) {
        return Err(format!("00_graphics.lua is missing {}", missing.0));
    }
    Ok(out)
}

/// Residuals of the files-derived laws against one captured [`SceneLighting`].
/// Each `*_residual` is `captured - law`; a caller asserts `.abs()` within
/// tolerance. `None` means the field is engine-runtime (not files-derivable).
#[derive(Clone, Debug, PartialEq)]
pub struct LightingLawResidual {
    /// `vVirtualSunPos.y` vs `SUN_HEIGHT`.
    pub sun_height_residual: f32,
    /// `vVirtualSunPos.z` vs `SUN_LATITUDE`.
    pub sun_latitude_residual: f32,
    /// `vVirtualMoonPos.y` vs `MOON_HEIGHT`.
    pub moon_height_residual: f32,
    /// `vVirtualMoonPos.z` vs the `SUN_LATITUDE + MOON_HEIGHT` HYPOTHESIS.
    pub moon_z_hypothesis_residual: f32,
    /// `SunDiffuseIntensity.rgb` vs the HSV->linear law (max component).
    pub sun_diffuse_rgb_residual: f32,
    /// `SunDiffuseIntensity.a` vs `SUN_INTENSITY`.
    pub sun_intensity_residual: f32,
    /// `MoonDiffuseIntensity.rgb` vs the HSV->linear law (max component).
    pub moon_diffuse_rgb_residual: f32,
    /// `MoonDiffuseIntensity.a` vs `MOON_INTENSITY`.
    pub moon_intensity_residual: f32,
    /// `SunSpecularIntensity` vs `SUN_SPECULAR_INTENSITY`.
    pub sun_specular_residual: f32,
    /// `CubemapIntensity` vs `CUBEMAP_INTENSITY`.
    pub cubemap_intensity_residual: f32,
    /// `|DayNight_Hour_SunDir.yzw| - 1` — the sun direction is a unit vector.
    pub sun_dir_unit_residual: f32,
    /// `vVirtualSunPos.x - vCamPos.x`. The CAMERA-FOLLOW test: if the sun
    /// followed the camera this would be ~0. It is NOT (2816 vs a varying
    /// camera x), so this residual is deliberately reported, not asserted small.
    pub sun_x_minus_cam_x: f32,
}

/// Validate the files-derived lighting laws against a captured scene CB0.
pub fn validate_lighting_law_against_cb0(cb0: &[u8]) -> Option<LightingLawResidual> {
    let s = SceneLighting::from_cb0_bytes(cb0)?;
    let max3 = |a: [f32; 3], b: [f32; 3]| {
        (a[0] - b[0])
            .abs()
            .max((a[1] - b[1]).abs())
            .max((a[2] - b[2]).abs())
    };
    let sun_rgb_law = diffuse_color_rgb_from_hsv_define(defines::SUN_DIFFUSE_COLOR_HSV);
    let moon_rgb_law = diffuse_color_rgb_from_hsv_define(defines::MOON_DIFFUSE_COLOR_HSV);
    let sun_dir = s.sun_dir_yzw();
    let sun_dir_len =
        (sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2]).sqrt();
    Some(LightingLawResidual {
        sun_height_residual: s.virtual_sun_pos[1] - defines::SUN_HEIGHT,
        sun_latitude_residual: s.virtual_sun_pos[2] - defines::SUN_LATITUDE,
        moon_height_residual: s.virtual_moon_pos[1] - defines::MOON_HEIGHT,
        moon_z_hypothesis_residual: s.virtual_moon_pos[2] - defines::MOON_Z_HYPOTHESIS,
        sun_diffuse_rgb_residual: max3(
            [
                s.sun_diffuse_intensity[0],
                s.sun_diffuse_intensity[1],
                s.sun_diffuse_intensity[2],
            ],
            sun_rgb_law,
        ),
        sun_intensity_residual: s.sun_diffuse_intensity[3] - defines::SUN_INTENSITY,
        moon_diffuse_rgb_residual: max3(
            [
                s.moon_diffuse_intensity[0],
                s.moon_diffuse_intensity[1],
                s.moon_diffuse_intensity[2],
            ],
            moon_rgb_law,
        ),
        moon_intensity_residual: s.moon_diffuse_intensity[3] - defines::MOON_INTENSITY,
        sun_specular_residual: s.sun_specular_intensity - defines::SUN_SPECULAR_INTENSITY,
        cubemap_intensity_residual: s.cubemap_intensity - defines::CUBEMAP_INTENSITY,
        sun_dir_unit_residual: sun_dir_len - 1.0,
        sun_x_minus_cam_x: s.virtual_sun_pos[0] - s.cam_pos[0],
    })
}

/// Horizontal day-cycle rig law, CLOSED 2026-07-07 from session_FINAL_1's
/// unpaused CB0 samples (5 samples across game hours 0.00..15.38, residual
/// < 0.01 map units on every moving sample; the paused corpus's fixed
/// sun.x=2816 is exactly this law at the noon hour those saves sat on):
///
///   sun.x        = MAP_SIZE_X * (1 - hour24/24)   (mod MAP_SIZE_X)
///   moon.x       = sun.x + MAP_SIZE_X/2           (mod MAP_SIZE_X)  [antipodal]
///   second_sun.x = moon.x   [seam-hiding fake at the opposite body]
///   second_moon.x = sun.x
///
/// hour24 is DayNight_Hour_SunDir.x * 24. The .y/.z/.w height/latitude
/// components stay at their file-derived constants (600/848/5000 etc.);
/// the sun-direction .yzw seasonal drift is date-driven and handled separately.
pub fn virtual_sun_longitude(hour24: f32, map_size_x: f32) -> f32 {
    (map_size_x * (1.0 - hour24 / 24.0)).rem_euclid(map_size_x)
}

pub fn virtual_moon_longitude(hour24: f32, map_size_x: f32) -> f32 {
    (virtual_sun_longitude(hour24, map_size_x) + map_size_x * 0.5).rem_euclid(map_size_x)
}

/// The four virtual-rig `.x` longitudes the render must write into scene CB0,
/// evaluated at the render hour, in CB0 field order:
///   `[vVirtualSunPos.x, vVirtualMoonPos.x,
///     vSecondVirtualSunPos.x, vSecondVirtualMoonPos.x]`.
///
/// From the day-cycle law (see [`virtual_sun_longitude`]): the sun and moon
/// ride their longitudes; the two seam-hiding "second" bodies are the SWAPPED
/// pair — `vSecondVirtualSunPos` rides the *moon* longitude and
/// `vSecondVirtualMoonPos` rides the *sun* longitude (the fake sun/moon sit on
/// the opposite side of the globe to hide the wrap seam; measured in
/// session_FINAL_1).
///
/// Only the `.x` lane of each float4 is produced here; the heights/latitudes
/// (`.y/.z/.w`) are file-exact constants the render must leave untouched.
///
/// At `hour24 == 12.0` (noon) with `map_size_x == 5632.0` this returns
/// `[2816.0, 0.0, 0.0, 2816.0]` — exactly the values every paused/noon capture
/// carries — so writing them is byte-identical to the captured rig at noon.
pub fn virtual_rig_longitudes_x(hour24: f32, map_size_x: f32) -> [f32; 4] {
    let sun = virtual_sun_longitude(hour24, map_size_x);
    let moon = virtual_moon_longitude(hour24, map_size_x);
    [sun, moon, moon, sun]
}

/// NIGHT1 — the night-side sun-direction (`DayNight_Hour_SunDir.yzw`) law,
/// mined from tonight's UNPAUSED hour-swept CB0 corpora
/// (session_NIGHT_SWEEP_6 + session_CAPFIX2_FULL_2 + session_CAPFIX2_GAPS_4,
/// 468 decoded samples spanning in-game hours 0..23).
///
/// # The finding: `.yzw` is HOUR-INVARIANT (the hour lives entirely in `.x`)
///
/// The hour-swept sweep establishes that **the
/// sun-direction vector does NOT rotate with the game hour.** Measured across
/// all 468 samples:
/// * `.y` (dir.x) = `0.000000` **exactly**, at every hour.
/// * `.w` (dir.z) ∈ `[0.99700, 0.99996]` (≈ 1.0), at every hour.
/// * `.z` (dir.y) is the only mover, range `−0.0345..+0.0774` — and it tracks
///   advancing **simulation time** (the in-game date), NOT the hour. (That the
///   drift is specifically a *seasonal/declination* term is the natural reading
///   but is not fully pinned; what IS proven is that it moves with elapsed
///   sim-days and is hour-independent — a controlled same-date/different-date
///   capture would pin the exact driver.)
///
/// The decisive control is session_CAPFIX2_FULL_2: the game hour advances
/// MONOTONICALLY 1→2→3→4→5→6 (unpaused, 25 samples) while `.z` stays pinned to
/// within `1e-6` (0.077357..0.077358). A rotation model (`.y=r·cosθ`,
/// `.w=r·sinθ`) is therefore inadmissible — `.y` never leaves 0 and `.w` never
/// leaves ≈1 as the hour sweeps a full turn. In session_NIGHT_SWEEP_6 the hour
/// bounces (1,2,4,9,10,12,16,18,20,21,…) while `.z` marches with **wall-clock**
/// (−0.0086→−0.0316) then FREEZES the instant the game pauses — i.e. `.z` is a
/// season/date term, decoupled from the hour.
///
/// # Physical reading (consistent with the shipped shader)
///
/// This is exactly what `standardfuncsgfx.fxh` implies: `CalcGlobeNormal`
/// (fxh:347) folds the hour into the terminator via `+ DayNight_Hour_SunDir.x`
/// (the east/west longitude phase), and `DayNightFactor` (fxh:358) dots the
/// per-pixel globe normal against `.yzw`. The sun's *elevation* direction for a
/// given date is fixed; the day rotates the *terminator* (through `.x`), not the
/// sun vector. So the whole hour dependence is already carried by `.x` — the
/// same `.x` the rig-longitude law (`virtual_rig_longitudes_x`) and
/// `CalcGlobeNormal` consume.
///
/// # Consequence for the replay `--hour` path (files-first, byte-safe)
///
/// The correct render behavior is: write `DayNight_Hour_SunDir.x = hour/24` and
/// **leave `.yzw` at the captured value** — because the captured `.yzw` is the
/// (hour-invariant) sun direction for the recording's own date, and no hour
/// synthesis is warranted. This function is the identity/passthrough that makes
/// that law explicit and unit-testable: given a render `hour24` and the
/// captured `.yzw`, it returns the `.yzw` the render must write — the captured
/// vector, unchanged. It exists so the "do not rotate `.yzw` with the hour"
/// decision is a named, tested law rather than an implicit omission, and so the
/// `--hour 0.5` byte-identity gate is provably preserved (the captured `.yzw`
/// is echoed bit-for-bit, independent of the hour argument).
///
/// `hour24` is accepted only to document that the result is independent of it;
/// it is asserted finite so a non-finite render clock is caught here too.
pub fn render_sun_dir_yzw(hour24: f32, captured_yzw: [f32; 3]) -> [f32; 3] {
    debug_assert!(
        hour24.is_finite(),
        "render_sun_dir_yzw: hour24 must be finite, got {hour24}"
    );
    // Hour-invariant: the sun direction for the recording's date is echoed
    // verbatim. The `_ = hour24` makes the independence explicit to readers.
    let _ = hour24;
    captured_yzw
}

/// Residual of the NIGHT1 hour-invariance law against one captured sample:
/// how far the captured `.yzw` is from the law-predicted `.yzw` at that hour.
/// By construction the law echoes the captured vector, so within a *single
/// date* this is identically 0; the useful signal is the SPREAD of `.yzw`
/// across many hours at a fixed date, which the sweep shows is ≤ the tolerances
/// asserted in the tests (`.y` bit-exact 0, `.w` within 3e-3 of 1, `.z` within
/// 1e-6 across a monotonic intra-date hour run). Returns
/// `(unit_residual, y_abs, one_minus_w_abs)` for a decoded sample's `.yzw`.
pub fn night_yzw_residual(yzw: [f32; 3]) -> (f32, f32, f32) {
    let len = (yzw[0] * yzw[0] + yzw[1] * yzw[1] + yzw[2] * yzw[2]).sqrt();
    (
        (len - 1.0).abs(), // unit-vector residual
        yzw[0].abs(),      // .y (dir.x) — measured exactly 0
        (yzw[2] - 1.0).abs(),
    ) // .w (dir.z) — measured ≈ 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a scene CB0 byte buffer carrying the lighting fields exactly as
    /// measured in the mined captures (this specific tuple is
    /// live_PAN2/map_frame_draw000; the sun/moon/color fields are identical
    /// across all 1212 mined scene CB0s from 59 sessions with map bundles).
    /// Measured directly
    /// from
    ///   C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2\
    ///     map_frame_draw000_ps_constant_buffer_slot0.bin
    fn mined_scene_cb0_live_pan2() -> Vec<u8> {
        let mut cb0 = vec![0u8; offsets::CB0_LEN];
        let mut put4 = |off: usize, v: [f32; 4]| {
            for (i, c) in v.iter().enumerate() {
                cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&c.to_le_bytes());
            }
        };
        put4(offsets::VIRTUAL_SUN_POS, [2816.0, 600.0, 848.0, 5000.0]);
        put4(offsets::VIRTUAL_MOON_POS, [0.0, 600.0, 1448.0, 550.0]);
        put4(offsets::SECOND_VIRTUAL_SUN_POS, [0.0, 600.0, 100.0, 5000.0]);
        put4(
            offsets::SECOND_VIRTUAL_MOON_POS,
            [2816.0, 600.0, 100.0, 550.0],
        );
        // vCamPos.xyz (208..220) + HdrRange (220) share reg 13.
        put4(offsets::CAM_POS, [2958.0, 800.0, 1400.0, 1.0]);
        // vCamLookAtDir.xyz (224..236) + vGlobalTime (236) share reg 14.
        put4(offsets::CAM_LOOK_AT_DIR, [0.0, -0.98912, 0.14713, 23.2925]);
        put4(
            offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED,
            [0.0, 23.29197, 0.0, 0.0],
        );
        put4(
            offsets::DAY_NIGHT_HOUR_SUN_DIR,
            [0.0416666679, 0.0, -0.07833850, 0.99692684],
        );
        put4(offsets::SUN_DIFFUSE_INTENSITY, [1.0, 1.0, 1.0, 1.0]);
        put4(
            offsets::MOON_DIFFUSE_INTENSITY,
            [0.21763763, 0.54675138, 1.0, 2.5],
        );
        // GB_TextureHeight (400) + SunSpecularIntensity (404) share reg 25;
        // CubemapIntensity is reg 22 .w (364).
        put4(offsets::GB_TEXTURE_HEIGHT, [1024.0, 1.0, 1.0, 0.0]);
        cb0[offsets::CUBEMAP_INTENSITY..offsets::CUBEMAP_INTENSITY + 4]
            .copy_from_slice(&1.0f32.to_le_bytes());
        cb0
    }

    #[test]
    fn decodes_every_lighting_field_from_mined_cb0() {
        let cb0 = mined_scene_cb0_live_pan2();
        let s = SceneLighting::from_cb0_bytes(&cb0).expect("full CB0 decodes");
        assert_eq!(s.virtual_sun_pos, [2816.0, 600.0, 848.0, 5000.0]);
        assert_eq!(s.virtual_moon_pos, [0.0, 600.0, 1448.0, 550.0]);
        assert_eq!(s.cam_pos, [2958.0, 800.0, 1400.0]);
        assert!((s.global_time - 23.2925).abs() < 1e-3);
        assert_eq!(s.sun_diffuse_intensity, [1.0, 1.0, 1.0, 1.0]);
        assert!((s.moon_diffuse_intensity[3] - 2.5).abs() < 1e-6);
        assert!((s.sun_specular_intensity - 1.0).abs() < 1e-6);
        assert!((s.cubemap_intensity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn day_night_hour_reads_out_of_dot_x() {
        let cb0 = mined_scene_cb0_live_pan2();
        let s = SceneLighting::from_cb0_bytes(&cb0).unwrap();
        // 0.0416666 * 24 == 1.0 -> in-game hour 1.
        assert!(
            (s.day_night_hour() - 1.0).abs() < 1e-3,
            "{}",
            s.day_night_hour()
        );
    }

    #[test]
    fn corpus_is_full_daylight_only() {
        // FoW.w == 0 in every mined capture gates DayNightFactor to 0.
        let cb0 = mined_scene_cb0_live_pan2();
        let s = SceneLighting::from_cb0_bytes(&cb0).unwrap();
        assert!(s.is_full_daylight_capture());
    }

    #[test]
    fn sun_direction_is_a_unit_vector() {
        let cb0 = mined_scene_cb0_live_pan2();
        let r = validate_lighting_law_against_cb0(&cb0).unwrap();
        assert!(
            r.sun_dir_unit_residual.abs() < 2e-3,
            "sun dir unit residual {}",
            r.sun_dir_unit_residual
        );
    }

    #[test]
    fn diffuse_color_hsv_to_linear_law_matches_captured_rgb() {
        // Moon: HSV{0.58,0.5,1.0} -> ToLinear(HSVtoRGBPost) == captured rgb.
        let moon = diffuse_color_rgb_from_hsv_define(defines::MOON_DIFFUSE_COLOR_HSV);
        assert!((moon[0] - 0.21763763).abs() < 1e-4, "moon.r {}", moon[0]);
        assert!((moon[1] - 0.54675138).abs() < 1e-4, "moon.g {}", moon[1]);
        assert!((moon[2] - 1.0).abs() < 1e-4, "moon.b {}", moon[2]);
        // Sun: HSV{0.14,0.0,1.0} -> white (S==0 -> vvv, then ^2.2 == 1).
        let sun = diffuse_color_rgb_from_hsv_define(defines::SUN_DIFFUSE_COLOR_HSV);
        assert_eq!(sun, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn files_derived_residuals_are_zero_against_mined_cb0() {
        let cb0 = mined_scene_cb0_live_pan2();
        let r = validate_lighting_law_against_cb0(&cb0).unwrap();
        assert!(r.sun_height_residual.abs() < 1e-3);
        assert!(r.sun_latitude_residual.abs() < 1e-3);
        assert!(r.moon_height_residual.abs() < 1e-3);
        // Moon .z reproduced by the SUN_LATITUDE + MOON_HEIGHT hypothesis.
        assert!(
            r.moon_z_hypothesis_residual.abs() < 1e-3,
            "moon z {}",
            r.moon_z_hypothesis_residual
        );
        assert!(r.sun_diffuse_rgb_residual < 1e-4);
        assert!(r.sun_intensity_residual.abs() < 1e-6);
        assert!(r.moon_diffuse_rgb_residual < 1e-4);
        assert!(r.moon_intensity_residual.abs() < 1e-6);
        assert!(r.sun_specular_residual.abs() < 1e-6);
        assert!(r.cubemap_intensity_residual.abs() < 1e-6);
    }

    #[test]
    fn camera_follow_hypothesis_is_refuted() {
        // The spec's open question: does sun.x follow the camera x? The mined
        // data says NO: sun.x is a fixed 2816 while cam.x here is 2958 (and
        // ranges 2714..2958 across the corpus). The residual is far from 0.
        let cb0 = mined_scene_cb0_live_pan2();
        let r = validate_lighting_law_against_cb0(&cb0).unwrap();
        assert!(
            r.sun_x_minus_cam_x.abs() > 100.0,
            "sun.x - cam.x = {} (would be ~0 under camera-follow)",
            r.sun_x_minus_cam_x
        );
        assert!((r.sun_x_minus_cam_x - (2816.0 - 2958.0)).abs() < 1e-3);
    }

    #[test]
    fn cb0_decode_fails_closed_on_short_buffer() {
        let short = vec![0u8; offsets::DAY_NIGHT_HOUR_SUN_DIR + 8];
        assert!(SceneLighting::from_cb0_bytes(&short).is_none());
        assert!(validate_lighting_law_against_cb0(&short).is_none());
    }

    #[test]
    fn day_cycle_longitude_law_matches_unpaused_session_final_1_samples() {
        // Measured (hour24, sun.x, moon.x, second_sun.x, second_moon.x)
        // decoded from session_FINAL_1 cb0_samples.jsonl (live, unpaused).
        // hour 0.00 recorded sun.x as the unwrapped 5632 == 0 mod width.
        const MAPX: f32 = 5632.0;
        let samples: [(f32, f32, f32); 3] = [
            (10.147_274, 3_250.772_9, 434.773_44),
            (7.876_204, 3_783.717_3, 967.716_8),
            (15.377_023, 2_023.525_4, 4_839.525_4),
        ];
        for (h, sun_x, moon_x) in samples {
            assert!(
                (virtual_sun_longitude(h, MAPX) - sun_x).abs() < 0.5,
                "sun law at h={h}: {} vs measured {sun_x}",
                virtual_sun_longitude(h, MAPX)
            );
            assert!(
                (virtual_moon_longitude(h, MAPX) - moon_x).abs() < 0.5,
                "moon law at h={h}: {} vs measured {moon_x}",
                virtual_moon_longitude(h, MAPX)
            );
        }
        // The paused corpus's fixed sun.x=2816 is this law at noon.
        assert!((virtual_sun_longitude(12.0, MAPX) - 2816.0).abs() < 1e-3);
        // Wrap behavior: hour 0 and hour 24 are the same longitude, 0.
        assert!((virtual_sun_longitude(0.0, MAPX)).abs() < 1e-3);
        assert!((virtual_sun_longitude(24.0, MAPX)).abs() < 1e-3);
    }

    #[test]
    fn map_size_x_define_matches_heightmap_width() {
        // MAP_SIZE_X is the shipped 5632-wide heightmap width and the wrap
        // period the day-cycle law is fitted against.
        assert_eq!(defines::MAP_SIZE_X, 5632.0);
    }

    #[test]
    fn rig_longitudes_are_byte_exact_captured_values_at_noon() {
        // The single hard gate the whole job rides on: at noon the four rig
        // x lanes the law writes are bit-for-bit the values every paused
        // (noon) capture carries -> the flag-off baseline stays byte-identical.
        // hour24 = (--hour 0.5 fraction) * 24 == 12.0.
        let x = virtual_rig_longitudes_x(12.0, defines::MAP_SIZE_X);
        assert_eq!(x[0].to_bits(), 2816.0f32.to_bits(), "sun.x");
        assert_eq!(x[1].to_bits(), 0.0f32.to_bits(), "moon.x");
        assert_eq!(x[2].to_bits(), 0.0f32.to_bits(), "second_sun.x");
        assert_eq!(x[3].to_bits(), 2816.0f32.to_bits(), "second_moon.x");
        // Positive zero, never -0.0 (which would flip the sign bit and break
        // byte-identity even though it compares == as a float).
        assert!(!x[1].is_sign_negative());
        assert!(!x[2].is_sign_negative());
    }

    #[test]
    fn rig_longitudes_move_with_the_hour_per_the_law() {
        const MAPX: f32 = 5632.0;
        // 06:00 (fraction 0.25): sun is a quarter-day east of noon.
        let six = virtual_rig_longitudes_x(6.0, MAPX);
        assert!((six[0] - 4224.0).abs() < 1e-3, "sun.x@06:00 {}", six[0]);
        assert!((six[1] - 1408.0).abs() < 1e-3, "moon.x@06:00 {}", six[1]);
        // 18:00 (fraction 0.75): antipodal to 06:00 — bodies swapped.
        let eighteen = virtual_rig_longitudes_x(18.0, MAPX);
        assert!(
            (eighteen[0] - 1408.0).abs() < 1e-3,
            "sun.x@18:00 {}",
            eighteen[0]
        );
        assert!(
            (eighteen[1] - 4224.0).abs() < 1e-3,
            "moon.x@18:00 {}",
            eighteen[1]
        );
        // Second bodies always ride the opposite longitude.
        assert_eq!(six[2], six[1]);
        assert_eq!(six[3], six[0]);
    }

    // ---- NIGHT1: the .yzw hour-invariance law, on REAL swept bytes ----

    /// Decode a hex string (from `cb0_samples.jsonl`'s `cb0_hex`) to bytes.
    fn hexbytes(hex: &str) -> Vec<u8> {
        assert_eq!(hex.len() % 2, 0, "odd hex len");
        let nib = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex nibble {c}"),
        };
        hex.as_bytes()
            .chunks(2)
            .map(|p| (nib(p[0]) << 4) | nib(p[1]))
            .collect()
    }

    /// Two REAL scene CB0 samples from session_CAPFIX2_FULL_2 (captured tonight,
    /// UNPAUSED), decoded verbatim from `cb0_samples.jsonl`. Same in-game DATE;
    /// the game hour advances 1 → 6 between them. These are the decisive
    /// hour-invariance control: only `DayNight_Hour_SunDir.x` differs.
    const SWEEP_FULL2_HOUR1_HEX: &str = "daebad3f00000000000000000000000000000000cbf7b53ee7877dbfc6367dbf000000000cea1840d3d9163e8da9163e66337bc52ed562c552421144d753124435683c3f00000000000000000000000000000000267c793db6a6d13e0000000012b713c4cccc1fc333d38bc3438b4cbe67e61344c9021f43d5128c43cdcc4c3e000000000000000072f97f3f72f97f3f00003045000016440000544400409c4500000000000016440000b5440080094400000000000016440000c84200409c4500003045000016440000c8420080094400e03845000048440000af440000803f00000000c6367dbf8da9163ee354d54100000000000000000000000000000000abaa2a3d000000009c6d9e3d9f3b7f3fae18d93cec7eed3cef5c913c0000000012fd833e3f6ba63e3f3d913e00000000000000000000000000000000000000000000000000000000000000000000803fcfad033fd765223f7b23573f6a020000b207213f64e03d3f44094b3f0000803f0000803f0000803f0000803f0000803f66dc5e3ee6f70b3f0000803f00002040000080440000803f00540500f8000000";
    const SWEEP_FULL2_HOUR6_HEX: &str = "daebad3f000000000000000000000000000000000224763e560c7fbfb8ba7ebf0000000053d41940c20ccc3d76cbcb3d55445ec563df50c5d40c2943bbd62d4335683c3f00000000000000000000000000000000edbb283debe7d23e0000000072b302c4173578c2a1c387c3438b4cbe53dd0244b4897442dffb8743cdcc4c3e0000803f0000000048e17a3f8716793f00003045000016440000544400409c4500000000000016440000b5440080094400000000000016440000c84200409c4500003045000016440000c84200800944a8942345e3529b43eaeaa9440000803f00000000b8ba7ebf76cbcb3d9f2b374300000000c0b72843821ff83d000000000000803e000000009c6d9e3d9f3b7f3fae18d93cec7eed3cef5c913c0000803f12fd833e3f6ba63e3f3d913e247a853d000000000000000000000000000000000000000000000000000000000000803fcfad033fd765223f7b23573f00000000b207213f64e03d3f44094b3f0000803f0000803f0000803f0000803f0000803f66dc5e3ee6f70b3f0000803f00002040000080440000803f0000803f00000000";

    #[test]
    fn night_sweep_yzw_is_bit_identical_across_hours_at_one_date() {
        // The core NIGHT1 finding on real bytes: within one in-game date the
        // sun-direction .yzw is BIT-FOR-BIT identical as the hour sweeps; only
        // DayNight_Hour_SunDir.x (the terminator longitude phase) changes.
        let h1 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        let h6 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR6_HEX)).unwrap();

        // The hour DID advance (0.0417 = 1/24 → 0.25 = 6/24).
        assert!(
            (h1.day_night_hour() - 1.0).abs() < 1e-3,
            "h1 {}",
            h1.day_night_hour()
        );
        assert!(
            (h6.day_night_hour() - 6.0).abs() < 1e-3,
            "h6 {}",
            h6.day_night_hour()
        );

        // ...yet .yzw is byte-identical between the two hours.
        for i in 0..3 {
            assert_eq!(
                h1.sun_dir_yzw()[i].to_bits(),
                h6.sun_dir_yzw()[i].to_bits(),
                "sun_dir .yzw component {i} moved with the hour (h1 {} vs h6 {})",
                h1.sun_dir_yzw()[i],
                h6.sun_dir_yzw()[i]
            );
        }

        // Measured shape: .y (dir.x) exactly 0; .w (dir.z) ≈ 1; unit vector.
        let (unit, y_abs, one_minus_w) = night_yzw_residual(h1.sun_dir_yzw());
        assert_eq!(y_abs, 0.0, "sun_dir .y (dir.x) is exactly 0 in the sweep");
        assert!(
            one_minus_w < 3e-3,
            "sun_dir .w (dir.z) ≈ 1, off by {one_minus_w}"
        );
        assert!(unit < 2e-3, "sun_dir is a unit vector, |yzw|-1 = {unit}");

        // And FoW.w is still 0 — the night blend gate is NOT exercised (the
        // sweep validates the .yzw curve, not the FoW.w≠0 night darkening).
        assert_eq!(h1.fow_opacity_time_snowmud_maxspeed[3], 0.0);
        assert_eq!(h6.fow_opacity_time_snowmud_maxspeed[3], 0.0);
    }

    #[test]
    fn render_sun_dir_yzw_echoes_captured_vector_independent_of_hour() {
        // The render law: at ANY hour, the .yzw written is the captured .yzw,
        // bit-for-bit — this is what keeps --hour 0.5 byte-identical (the
        // captured .yzw is echoed, not synthesized) and is proven correct by the
        // hour-invariance finding above.
        let captured = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX))
            .unwrap()
            .sun_dir_yzw();
        for hour24 in [0.0f32, 6.0, 12.0, 18.0, 23.5] {
            let out = render_sun_dir_yzw(hour24, captured);
            for i in 0..3 {
                assert_eq!(
                    out[i].to_bits(),
                    captured[i].to_bits(),
                    "render_sun_dir_yzw changed .yzw[{i}] at hour24 {hour24}"
                );
            }
        }
    }

    // ---- AMBIENT1: the CB0 ambient-rig compose law, on REAL swept bytes ----

    /// The captured CB0 ambient rig from session_CAPFIX2_FULL_2 hour 1 (decoded
    /// from `SWEEP_FULL2_HOUR1_HEX` offsets 272..364), kept as literals so the
    /// gate prints a meaningful diff if either the decode or the law drifts.
    const CAPTURED_AMBIENT_RIG_FULL2: [[f32; 3]; 6] = [
        [0.026501026, 0.028991185, 0.0177445095], // AmbientPosX
        [0.25779015, 0.32503697, 0.2836704],      // AmbientNegX
        [0.0, 0.0, 0.0],                          // AmbientPosY
        [0.0, 0.0, 0.0],                          // AmbientNegY
        [0.51437086, 0.63436645, 0.84038514],     // AmbientPosZ
        [0.62902367, 0.7417052, 0.79311013],      // AmbientNegZ
    ];

    #[test]
    fn gate_cb0_ambient_rig_composes_bit_exact_from_ambient_light_defines() {
        // THE item-1 gate: the CB0 Ambient{Pos,Neg}{X,Y,Z} fields captured from
        // the engine must be reproduced by composing the AMBIENT_LIGHT_* HSV
        // defines (00_graphics.lua:1053-1058) through the shader's manual-input
        // HSVtoRGB (standardfuncsgfx.fxh:145-152, ToLinear pow 2.2).
        let composed = cb0_ambient_rig_from_hsv_defines(vanilla_ambient_light_hsv_defines());
        for hex in [SWEEP_FULL2_HOUR1_HEX, SWEEP_FULL2_HOUR6_HEX] {
            let s = SceneLighting::from_cb0_bytes(&hexbytes(hex)).unwrap();
            let residual = ambient_rig_residual(&s.ambient_rig, &composed);
            assert_eq!(
                residual, 0.0,
                "composed ambient rig residual {residual} vs captured (must be bit-exact)"
            );
            // Bit-exactness, component by component (not just |delta| == 0).
            for (face, (cap, law)) in s.ambient_rig.iter().zip(composed.iter()).enumerate() {
                for lane in 0..3 {
                    assert_eq!(
                        cap[lane].to_bits(),
                        law[lane].to_bits(),
                        "ambient face {face} lane {lane}: captured {} vs composed {}",
                        cap[lane],
                        law[lane]
                    );
                }
            }
        }
        // And the literal expectation table matches the decode (guards the
        // offsets themselves).
        let s = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        assert_eq!(s.ambient_rig, CAPTURED_AMBIENT_RIG_FULL2);
    }

    #[test]
    fn ambient_rig_is_hour_invariant_in_swept_capture() {
        // The captured ambient carries no day/night state: hour 1 and hour 6
        // samples (same date, unpaused sweep) are byte-identical. The per-pixel
        // day/night ambient blend lives in the shader (AmbientLight,
        // standardfuncsgfx.fxh:456-470), not in the CB0 compose.
        let h1 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        let h6 = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR6_HEX)).unwrap();
        for face in 0..6 {
            for lane in 0..3 {
                assert_eq!(
                    h1.ambient_rig[face][lane].to_bits(),
                    h6.ambient_rig[face][lane].to_bits(),
                    "ambient face {face} lane {lane} moved between hours 1 and 6"
                );
            }
        }
    }

    #[test]
    fn cb0_ambient_compose_is_not_the_day_ambient_map_rig() {
        // Negative control for the (corrected) provenance: the CB0 ambient is
        // NOT the shader-side DayAmbientMap* rig (constants.fxh:17-22). E.g.
        // DayAmbientMapPosX = (0.1, 0.1, 0.05) vs captured (0.0265, 0.0290,
        // 0.0177). The right-face comparison alone separates them by > 0.07.
        let composed = cb0_ambient_rig_from_hsv_defines(vanilla_ambient_light_hsv_defines());
        let day_ambient_map_pos_x = [0.1f32, 0.1, 0.05];
        let delta = (composed[0][0] - day_ambient_map_pos_x[0]).abs();
        assert!(
            delta > 0.05,
            "CB0 PosX unexpectedly matches DayAmbientMapPosX (delta {delta})"
        );
    }

    #[test]
    fn ambient_light_defines_parse_from_lua_source() {
        // Verbatim 00_graphics.lua:1052-1058 lines (comments included).
        let source = r#"
	-- hsv color ambient light
	AMBIENT_LIGHT_POS_X = { 0.2,  0.2, 0.2 }, -- right
	AMBIENT_LIGHT_NEG_X = { 0.4,  0.1, 0.6 }, -- left
	AMBIENT_LIGHT_POS_Y = { 0.0,  0.0, 0.0 },-- kills everything
	AMBIENT_LIGHT_NEG_Y = { 0.35, 0.2, 0.0 },-- from under
	AMBIENT_LIGHT_POS_Z = { 0.6,  0.2, 0.924 }, -- top
	AMBIENT_LIGHT_NEG_Z = { 0.55, 0.1, 0.9 }, -- bottom
"#;
        let parsed = ambient_light_hsv_defines_from_graphics_source(source)
            .expect("parse AMBIENT_LIGHT_* from lua source");
        assert_eq!(parsed, vanilla_ambient_light_hsv_defines());
        // Fail-closed on a missing key.
        let err = ambient_light_hsv_defines_from_graphics_source("AMBIENT_LIGHT_POS_X = { 0 }")
            .expect_err("must fail closed");
        assert!(err.contains("AMBIENT_LIGHT_POS_X"), "{err}");
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_graphics_lua_ambient_defines_compose_to_captured_rig() {
        // End-to-end: real shipped 00_graphics.lua -> parse -> compose ->
        // captured CB0 bits. Skips when the vanilla install is absent.
        let path =
            replay_test_gates::require_vanilla_game_root().join("common/defines/00_graphics.lua");
        let source = std::fs::read_to_string(&path).expect("read shipped 00_graphics.lua");
        let parsed = ambient_light_hsv_defines_from_graphics_source(&source)
            .expect("parse AMBIENT_LIGHT_* from the shipped 00_graphics.lua");
        assert_eq!(
            parsed,
            vanilla_ambient_light_hsv_defines(),
            "shipped defines drifted from the cited vanilla values"
        );
        let composed = cb0_ambient_rig_from_hsv_defines(parsed);
        let s = SceneLighting::from_cb0_bytes(&hexbytes(SWEEP_FULL2_HOUR1_HEX)).unwrap();
        let residual = ambient_rig_residual(&s.ambient_rig, &composed);
        assert_eq!(residual, 0.0, "file->law->CB0 residual {residual}");
        println!(
            "vanilla ambient-defines gate: shipped lua -> composed rig bit-exact (residual 0)"
        );
    }

    #[test]
    fn night_yzw_residual_law_holds_over_proven_hour_range() {
        // The two real swept samples span in-game hours 1..6 at one date; assert
        // the law's residual bounds hold on both (proven range for THIS date;
        // cross-date .z drift is the seasonal term, tracked separately).
        for hex in [SWEEP_FULL2_HOUR1_HEX, SWEEP_FULL2_HOUR6_HEX] {
            let s = SceneLighting::from_cb0_bytes(&hexbytes(hex)).unwrap();
            let (unit, y_abs, one_minus_w) = night_yzw_residual(s.sun_dir_yzw());
            assert!(unit < 2e-3, "|yzw|-1 {unit}");
            assert_eq!(y_abs, 0.0, ".y exactly 0");
            assert!(one_minus_w < 3e-3, "1-.w {one_minus_w}");
        }
    }
}
