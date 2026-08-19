//! WATER1 — water reflection / glitter law (files-first, capture-validated).
//!
//! This module encodes the *virtual sun / moon position law* the game water
//! shader consumes for its specular glitter, derived from the shipped game
//! files and validated against captured scene constant buffer 0 (CB0) vectors.
//!
//! # Where the law comes from (shipped files, source of truth)
//!
//! `gfx/FX/standardfuncsgfx.fxh` declares CB0 (`ConstantBuffer(0,0)`) with, in
//! order, `vVirtualSunPos` (float4, byte offset 144) and `vVirtualMoonPos`
//! (float4, offset 160), followed by `vSecondVirtualSunPos` (176) and
//! `vSecondVirtualMoonPos` (192).
//!
//! Water uses a DIFFERENT swizzle from terrain:
//! * terrain: `CalculateSunDirection(pos, vVirtualSunPos.xyz, ...)`  (fxh:631)
//! * water:   `CalculateSunDirectionWater(pos)` = `CalculateSunDirection(pos,
//!   vVirtualSunPos.xwz, ...)`  (fxh:634-636)
//!
//! The `.xwz` swizzle takes the `.w` component in place of `.y`. So the water
//! sun/moon "height" (the Y coordinate of the light source position) is the
//! `.w` component of the packed CB0 vector, NOT the `.y` component. This is the
//! whole mechanism: the game packs the terrain height in `.y` and the *water*
//! height in `.w`, and the two shaders pick the height they want by swizzle.
//!
//! The heights and latitudes are shipped defines in
//! `common/defines/00_graphics.lua`:
//! * `SUN_HEIGHT = 600`, `MOON_HEIGHT = 600`              (packed in `.y`)
//! * `SUN_HEIGHT_WATER = 5000`, `MOON_HEIGHT_WATER = 550` (packed in `.w`)
//! * `SUN_LATITUDE = 848`                                  (sun `.z`)
//! * `SECOND_SUN_LATITUDE = 100`, `SECOND_MOON_LATITUDE = 100` (second `.z`)
//! * `MOON_LATITUDE = 0` (commented "NOT USED"; the primary moon's `.z` is an
//!   engine-runtime longitude, see the tracked gap below).
//!
//! The final water sun DIRECTION is then (fxh:598-626, day-side non-seam case):
//! `normalize(worldPos - sourcePos)`, where `sourcePos = (x, waterHeight, z)`.
//! Because `SUN_HEIGHT_WATER = 5000` (vs terrain's 600), the water sun sits far
//! higher and its direction is nearly vertical, which is what produces the
//! tight LEAN specular glitter lobe.
//!
//! # What is NOT files-derivable (tracked gap)
//!
//! The `.x` (longitude) of each virtual light, and the primary moon's `.z`
//! (1448 in the captures), are computed by the engine's C++ orbit code from the
//! time-of-day (`DayNight_Hour_SunDir`, CB0 offset 256) and are not present in
//! any shipped shader/define file. We therefore treat the packed `.x`/moon `.z`
//! as engine-runtime, modset-bound values read from the captured CB0 (a tracked
//! gap). What this module owns and
//! validate from files is the *packing/swizzle law* — which component carries
//! which define — because that is what selects the water glitter geometry.

/// Shipped `00_graphics.lua` water/sun defines (byte-for-byte from the file).
///
/// Kept as constants so the validation test fails loudly if a mod or a game
/// patch changes them (the resolver-based readers elsewhere parse the live
/// file; these mirror the vanilla values used to derive the law).
pub mod defines {
    /// `SUN_HEIGHT` — terrain sun height, packed in `vVirtualSunPos.y`.
    pub const SUN_HEIGHT: f32 = 600.0;
    /// `MOON_HEIGHT` — terrain moon height, packed in `vVirtualMoonPos.y`.
    pub const MOON_HEIGHT: f32 = 600.0;
    /// `SUN_HEIGHT_WATER` — water sun height, packed in `vVirtualSunPos.w`.
    pub const SUN_HEIGHT_WATER: f32 = 5000.0;
    /// `MOON_HEIGHT_WATER` — water moon height, packed in `vVirtualMoonPos.w`.
    pub const MOON_HEIGHT_WATER: f32 = 550.0;
    /// `SUN_LATITUDE` — primary sun north/south offset, packed in `.z`.
    pub const SUN_LATITUDE: f32 = 848.0;
    /// `SECOND_SUN_LATITUDE` — second sun `.z`.
    pub const SECOND_SUN_LATITUDE: f32 = 100.0;
    /// `SECOND_MOON_LATITUDE` — second moon `.z`.
    pub const SECOND_MOON_LATITUDE: f32 = 100.0;
}

/// A packed CB0 virtual-light vector `(x, y, z, w)` exactly as stored at CB0
/// offsets 144 (sun) / 160 (moon) / 176 (second sun) / 192 (second moon).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualLightPacked {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl VirtualLightPacked {
    /// The 3D source position the *water* shader uses for this light: the
    /// `.xwz` swizzle of the packed vector (fxh:636). Returned as `(x, y, z)`
    /// world coordinates where the Y (height) is the packed `.w`.
    pub fn water_source_position(self) -> [f32; 3] {
        [self.x, self.w, self.z]
    }

    /// The 3D source position the *terrain* shader uses for this light: the
    /// `.xyz` swizzle (fxh:631).
    pub fn terrain_source_position(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// Read a packed vector from raw CB0 bytes at `byte_offset` (little-endian
    /// f32x4). Returns `None` if the buffer is too short.
    pub fn from_cb0_bytes(cb0: &[u8], byte_offset: usize) -> Option<Self> {
        let end = byte_offset.checked_add(16)?;
        let slice = cb0.get(byte_offset..end)?;
        let f = |i: usize| f32::from_le_bytes([slice[i], slice[i + 1], slice[i + 2], slice[i + 3]]);
        Some(Self {
            x: f(0),
            y: f(4),
            z: f(8),
            w: f(12),
        })
    }
}

/// CB0 byte offset of `vVirtualSunPos` (float4). standardfuncsgfx.fxh CB0 layout.
pub const CB0_OFFSET_VIRTUAL_SUN_POS: usize = 144;
/// CB0 byte offset of `vVirtualMoonPos` (float4).
pub const CB0_OFFSET_VIRTUAL_MOON_POS: usize = 160;
/// CB0 byte offset of `vSecondVirtualSunPos` (float4).
pub const CB0_OFFSET_SECOND_VIRTUAL_SUN_POS: usize = 176;
/// CB0 byte offset of `vSecondVirtualMoonPos` (float4).
pub const CB0_OFFSET_SECOND_VIRTUAL_MOON_POS: usize = 192;

/// Result of validating the packing law against a captured CB0 vector.
#[derive(Clone, Debug, PartialEq)]
pub struct PackingLawResidual {
    /// Which packed light this is (for diagnostics).
    pub label: &'static str,
    /// Residual between the shipped define and the captured `.y` height.
    pub terrain_height_residual: f32,
    /// Residual between the shipped water-height define and the captured `.w`.
    pub water_height_residual: f32,
    /// Residual between the shipped latitude define and the captured `.z`
    /// (only meaningful for lights whose `.z` is a define, i.e. the sun and the
    /// second sun/moon; `None` for the primary moon whose `.z` is runtime).
    pub latitude_residual: Option<f32>,
}

/// Validate the water virtual-sun *packing law* against a captured CB0.
///
/// For the primary sun and both second lights, the `.y` height, `.w`
/// water-height, and `.z` latitude are shipped defines, so the residuals must
/// be ~0. For the primary moon, `.y`/`.w` are defines but `.z` is an
/// engine-runtime longitude (no define), so its latitude residual is `None`.
///
/// Returns one residual record per light. A caller asserts each numeric
/// residual is within tolerance.
pub fn validate_packing_law_against_cb0(cb0: &[u8]) -> Option<Vec<PackingLawResidual>> {
    let sun = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_VIRTUAL_SUN_POS)?;
    let moon = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_VIRTUAL_MOON_POS)?;
    let second_sun = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_SECOND_VIRTUAL_SUN_POS)?;
    let second_moon = VirtualLightPacked::from_cb0_bytes(cb0, CB0_OFFSET_SECOND_VIRTUAL_MOON_POS)?;

    Some(vec![
        PackingLawResidual {
            label: "vVirtualSunPos",
            terrain_height_residual: sun.y - defines::SUN_HEIGHT,
            water_height_residual: sun.w - defines::SUN_HEIGHT_WATER,
            latitude_residual: Some(sun.z - defines::SUN_LATITUDE),
        },
        PackingLawResidual {
            label: "vVirtualMoonPos",
            terrain_height_residual: moon.y - defines::MOON_HEIGHT,
            water_height_residual: moon.w - defines::MOON_HEIGHT_WATER,
            // Primary moon .z is an engine-runtime longitude (tracked gap).
            latitude_residual: None,
        },
        PackingLawResidual {
            label: "vSecondVirtualSunPos",
            terrain_height_residual: second_sun.y - defines::SUN_HEIGHT,
            water_height_residual: second_sun.w - defines::SUN_HEIGHT_WATER,
            latitude_residual: Some(second_sun.z - defines::SECOND_SUN_LATITUDE),
        },
        PackingLawResidual {
            label: "vSecondVirtualMoonPos",
            terrain_height_residual: second_moon.y - defines::MOON_HEIGHT,
            water_height_residual: second_moon.w - defines::MOON_HEIGHT_WATER,
            latitude_residual: Some(second_moon.z - defines::SECOND_MOON_LATITUDE),
        },
    ])
}

// ===========================================================================
// WATER2 — the underwater seafloor pass law (produces `WaterRefraction`)
// ===========================================================================

/// File-derived constants for the underwater/seafloor pass that the game
/// renders to fill the `WaterRefraction` buffer (pdxwater.shader slot 5).
///
/// # Source of truth (shipped files, verified line-by-line)
///
/// The pass is `pdxmap.shader` `Effect underwater` (pdxmap.shader:495-499),
/// which binds the SAME terrain `VertexShader` (pdxmap.shader:183) and the
/// `PixelShaderUnderwater` MainCode (pdxmap.shader:407-454) with NO extra
/// `Defines`. `PixelShaderUnderwater`:
///
/// * `clip( WATER_HEIGHT - Input.prepos.y + TERRAIN_WATER_CLIP_HEIGHT );`
///   (line 411) — discards fragments where the argument is `< 0`, i.e. keeps
///   only `prepos.y <= WATER_HEIGHT + TERRAIN_WATER_CLIP_HEIGHT` (the seafloor
///   and shallow shore; mountains above that height are discarded).
/// * `waterColorTint = tex2D( TerrainColorTint, uv2 ).rgb * WATER_COLOR_LIGHTNESS;`
///   (lines 421-422).
/// * depth-fog: `vWaterAlpha = saturate( (prepos.y - 5.3) / (13.0 - 5.3) );`
///   (lines 424-425, with `vMin = 5.3`, `vMax = 13.0` literals).
/// * `vGlossiness = MAP_SPECULAR_WIDTH;` (line 427).
/// * lit seafloor via `CalculateSunLight` + `CalculatePointLights` +
///   `GetShadowScaled( SHADOW_WEIGHT_TERRAIN, ... )` (lines 444-449).
/// * `vOut = lerp( waterColorTint, litSeafloor, vWaterAlpha );` (line 451) —
///   the depth fog: at the sea BOTTOM `prepos.y` is LOW, so `vWaterAlpha` is
///   LOW ⇒ the dark-blue tint dominates; as the floor rises toward the
///   surface `prepos.y` grows ⇒ the fragment TRENDS toward the lit seafloor.
///   It never reaches fully-lit: the clip at 12.5 caps `vWaterAlpha` at
///   `(12.5 − 5.3) / 7.7 ≈ 0.935 < 1.0`.
/// * returns `float4( vOut, 1.0 )` (line 453).
///
/// The literal constants are shipped defines in `constants.fxh`:
/// `WATER_HEIGHT = 9.5` (line 133), `TERRAIN_WATER_CLIP_HEIGHT = 3.0`
/// (line 57), `WATER_COLOR_LIGHTNESS = 0.5` (line 90),
/// `MAP_SPECULAR_WIDTH = 15.0` (line 44), `SHADOW_WEIGHT_TERRAIN = 0.7`
/// (line 170). The depth-fog bounds 5.3 / 13.0 are inline literals in the
/// pixel shader (not named defines).
pub mod underwater_defines {
    /// `WATER_HEIGHT` (constants.fxh:133). Sea level in world Y.
    pub const WATER_HEIGHT: f32 = 9.5;
    /// `TERRAIN_WATER_CLIP_HEIGHT` (constants.fxh:57). Extra headroom added to
    /// the clip so shallow shore fragments just above sea level are retained.
    pub const TERRAIN_WATER_CLIP_HEIGHT: f32 = 3.0;
    /// `WATER_COLOR_LIGHTNESS` (constants.fxh:90). Darkens the seafloor tint.
    pub const WATER_COLOR_LIGHTNESS: f32 = 0.5;
    /// `MAP_SPECULAR_WIDTH` (constants.fxh:44). Seafloor specular glossiness.
    pub const MAP_SPECULAR_WIDTH: f32 = 15.0;
    /// `SHADOW_WEIGHT_TERRAIN` (constants.fxh:170). Shadow term scaler.
    pub const SHADOW_WEIGHT_TERRAIN: f32 = 0.7;
    /// Depth-fog lower bound `vMin` (pdxmap.shader:424, inline literal `5.3f`).
    pub const DEPTH_FOG_MIN_Y: f32 = 5.3;
    /// Depth-fog upper bound `vMax` (pdxmap.shader:425, inline literal `13.0f`).
    pub const DEPTH_FOG_MAX_Y: f32 = 13.0;
}

/// The seafloor clip-height threshold in world Y: fragments with
/// `prepos.y <= threshold` are KEPT (rendered), higher fragments discarded.
///
/// `clip( WATER_HEIGHT - prepos.y + TERRAIN_WATER_CLIP_HEIGHT )` keeps the
/// fragment while the argument `>= 0`, i.e. `prepos.y <= WATER_HEIGHT +
/// TERRAIN_WATER_CLIP_HEIGHT = 9.5 + 3.0 = 12.5`.
pub fn underwater_clip_height_threshold() -> f32 {
    underwater_defines::WATER_HEIGHT + underwater_defines::TERRAIN_WATER_CLIP_HEIGHT
}

/// The depth-fog blend factor `vWaterAlpha` for a seafloor fragment at world
/// height `prepos_y`: `saturate( (prepos_y - 5.3) / (13.0 - 5.3) )`
/// (pdxmap.shader:424-425). 0 at/below `DEPTH_FOG_MIN_Y` (fully tinted deep
/// water), 1 at/above `DEPTH_FOG_MAX_Y` (fully lit seafloor).
pub fn underwater_depth_fog_alpha(prepos_y: f32) -> f32 {
    let span = underwater_defines::DEPTH_FOG_MAX_Y - underwater_defines::DEPTH_FOG_MIN_Y;
    ((prepos_y - underwater_defines::DEPTH_FOG_MIN_Y) / span).clamp(0.0, 1.0)
}

/// WATER3/WATER5 — the underwater pass's per-pass output-matching correction.
///
/// # Status (WATER5, 2026-07-07): NOT an engine CB0 light-state law — REFUTED.
///
/// The WATER3 hypothesis was that the engine *swaps an absorbed light state into
/// the underwater pass's CB0 at render time* (it multiplied the scene CB0's
/// `SunDiffuseIntensity.rgb`/`MoonDiffuseIntensity.rgb` by this constant on the
/// theory that no un-absorbed white-sun CB0 could produce the dim-blue captured
/// `WaterRefraction`). **That premise is now disproved by direct capture.**
/// `session_CAPFIX2_GAPS_4` captured the engine's OWN underwater pass for the
/// first time (10 `DrawIndexed` draws with the underwater PS
/// `c1bfddff00056313`). Decoding the scene CB0 (slot 0, 416 B) bound to those
/// engine underwater draws (see [`underwater_cb0_absorption_ratio`] and the
/// `engine_underwater_cb0_carries_unabsorbed_white_sun` test) shows:
///   * `SunDiffuseIntensity  = (1, 1, 1, 1)`      — WHITE, UN-absorbed.
///   * `MoonDiffuseIntensity = (0.2176, 0.5468, 1, 2.5)` — the shipped moon
///     color (`light_law::diffuse_color_rgb_from_hsv_define`), UN-absorbed.
///   * `vFoWOpacity_..._MaxGameSpeed.w = 0` (byte 252) — day-gated / full day.
/// Byte-diffing this underwater CB0 against the SAME session's terrain scene
/// CB0 (`cb0_samples.jsonl`, matched wall-clock) leaves ONLY camera position,
/// time counters, and the shadow/FoW fade `.w` lanes different; EVERY light
/// color field is byte-identical. The engine binds the *same* white-sun,
/// day-gated CB0 to the underwater pass as to terrain — it applies NO CB0 light
/// absorption. (Codex-reviewed, WATER5.)
///
/// # Where the dim-blue actually comes from (files-first)
///
/// The shipped `PixelShaderUnderwater` (pdxmap.shader:449-451) returns
/// `vOut = lerp( waterColorTint, litSeafloor, vWaterAlpha )` with
/// `waterColorTint = tex2D(TerrainColorTint, uv2).rgb * WATER_COLOR_LIGHTNESS`
/// (`*0.5`). Over the open sea the shipped `map/heightmap.bmp` puts the whole
/// floor at raw 89 ⇒ world y = 8.9, so `vWaterAlpha = (8.9-5.3)/7.7 = 0.4675`
/// (a CONSTANT over deep ocean). The output is therefore dominated by the
/// `(1 - 0.4675) = 0.5325 ×` dark-bluish `TerrainColorTint*0.5` arm, with only
/// ~47% weight on the lit arm. **The dim blue is the seafloor TINT texture, not
/// a light-color absorption.** The engine's underwater draws in this session
/// bind exactly that `TerrainColorTint` (fmt `B8G8R8A8`) and the 512×512
/// `TerrainDiffuse` detail atlas — no absorbed light, no special second CB.
///
/// # Why this constant remains in the law module (forensic evidence only)
///
/// It remains an EMPIRICAL output-matching correction, not a mechanism: the
/// app's own reconstruction ran ~5× too bright (surfacing the shipped 32-world-
/// px `TerrainDiffuse` detail repeat, `pdxmap.shader:414`, as the WATER2
/// oracle's "checkerboard"), most plausibly because the app's re-render weighted
/// the lit arm higher than the engine (a higher effective `vWaterAlpha`, or a
/// brighter/synthesized `TerrainColorTint`) rather than because the engine
/// absorbs light. Multiplying the direct light by `(0.133, 0.234, 0.570)`
/// darkened that lit arm into agreement and measurably improved one oracle (mean
/// abs err 20.84 → 12.40), but it is now blocked from product
/// rendering. The engine's true light law for this pass is closed: *scene CB0
/// unchanged* (see [`underwater_cb0_absorption_ratio`]). This constant and helper
/// remain only for forensic comparisons and regression tests that demonstrate why
/// the old hypothesis was rejected.
///
/// Modset-bound evidence: fitted from vanilla `live_PAN2`, refuted-as-a-light-
/// law from vanilla `session_CAPFIX2_GAPS_4`. Re-validate per modset (the
/// `PROJECT.md` heading "Content and mod contract").
pub const UNDERWATER_LIGHT_ABSORPTION: [f32; 3] = [0.133, 0.234, 0.570];

/// Forensic helper: apply [`UNDERWATER_LIGHT_ABSORPTION`] to a raw 416-byte
/// scene CB0 image:
/// multiplies the rgb of `SunDiffuseIntensity` (byte offset 368) and
/// `MoonDiffuseIntensity` (384), leaving their `.a` intensity lanes and every
/// other field untouched. The CB0 ambient cube is deliberately NOT patched:
/// `PixelShaderUnderwater`'s `ComposeLight` uses the compiled-in static
/// `DayAmbientMap*` constants (standardfuncsgfx.fxh:475-494,683), so a CB0
/// ambient patch would be inert for this pass (Codex-confirmed).
/// Offsets per `light_law::offsets` (standardfuncsgfx.fxh CB0 packing).
/// Fails closed if the buffer is shorter than the CB0 length (416).
pub fn apply_underwater_light_absorption(cb0: &mut [u8]) -> crate::Result<()> {
    use crate::light_law::offsets;
    if cb0.len() < offsets::CB0_LEN {
        return Err(format!(
            "--water-full underwater pass blocked: scene CB0 has {} bytes, need {} to apply \
             the underwater light absorption",
            cb0.len(),
            offsets::CB0_LEN
        )
        .into());
    }
    let direct_light_offsets = [
        offsets::SUN_DIFFUSE_INTENSITY,
        offsets::MOON_DIFFUSE_INTENSITY,
    ];
    for base in direct_light_offsets {
        for (channel, absorption) in UNDERWATER_LIGHT_ABSORPTION.iter().enumerate() {
            let at = base + channel * 4;
            let raw = f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]]);
            let absorbed = raw * absorption;
            cb0[at..at + 4].copy_from_slice(&absorbed.to_le_bytes());
        }
    }
    Ok(())
}

/// WATER5 — the measured light-color delta between the engine's own underwater
/// pass CB0 and the scene (terrain) CB0 of the same frame.
///
/// This is the decode that SETTLES whether the engine applies a light-color
/// absorption to the underwater pass (the WATER3 hypothesis, now refuted): if
/// the engine absorbed, the per-channel ratio `underwater_sun / scene_sun` and
/// `underwater_moon / scene_moon` would be the absorption filter
/// (`~(0.133, 0.234, 0.570)`); the captured
/// `session_CAPFIX2_GAPS_4` underwater CB0 instead gives ratio `1.0` on every
/// channel — the engine binds the scene light rig UNCHANGED.
///
/// `underwater_cb0` is the slot-0 scene CB0 bound to the engine's underwater PS
/// draw; `scene_cb0` is the terrain/scene CB0 of the same frame. Both must be at
/// least [`crate::light_law::offsets::CB0_LEN`] bytes. Returns per-channel
/// ratios for sun and moon rgb (`None` if a scene channel is 0, i.e. no ratio
/// is defined), computed from `light_law::offsets`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnderwaterLightDelta {
    /// `underwater SunDiffuseIntensity.rgb ./ scene SunDiffuseIntensity.rgb`.
    pub sun_rgb_ratio: [Option<f32>; 3],
    /// `underwater MoonDiffuseIntensity.rgb ./ scene MoonDiffuseIntensity.rgb`.
    pub moon_rgb_ratio: [Option<f32>; 3],
    /// The raw underwater `SunDiffuseIntensity.rgb` (for the delta table).
    pub underwater_sun_rgb: [f32; 3],
    /// The raw scene `SunDiffuseIntensity.rgb`.
    pub scene_sun_rgb: [f32; 3],
    /// The raw underwater `MoonDiffuseIntensity.rgb`.
    pub underwater_moon_rgb: [f32; 3],
    /// The raw scene `MoonDiffuseIntensity.rgb`.
    pub scene_moon_rgb: [f32; 3],
    /// Underwater `vFoWOpacity_..._MaxGameSpeed.w` (byte 252): 0 ⇒ day-gated.
    pub underwater_day_gate: f32,
}

impl UnderwaterLightDelta {
    /// True iff the engine's underwater pass applied NO light-color absorption,
    /// i.e. every defined sun/moon channel ratio is within `tol` of 1.0. This is
    /// the WATER5 conclusion for `session_CAPFIX2_GAPS_4` (ratios == 1.0 exactly).
    pub fn is_unabsorbed(&self, tol: f32) -> bool {
        self.sun_rgb_ratio
            .iter()
            .chain(self.moon_rgb_ratio.iter())
            .all(|r| match r {
                Some(v) => (v - 1.0).abs() <= tol,
                None => true,
            })
    }
}

/// Read `SunDiffuseIntensity.rgb` / `MoonDiffuseIntensity.rgb` from two CB0s and
/// compute the underwater-vs-scene light ratio. Fails closed (`None`) if either
/// buffer is shorter than the CB0 layout.
pub fn underwater_cb0_absorption_ratio(
    underwater_cb0: &[u8],
    scene_cb0: &[u8],
) -> Option<UnderwaterLightDelta> {
    use crate::light_law::offsets;
    if underwater_cb0.len() < offsets::CB0_LEN || scene_cb0.len() < offsets::CB0_LEN {
        return None;
    }
    let rgb = |cb0: &[u8], base: usize| -> [f32; 3] {
        let f = |i: usize| {
            let at = base + i * 4;
            f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]])
        };
        [f(0), f(1), f(2)]
    };
    let ratio = |num: [f32; 3], den: [f32; 3]| -> [Option<f32>; 3] {
        [
            if den[0] != 0.0 {
                Some(num[0] / den[0])
            } else {
                None
            },
            if den[1] != 0.0 {
                Some(num[1] / den[1])
            } else {
                None
            },
            if den[2] != 0.0 {
                Some(num[2] / den[2])
            } else {
                None
            },
        ]
    };
    let uw_sun = rgb(underwater_cb0, offsets::SUN_DIFFUSE_INTENSITY);
    let sc_sun = rgb(scene_cb0, offsets::SUN_DIFFUSE_INTENSITY);
    let uw_moon = rgb(underwater_cb0, offsets::MOON_DIFFUSE_INTENSITY);
    let sc_moon = rgb(scene_cb0, offsets::MOON_DIFFUSE_INTENSITY);
    let day_gate = {
        let at = offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED + 12;
        f32::from_le_bytes([
            underwater_cb0[at],
            underwater_cb0[at + 1],
            underwater_cb0[at + 2],
            underwater_cb0[at + 3],
        ])
    };
    Some(UnderwaterLightDelta {
        sun_rgb_ratio: ratio(uw_sun, sc_sun),
        moon_rgb_ratio: ratio(uw_moon, sc_moon),
        underwater_sun_rgb: uw_sun,
        scene_sun_rgb: sc_sun,
        underwater_moon_rgb: uw_moon,
        scene_moon_rgb: sc_moon,
        underwater_day_gate: day_gate,
    })
}

/// The exact set of pixel-shader shader-resource slots (by declared sampler
/// name and register index) that `PixelShaderUnderwater` samples, derived from
/// the shader body (pdxmap.shader:407-454) and the shared sampler block
/// (pdxmap.shader:10-165). These MUST all be present in the terrain draw whose
/// draws we re-render, or the underwater pass cannot be reproduced.
///
/// * `TerrainDiffuse`   slot 0  (line 414)
/// * `HeightNormal`     slot 1  (line 413)
/// * `TerrainColorTint` slot 2  (line 421)
/// * `LightIndexMap`    slot 10 (line 447, via `CalculatePointLights`)
/// * `LightDataMap`     slot 11 (line 447, via `CalculatePointLights`)
/// * `ShadowMap`        slot 12 (line 444, via `GetShadowScaled`)
pub const UNDERWATER_PS_REQUIRED_SRV_SLOTS: &[(&str, u32)] = &[
    ("TerrainDiffuse", 0),
    ("HeightNormal", 1),
    ("TerrainColorTint", 2),
    ("LightIndexMap", 10),
    ("LightDataMap", 11),
    ("ShadowMap", 12),
];

// ===========================================================================
// WATER3 — the LEAN specular (sun-glint) law + its overflow analysis
// ===========================================================================
//
// The the game water glint is a LEAN (Linear Efficient Antialiased Normal mapping)
// specular lobe. The shipped `Effect water` PS (which the app REPLAYS as
// compiled DXBC — it is not re-authored here) computes, per
// `pdxwater.shader` main:
//
//   float vSpecMap = tex2D( SpecularMap, uv ).a;            // line 289
//   float vFlatten = vSpecMap;                              // line 294
//   B *= vFlatten;  M *= vFlatten * vFlatten;               // lines 295-296
//   float3 sigma = M - float3( B*B, B.x*B.y );              // line 306
//   float det = sigma.x*sigma.y - sigma.z*sigma.z;          // line 307
//   float e = HWave.x*HWave.x*sigma.y + HWave.y*HWave.y*sigma.x
//             - 2*HWave.x*HWave.y*sigma.z;                   // line 308
//   float spec = (det<=0) ? 0 : exp(-0.5*e/det) / sqrt(det); // line 309
//   float vGlossiness = (spec/9.0f) * (1 - vSpecMap);        // line 322
//
// `vGlossiness` then drives `_Glossiness = vSpecularPower` in
// `CalculatePBRSpecularPower` (water has NO `PDX_IMPROVED_BLINN_PHONG`, so it
// takes this branch, `standardfuncsgfx.fxh:522-530`):
//
//   vSpec = FresnelSchlick(...) * ((vSpecularPower + 2)/8)
//           * saturate( pow(NdotH, vSpecularPower) ) * NdotL * vLightColor;
//
// THE OVERFLOW (files-derived): the LEAN NDF is a NORMALIZED Gaussian whose
// PEAK value is `spec ~= 1/sqrt(det)` (at the specular direction where the
// exponent `e -> 0`). As the slope covariance `det -> 0+` (glassy/calm water),
// `spec -> +INF`, so `vGlossiness -> +INF`, and the `(vSpecularPower + 2)/8`
// normalization term is likewise UNBOUNDED. In the HDR `R16G16B16A16_FLOAT`
// scene target this overflows to `+Inf`, which becomes `NaN` through the
// downstream tonemap/bloom (a NaN typically reads as a saturated/garbage pixel
// — the reported "blown-out white disc with red artifacts"). There is NO clamp
// in the shipped shader to mirror (verified: `pdxwater.shader` and
// `standardfuncsgfx.fxh` add none).
//
// WHY THE SHIPPED PIPELINE DOES NOT BLOW OUT (the "clamp the shader implies"):
// the taming is entirely in `SpecularMap.a` (= `vSpecMap`) plus the shore
// alpha, NOT a numeric clamp:
//   1. `vFlatten = vSpecMap` multiplies `B` and `M *= vFlatten^2`, so where
//      `vSpecMap` is small the slope statistics `sigma` shrink toward 0 — the
//      water is treated as flat, so no wide glint lobe forms there.
//   2. `vGlossiness = (spec/9) * (1 - vSpecMap)`: where `vSpecMap -> 1` (open
//      sea in the shipped SpecularMap) the `(1 - vSpecMap)` factor -> 0, so the
//      glossiness (and thus the unbounded normalization) collapses to ~0.
//   3. Near shore, `waterShore = saturate((waterHeight-0.954)*25)` drives the
//      output alpha `1 - waterShore -> 0` (pdxwater.shader:282,414), and the
//      src_alpha/inv_src_alpha blend fades any residual bright fragment out.
//
// CONSEQUENCE FOR THE APP (WATER3 conclusion, Codex-refuted): the app binds the
// REAL captured `SpecularMap` (slot 4, its own `.a` channel — verified not
// mean-flattened, format preserved) and replays the shipped PS with the
// captured src_alpha blend, so it reproduces the game's taming; there is no
// app-specific input that pushes `det -> 0` where the game would not. The
// functions below encode this law so a regression that (e.g.) flattened
// `SpecularMap.a` to a constant, or that fed the NDF un-flattened slopes, would
// be caught by the taming tests.

/// The LEAN slope-covariance determinant `det = sigma.x*sigma.y - sigma.z^2`
/// with `sigma = M - (B.x^2, B.y^2, B.x*B.y)` AFTER the `SpecularMap.a` flatten
/// (`B *= vSpecMap`, `M *= vSpecMap^2`), exactly as `pdxwater.shader:294-307`.
/// `b` is the pre-flatten LEAN first moment `(B.x, B.y)`, `m` the pre-flatten
/// second moment `(M.x, M.y, M.z)` from `SampleWater`.
pub fn lean_flattened_det(b: [f32; 2], m: [f32; 3], spec_map_a: f32) -> f32 {
    let vf = spec_map_a;
    let bx = b[0] * vf;
    let by = b[1] * vf;
    let mx = m[0] * vf * vf;
    let my = m[1] * vf * vf;
    let mz = m[2] * vf * vf;
    let sx = mx - bx * bx;
    let sy = my - by * by;
    let sz = mz - bx * by;
    sx * sy - sz * sz
}

/// The shipped LEAN specular peak `spec = 1/sqrt(det)` (the value at the
/// specular direction, `e -> 0`, `pdxwater.shader:309`). Returns `0.0` for
/// `det <= 0` exactly like the shader's `(det <= 0) ? 0` guard. This is the
/// UNBOUNDED quantity: it grows without limit as `det -> 0+`.
pub fn lean_specular_peak(det: f32) -> f32 {
    if det <= 0.0 {
        0.0
    } else {
        1.0 / det.sqrt()
    }
}

/// The shipped water glossiness `vGlossiness = (spec/9) * (1 - vSpecMap)`
/// (`pdxwater.shader:322`) at the specular peak. This is what feeds
/// `_Glossiness`/`vSpecularPower`; the `(1 - vSpecMap)` factor is the shader's
/// implied bound — as `vSpecMap -> 1` it collapses the glint to ~0.
pub fn lean_specular_peak_glossiness(det: f32, spec_map_a: f32) -> f32 {
    lean_specular_peak(det) / 9.0 * (1.0 - spec_map_a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a CB0 byte buffer carrying the four virtual-light vectors exactly
    /// as measured in the captured scene CB0 of restore_gate_1 AND live_PAN2:
    ///   vVirtualSunPos        = (2816, 600, 848, 5000)
    ///   vVirtualMoonPos       = (0,    600, 1448, 550)
    ///   vSecondVirtualSunPos  = (0,    600, 100, 5000)
    ///   vSecondVirtualMoonPos = (2816, 600, 100, 550)
    /// (measured directly from
    ///  C:\hoi4_evidence\capture_corpus_preserved\restore_gate_1\map_frame_draw000_ps_constant_buffer_slot0.bin
    ///  and .../live_PAN2/water_frame_draw000_ps_constant_buffer_slot0.bin)
    fn captured_cb0_from_restore_gate_1_and_live_pan2() -> Vec<u8> {
        let mut cb0 = vec![0u8; 272];
        let mut put = |off: usize, v: [f32; 4]| {
            for (i, comp) in v.iter().enumerate() {
                cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&comp.to_le_bytes());
            }
        };
        put(CB0_OFFSET_VIRTUAL_SUN_POS, [2816.0, 600.0, 848.0, 5000.0]);
        put(CB0_OFFSET_VIRTUAL_MOON_POS, [0.0, 600.0, 1448.0, 550.0]);
        put(
            CB0_OFFSET_SECOND_VIRTUAL_SUN_POS,
            [0.0, 600.0, 100.0, 5000.0],
        );
        put(
            CB0_OFFSET_SECOND_VIRTUAL_MOON_POS,
            [2816.0, 600.0, 100.0, 550.0],
        );
        cb0
    }

    #[test]
    fn water_swizzle_takes_w_as_height() {
        // The captured sun packs terrain height 600 in .y and water height 5000
        // in .w; the water swizzle .xwz must expose 5000 as the Y of the source
        // position, while terrain exposes 600.
        let sun = VirtualLightPacked {
            x: 2816.0,
            y: defines::SUN_HEIGHT,
            z: defines::SUN_LATITUDE,
            w: defines::SUN_HEIGHT_WATER,
        };
        assert_eq!(sun.water_source_position(), [2816.0, 5000.0, 848.0]);
        assert_eq!(sun.terrain_source_position(), [2816.0, 600.0, 848.0]);
    }

    #[test]
    fn packing_law_reproduces_captured_cb0_vectors() {
        let cb0 = captured_cb0_from_restore_gate_1_and_live_pan2();
        let residuals =
            validate_packing_law_against_cb0(&cb0).expect("CB0 long enough for all four lights");
        assert_eq!(residuals.len(), 4);
        for r in &residuals {
            assert!(
                r.terrain_height_residual.abs() < 1e-3,
                "{}: terrain height residual {} exceeds tolerance",
                r.label,
                r.terrain_height_residual
            );
            assert!(
                r.water_height_residual.abs() < 1e-3,
                "{}: water height (.w) residual {} exceeds tolerance",
                r.label,
                r.water_height_residual
            );
            if let Some(lat) = r.latitude_residual {
                assert!(
                    lat.abs() < 1e-3,
                    "{}: latitude (.z) residual {} exceeds tolerance",
                    r.label,
                    lat
                );
            }
        }
        // Explicitly confirm the water source position of the primary sun sits
        // at the 5000 water height (the glitter-defining fact), not 600.
        let sun = VirtualLightPacked::from_cb0_bytes(&cb0, CB0_OFFSET_VIRTUAL_SUN_POS).unwrap();
        assert_eq!(sun.water_source_position(), [2816.0, 5000.0, 848.0]);
    }

    #[test]
    fn cb0_reader_fails_closed_on_short_buffer() {
        let short = vec![0u8; CB0_OFFSET_VIRTUAL_MOON_POS + 8];
        // Sun (offset 144) fits, but second-moon (offset 192) does not.
        assert!(VirtualLightPacked::from_cb0_bytes(&short, CB0_OFFSET_VIRTUAL_SUN_POS).is_some());
        assert!(
            VirtualLightPacked::from_cb0_bytes(&short, CB0_OFFSET_SECOND_VIRTUAL_MOON_POS)
                .is_none()
        );
        assert!(validate_packing_law_against_cb0(&short).is_none());
    }

    // ----- WATER2 underwater law -----

    #[test]
    fn underwater_clip_threshold_from_shipped_defines() {
        // clip( WATER_HEIGHT - prepos.y + TERRAIN_WATER_CLIP_HEIGHT ) keeps
        // prepos.y <= 9.5 + 3.0 = 12.5 (constants.fxh:133,57).
        assert_eq!(underwater_clip_height_threshold(), 12.5);
        assert_eq!(underwater_defines::WATER_HEIGHT, 9.5);
        assert_eq!(underwater_defines::TERRAIN_WATER_CLIP_HEIGHT, 3.0);
    }

    #[test]
    fn underwater_depth_fog_alpha_matches_shader_lerp() {
        // vWaterAlpha = saturate((prepos.y - 5.3) / (13.0 - 5.3))
        // (pdxmap.shader:424-425). 0 at/below vMin, 1 at/above vMax, linear
        // between, and the midpoint is (5.3+13)/2 = 9.15 -> 0.5.
        assert_eq!(underwater_depth_fog_alpha(5.3), 0.0);
        assert_eq!(underwater_depth_fog_alpha(0.0), 0.0);
        assert_eq!(underwater_depth_fog_alpha(13.0), 1.0);
        assert_eq!(underwater_depth_fog_alpha(20.0), 1.0);
        let mid = underwater_depth_fog_alpha(9.15);
        assert!((mid - 0.5).abs() < 1e-4, "midpoint alpha {mid}");
        // Monotonic increasing with depth-from-bottom (rising seafloor).
        assert!(underwater_depth_fog_alpha(7.0) < underwater_depth_fog_alpha(10.0));
    }

    #[test]
    fn underwater_required_srv_slots_are_the_shader_sampled_set() {
        // PixelShaderUnderwater samples exactly these six, at these register
        // indices (verified against the compiled PS reflection and
        // pdxmap.shader:413-447). This is the fail-closed contract the pass
        // asserts on the terrain draws it re-renders.
        let slots: Vec<(&str, u32)> = UNDERWATER_PS_REQUIRED_SRV_SLOTS.to_vec();
        assert_eq!(
            slots,
            vec![
                ("TerrainDiffuse", 0),
                ("HeightNormal", 1),
                ("TerrainColorTint", 2),
                ("LightIndexMap", 10),
                ("LightDataMap", 11),
                ("ShadowMap", 12),
            ]
        );
    }

    // ----- WATER3 underwater output-matching correction (fallback shape) -----

    #[test]
    fn underwater_light_absorption_fallback_is_blue_dominant_and_bounded() {
        // This is a stability/shape test of the FALLBACK output-matching
        // correction, NOT a proof of engine behavior (WATER5: the engine's
        // captured underwater CB0 is un-absorbed, see
        // `engine_underwater_cb0_carries_unabsorbed_white_sun`). The correction
        // must attenuate every channel (components in (0,1)) and be
        // blue-dominant (r < g < b) — the water-absorption shape both empirical
        // fits show (lit-arm ratio (0.133,0.234,0.570); scene fit
        // (0.13,0.20,0.42)).
        let [r, g, b] = UNDERWATER_LIGHT_ABSORPTION;
        assert!(r > 0.0 && r < 1.0, "red absorption {r}");
        assert!(g > 0.0 && g < 1.0, "green absorption {g}");
        assert!(b > 0.0 && b < 1.0, "blue absorption {b}");
        assert!(
            r < g && g < b,
            "absorption must be blue-dominant: {r} {g} {b}"
        );
    }

    #[test]
    fn underwater_light_absorption_patches_direct_light_only() {
        use crate::light_law::offsets;
        // Build a CB0 image with every f32 set to 1.0 and verify: ONLY the
        // sun/moon rgb are scaled by the absorption. The CB0 ambient cube must
        // be PRESERVED (PixelShaderUnderwater's ComposeLight uses the static
        // DayAmbientMap* constants, standardfuncsgfx.fxh:475-494,683 — a CB0
        // ambient patch would be inert and is deliberately not applied), as
        // must every .w lane (shadow fade, fog fade, mesh alpha, fog
        // multiplier, cubemap intensity, sun/moon intensity .a) and every
        // other field (camera, HdrRange, virtual light positions).
        let mut cb0 = vec![0u8; offsets::CB0_LEN];
        for chunk in cb0.chunks_exact_mut(4) {
            chunk.copy_from_slice(&1.0f32.to_le_bytes());
        }
        apply_underwater_light_absorption(&mut cb0).expect("CB0 long enough");
        let read = |at: usize| f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]]);
        for base in [
            offsets::SUN_DIFFUSE_INTENSITY,
            offsets::MOON_DIFFUSE_INTENSITY,
        ] {
            for (channel, absorption) in UNDERWATER_LIGHT_ABSORPTION.iter().enumerate() {
                let value = read(base + channel * 4);
                assert!(
                    (value - absorption).abs() < 1e-6,
                    "direct light color at {base}+{channel} must be absorbed: {value} vs {absorption}"
                );
            }
            // The .a intensity lane is untouched.
            let a = read(base + 12);
            assert_eq!(a, 1.0, "intensity .a at {base}+12 must be preserved");
        }
        // The CB0 ambient cube and unrelated fields are untouched.
        for at in [
            offsets::AMBIENT_POS_X,
            offsets::AMBIENT_NEG_X,
            offsets::AMBIENT_POS_Y,
            offsets::AMBIENT_NEG_Y,
            offsets::AMBIENT_POS_Z,
            offsets::AMBIENT_NEG_Z,
            offsets::HDR_RANGE,
            offsets::CAM_POS,
            offsets::VIRTUAL_SUN_POS,
            offsets::DAY_NIGHT_HOUR_SUN_DIR,
            offsets::SUN_SPECULAR_INTENSITY,
            offsets::SHADOW_FADE_FACTOR,
        ] {
            assert_eq!(read(at), 1.0, "field at {at} must be preserved");
        }
    }

    #[test]
    fn underwater_light_absorption_fails_closed_on_short_cb0() {
        let mut short = vec![0u8; crate::light_law::offsets::CB0_LEN - 16];
        assert!(apply_underwater_light_absorption(&mut short).is_err());
    }

    // ----- WATER5 engine underwater CB0 decode (session_CAPFIX2_GAPS_4) -----

    /// Build a full 416-byte CB0 carrying the exact `SunDiffuseIntensity`,
    /// `MoonDiffuseIntensity`, and `vFoWOpacity_..._MaxGameSpeed.w` (day gate)
    /// measured from a real captured buffer.
    fn cb0_with_light(sun: [f32; 4], moon: [f32; 4], day_gate: f32) -> Vec<u8> {
        use crate::light_law::offsets;
        let mut cb0 = vec![0u8; offsets::CB0_LEN];
        let put4 = |cb0: &mut [u8], off: usize, v: [f32; 4]| {
            for (i, c) in v.iter().enumerate() {
                cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&c.to_le_bytes());
            }
        };
        put4(&mut cb0, offsets::SUN_DIFFUSE_INTENSITY, sun);
        put4(&mut cb0, offsets::MOON_DIFFUSE_INTENSITY, moon);
        cb0[offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED + 12
            ..offsets::FOW_OPACITY_TIME_SNOWMUD_MAXSPEED + 16]
            .copy_from_slice(&day_gate.to_le_bytes());
        cb0
    }

    /// The exact light fields decoded from the engine's OWN underwater pass CB0
    /// (`session_CAPFIX2_GAPS_4/underwater_frame_draw000_ps_constant_buffer_slot0.bin`,
    /// fnv1a64 `35a789eed695ca48`; draws 4-9 `dc539bf57791534e` carry the same
    /// light) — the first capture of the engine's underwater draw. See
    /// the exact content-admission path above.
    const CAPFIX2_UNDERWATER_SUN: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    const CAPFIX2_UNDERWATER_MOON: [f32; 4] = [0.217_637_63, 0.546_751_38, 1.0, 2.5];
    const CAPFIX2_UNDERWATER_DAY_GATE: f32 = 0.0;

    #[test]
    fn engine_underwater_cb0_carries_unabsorbed_white_sun() {
        // The engine's underwater pass binds the SAME scene light rig as terrain:
        // a WHITE sun (1,1,1) and the shipped moon color, with the day-night gate
        // OFF. This DISPROVES the WATER3 CB0-absorption hypothesis: the engine
        // applies no per-pass light-color absorption.
        let underwater = cb0_with_light(
            CAPFIX2_UNDERWATER_SUN,
            CAPFIX2_UNDERWATER_MOON,
            CAPFIX2_UNDERWATER_DAY_GATE,
        );
        // Same-frame scene (terrain) CB0 carries the identical light rig
        // (byte-diff of the real captures: only camera/time/fade lanes differ).
        let scene = cb0_with_light(
            CAPFIX2_UNDERWATER_SUN,
            CAPFIX2_UNDERWATER_MOON,
            CAPFIX2_UNDERWATER_DAY_GATE,
        );
        let delta = underwater_cb0_absorption_ratio(&underwater, &scene)
            .expect("both CB0s are full length");

        // The engine's underwater sun is white and un-absorbed.
        assert_eq!(delta.underwater_sun_rgb, [1.0, 1.0, 1.0]);
        // Day-night gate OFF (full daylight capture).
        assert_eq!(delta.underwater_day_gate, 0.0);
        // Every sun/moon channel ratio is exactly 1.0 — NO absorption.
        for r in delta
            .sun_rgb_ratio
            .iter()
            .chain(delta.moon_rgb_ratio.iter())
        {
            let v = r.expect("scene light channel is non-zero");
            assert!(
                (v - 1.0).abs() < 1e-6,
                "underwater/scene light ratio {v} must be 1.0 (no engine absorption)"
            );
        }
        assert!(
            delta.is_unabsorbed(1e-6),
            "engine underwater pass must carry an un-absorbed light rig"
        );
    }

    #[test]
    fn recovered_absorption_is_not_the_engine_cb0_ratio() {
        // Sanity guard on the WATER5 conclusion: IF the engine had applied the
        // recovered UNDERWATER_LIGHT_ABSORPTION as a CB0 ratio, an underwater CB0
        // built by absorbing a white-sun scene CB0 would show that ratio. It does
        // NOT match the captured underwater CB0 (which is un-absorbed, ratio 1),
        // so the constant is not the engine's CB0 light law.
        let scene = cb0_with_light([1.0, 1.0, 1.0, 1.0], CAPFIX2_UNDERWATER_MOON, 0.0);
        let mut hypothetical_absorbed = scene.clone();
        apply_underwater_light_absorption(&mut hypothetical_absorbed).unwrap();
        let hypo = underwater_cb0_absorption_ratio(&hypothetical_absorbed, &scene).unwrap();
        // The hypothetical (app-absorbed) CB0 reproduces the absorption ratio ...
        for (i, r) in hypo.sun_rgb_ratio.iter().enumerate() {
            let v = r.unwrap();
            assert!(
                (v - UNDERWATER_LIGHT_ABSORPTION[i]).abs() < 1e-6,
                "app-absorbed sun ratio {v} must equal the absorption {}",
                UNDERWATER_LIGHT_ABSORPTION[i]
            );
        }
        // ... but the REAL captured engine underwater CB0 does not (ratio 1.0).
        let real = cb0_with_light(CAPFIX2_UNDERWATER_SUN, CAPFIX2_UNDERWATER_MOON, 0.0);
        let real_delta = underwater_cb0_absorption_ratio(&real, &scene).unwrap();
        assert!(
            real_delta.is_unabsorbed(1e-6),
            "the captured engine underwater CB0 is un-absorbed, unlike the app's clone"
        );
        assert!(
            !hypo.is_unabsorbed(1e-6),
            "the app's absorbed clone is (correctly) detected as absorbed"
        );
    }

    #[test]
    fn underwater_cb0_absorption_ratio_fails_closed_on_short_cb0() {
        let short = vec![0u8; crate::light_law::offsets::CB0_LEN - 4];
        let full = vec![0u8; crate::light_law::offsets::CB0_LEN];
        assert!(underwater_cb0_absorption_ratio(&short, &full).is_none());
        assert!(underwater_cb0_absorption_ratio(&full, &short).is_none());
    }

    // ----- WATER3 LEAN specular (glint) overflow + taming law -----

    #[test]
    fn lean_specular_peak_is_unbounded_as_det_goes_to_zero() {
        // spec = 1/sqrt(det): the NORMALIZED-Gaussian NDF peak diverges as the
        // slope covariance collapses (glassy water). This is the files-derived
        // overflow input (pdxwater.shader:309). Each 100x drop in det raises the
        // peak 10x, without bound.
        let p_1 = lean_specular_peak(1.0);
        let p_1e2 = lean_specular_peak(1.0e-2);
        let p_1e6 = lean_specular_peak(1.0e-6);
        assert!((p_1 - 1.0).abs() < 1e-6);
        assert!((p_1e2 - 10.0).abs() < 1e-3, "peak at det=1e-2 was {p_1e2}");
        assert!(
            (p_1e6 - 1000.0).abs() < 1e-1,
            "peak at det=1e-6 was {p_1e6}"
        );
        assert!(
            p_1e6 > p_1e2 && p_1e2 > p_1,
            "peak must grow as det shrinks"
        );
        // The shader's (det <= 0) ? 0 guard: non-positive det yields exactly 0
        // (no exp/sqrt of a non-positive determinant).
        assert_eq!(lean_specular_peak(0.0), 0.0);
        assert_eq!(lean_specular_peak(-1.0), 0.0);
    }

    #[test]
    fn open_sea_specmap_collapses_glint_the_shader_implied_bound() {
        // The (1 - vSpecMap) factor (pdxwater.shader:322) is the shader's own
        // bound: where SpecularMap.a -> 1 (open sea) the glossiness -> 0 even
        // when the raw NDF peak is enormous. This is why the shipped pipeline
        // does not blow out over open ocean.
        let huge_peak_det = 1.0e-8; // spec peak ~1e4
        let gloss_open_sea = lean_specular_peak_glossiness(huge_peak_det, 1.0);
        assert_eq!(gloss_open_sea, 0.0, "vSpecMap=1 must collapse glossiness");
        // A near-1 SpecularMap.a still tames it by ~2 orders of magnitude vs
        // vSpecMap=0.
        let gloss_near_open = lean_specular_peak_glossiness(huge_peak_det, 0.99);
        let gloss_calm = lean_specular_peak_glossiness(huge_peak_det, 0.0);
        assert!(
            gloss_near_open < gloss_calm * 0.02,
            "near-open (a=0.99) gloss {gloss_near_open} must be <<2% of a=0 gloss {gloss_calm}",
        );
    }

    #[test]
    fn specmap_flatten_shrinks_lean_covariance_toward_flat() {
        // vFlatten = vSpecMap scales B and M *= vSpecMap^2 (pdxwater.shader:
        // 294-296), so a small SpecularMap.a drives the slope covariance det
        // toward the flat-water regime (all slopes ~0). Using a representative
        // LEAN sample, det at a=0.1 must be a small fraction of det at a=1.0.
        let b = [0.30_f32, -0.20];
        let m = [0.50_f32, 0.40, 0.05];
        let det_full = lean_flattened_det(b, m, 1.0);
        let det_flat = lean_flattened_det(b, m, 0.1);
        assert!(det_full > 0.0, "reference det must be positive: {det_full}");
        assert!(
            det_flat.abs() < det_full * 0.05,
            "flattened det {det_flat} must be <<5% of full det {det_full}",
        );
    }
}
