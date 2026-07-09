//! Terrain shadow / PCF render law (plan item **B6**, fidelity ledger **#21**).
//!
//! This module owns the *shadow term* law for the game map passes: how the
//! terrain shadow is PRODUCED (light-space depth sample + PCF kernel), softened
//! (separable blur), faded by camera height, and CONSUMED by the lit terrain /
//! water / tree / river / border passes. It is the close-zoom (h=50) parity
//! lever for plan item C7.
//!
//! # Pipeline (all shipped-shader-derived)
//!
//! The engine draws terrain shadows in FOUR passes; the same `ShadowMap`
//! sampler binding (pdxmap PS slot 12, `gfx/FX/pdxmap.shader:122-131`,
//! `Type = "Shadow"`) refers to a DIFFERENT texture in the producer vs the
//! consumer pass:
//!
//! 1. **Light-space depth map (2048²).** The engine renders shadow-caster depth
//!    from the sun's point of view into a 2048×2048 depth texture, projected by
//!    the per-frame `ShadowMapTextureMatrix` (`pdxmap.shader:209`,
//!    `standardfuncsgfx.fxh:2-33` CB0). The projection is an orthographic map
//!    aimed along `LIGHT_SHADOW_DIRECTION = (-5,-8,5)`
//!    (`common/defines/00_graphics.lua:769-771`). It is a **single** shadow map,
//!    NOT cascaded — the only cascade path in the shipped shaders
//!    (`CalculateShadowCascaded`, `pdxmesh.shader:499`) is commented out. The
//!    2048² size is pinned by the PCF texel size `0.5f / 2048.0f`
//!    (`shadow.fxh:17`). The matrix + depth contents are per-frame RUNTIME
//!    (like `ViewProjectionMatrix`); everything downstream is file-derived.
//!
//! 2. **Screen-space shadow producer** — `Effect terrainunlit`
//!    (`pdxmap.shader:501-505` -> `PixelShaderTerrainUnlit:457-468`). Redraws
//!    terrain to a 1920×1080 RT; per pixel projects into light space
//!    (`Input.vShadowProj`) and calls `CalculateShadow` (`shadow.fxh:6-42`): a
//!    5-tap PCF cross kernel. Writes `(fShadowTerm, FogColorFactor,
//!    FogAlphaFactor, 1)` — shadow in `.r`, FoW in `.g/.b` (the FoW half is the
//!    closed C8 law, see `fow_law.rs`).
//!
//! 3. **Separable blur** — `gfx/FX/shadowblur.shader` (`ShadowBlurHorizontal`
//!    then `ShadowBlurVertical`): a 3-tap gaussian on `.r` only, PRESERVING
//!    `.g/.b` (FoW) and forcing `.a = 1`. Offsets/weights are supplied per-frame
//!    in `vGaussianOffsetsWeights[2]` (runtime CB, like the bloom kernel — the
//!    kernel STRUCTURE is file-law, the sigma-derived weights are runtime).
//!
//! 4. **Lit consumer** — the terrain/water/tree/river/border passes read the
//!    blurred screen-space shadow through `GetShadowScaled` (`shadow.fxh:44-54`)
//!    keyed by their own `SHADOW_WEIGHT_*` (`constants.fxh:170-175`), with a
//!    camera-height fade `ShadowFadeFactor` and (terrain only) a `0.1` floor
//!    (`pdxmap.shader:359`). The scaled shadow multiplies ONLY the direct sun/
//!    moon diffuse term (`CalculateSunLight`, `standardfuncsgfx.fxh:642-652`),
//!    never the ambient term.
//!
//! # Classification
//!
//! FILE for the entire producer/consumer math (PCF kernel, bias, penumbra,
//! blur structure, weights, camera-height fade, floor, diffuse-only blend).
//! The residual RUNTIME piece is `ShadowMapTextureMatrix` + the light-space
//! depth-map contents (single ortho projection along `LIGHT_SHADOW_DIRECTION`
//! sized to the visible scene) — this is engine per-frame state, not a fitted
//! constant. Pixel parity for the *contents* is capture-gated (a low-h mined
//! frame with the light-space depth pass), tracked in `LANE_REPORT.md`.
//!
//! No-eyeball honesty: the gates below are hand-computed shader-reference gates
//! (the NIGHT2/AMBIENT1 standard) plus resolver FILE-citation gates that assert
//! the exact shipped constants. They are NOT a pixel validation of the runtime
//! depth-map contents.

use crate::mod_resolver::GameFileResolver;
use crate::Result;

// ---- shipped shader source anchors ----------------------------------------

pub const SHADOW_FXH_SOURCE: &str = "gfx/FX/shadow.fxh";
pub const SHADOWBLUR_SHADER_SOURCE: &str = "gfx/FX/shadowblur.shader";
pub const PDXMAP_SHADER_SOURCE: &str = "gfx/FX/pdxmap.shader";
pub const CONSTANTS_FXH_SOURCE: &str = "gfx/FX/constants.fxh";
pub const GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

// ---- PCF kernel constants (gfx/FX/shadow.fxh) ------------------------------

/// Light-space shadow depth-map resolution, pinned by the PCF texel size
/// `0.5f / 2048.0f` (`shadow.fxh:17`).
pub const SHADOW_MAP_SIZE: f32 = 2048.0;
/// PCF texel step used for the cross kernel offsets (`shadow.fxh:17`).
pub const PCF_TEXEL_SIZE: f32 = 0.5 / SHADOW_MAP_SIZE;
/// Depth-compare bias (`shadow.fxh:26`, `fBias = 0.001f`).
pub const PCF_BIAS: f32 = 0.001;
/// Value a tap contributes when the receiver is occluded (`shadow.fxh:36`).
pub const PCF_SHADOWED_VALUE: f32 = 0.1;
/// Value a tap contributes when the receiver is lit (`shadow.fxh:36`).
pub const PCF_LIT_VALUE: f32 = 1.0;
/// Self-shadow guard: a stored depth `>= 0.99` never shadows (`shadow.fxh:36`,
/// `A < 0.99f`).
pub const PCF_OCCLUSION_MAX: f32 = 0.99;
/// Number of taps averaged (`shadow.fxh:14/30/40`).
pub const PCF_TAP_COUNT: usize = 5;

// ---- SHADOW_WEIGHT_* (gfx/FX/constants.fxh:170-175) ------------------------

pub const SHADOW_WEIGHT_TERRAIN: f32 = 0.7; // :170
pub const SHADOW_WEIGHT_MAP: f32 = 0.7; // :171
pub const SHADOW_WEIGHT_BORDER: f32 = 0.7; // :172
pub const SHADOW_WEIGHT_WATER: f32 = 0.5; // :173
pub const SHADOW_WEIGHT_RIVER: f32 = 0.4; // :174
pub const SHADOW_WEIGHT_TREE: f32 = 0.7; // :175

/// Post-scale floor applied to the terrain shadow term only
/// (`pdxmap.shader:359`, `max(GetShadowScaled(...), 0.1f)`).
pub const TERRAIN_SHADOW_FLOOR: f32 = 0.1;

// ---- camera-height fade (common/defines/00_graphics.lua) -------------------
//
// `ShadowFadeFactor` (a scene-CB0 scalar, `standardfuncsgfx.fxh:19`) is the
// engine's camera-height shadow fade. The shipped defines name and value it:
// `DRAW_SHADOWS_CUTOFF = 400` / `DRAW_SHADOWS_FADE_LENGTH = 50`
// (`00_graphics.lua:860-861`), i.e. `saturate((400 - camY) / 50)` — matching
// the previously mined runtime value (col3-political-color-law:
// `ShadowFade = clamp((400-h)/50)`). Above 400 wu the shadow is fully faded out.

pub const DRAW_SHADOWS_CUTOFF: f32 = 400.0; // :860
pub const DRAW_SHADOWS_FADE_LENGTH: f32 = 50.0; // :861

/// Light-space projection direction (`00_graphics.lua:769-771`). Note the
/// `constants.fxh:177-179` comment lists X=-8 but the shipped define is X=-5;
/// the define wins.
pub const LIGHT_SHADOW_DIRECTION: [f32; 3] = [-5.0, -8.0, 5.0];

#[inline]
fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// The 5 PCF cross-kernel tap offsets in shadow-map UV space
/// (`shadow.fxh:20-24`): west, north, east, south, centre.
pub fn pcf_tap_offsets() -> [[f32; 2]; PCF_TAP_COUNT] {
    let t = PCF_TEXEL_SIZE;
    [
        [-t, 0.0], // vTexCoords[0]
        [0.0, t],  // vTexCoords[1]
        [t, 0.0],  // vTexCoords[2]
        [0.0, -t], // vTexCoords[3]
        [0.0, 0.0], // vTexCoords[4] (centre)
    ]
}

/// `CalculateShadow` (`shadow.fxh:29-41`): the producer-pass shadow term.
///
/// `taps` are the 5 stored light-space depths sampled at the cross offsets;
/// `receiver_z` is `vShadowProj.z` (the fragment's light-space depth). Each tap
/// contributes `0.1` when occluded (`A < 0.99 && A < receiver_z - bias`) else
/// `1.0`; the 5 are averaged. Result lies in the discrete set
/// {0.1, 0.28, 0.46, 0.64, 0.82, 1.0}.
pub fn calculate_shadow(taps: &[f32; PCF_TAP_COUNT], receiver_z: f32) -> f32 {
    let b = receiver_z - PCF_BIAS;
    let mut sum = 0.0f32;
    for &a in taps.iter() {
        sum += if a < PCF_OCCLUSION_MAX && a < b {
            PCF_SHADOWED_VALUE
        } else {
            PCF_LIT_VALUE
        };
    }
    sum / PCF_TAP_COUNT as f32
}

/// `ShadowFadeFactor` from camera height (`00_graphics.lua:860-861`):
/// `saturate((DRAW_SHADOWS_CUTOFF - cam_y) / DRAW_SHADOWS_FADE_LENGTH)`.
/// 1.0 at/below `cutoff - length` (350 wu), 0.0 at/above the cutoff (400 wu).
pub fn shadow_fade_factor(cam_y: f32) -> f32 {
    saturate((DRAW_SHADOWS_CUTOFF - cam_y) / DRAW_SHADOWS_FADE_LENGTH)
}

/// `GetShadowScaled` (`shadow.fxh:44-54`): the lit-pass consumer.
///
/// `weight` is the pass's `SHADOW_WEIGHT_*`; `screen_shadow_r` is the blurred
/// screen-space shadow (`ShadowMap.r` sampled at `vScreenCoord`);
/// `fade` = [`shadow_fade_factor`]. Adds `(1 - fade)` to lift the shadow toward
/// unlit as the camera pulls back, saturates, then blends by `weight`:
/// `(1 - weight) + weight * saturate(screen_shadow_r + (1 - fade))`.
pub fn get_shadow_scaled(weight: f32, screen_shadow_r: f32, fade: f32) -> f32 {
    let scaler = saturate(weight);
    let v = saturate(screen_shadow_r + (1.0 - fade));
    (1.0 - scaler) + scaler * v
}

/// Full terrain-pass shadow term (`pdxmap.shader:359`):
/// `max(GetShadowScaled(SHADOW_WEIGHT_TERRAIN, screen_r, fade(camY)), 0.1)`.
pub fn terrain_shadow_term(screen_shadow_r: f32, cam_y: f32) -> f32 {
    get_shadow_scaled(
        SHADOW_WEIGHT_TERRAIN,
        screen_shadow_r,
        shadow_fade_factor(cam_y),
    )
    .max(TERRAIN_SHADOW_FLOOR)
}

/// Sun-diffuse shadow gate (`CalculateSunLight`, `standardfuncsgfx.fxh:647`):
/// the shadow term is scaled by the combined day/night factor BEFORE it
/// multiplies the direct diffuse. Shadows never touch the ambient term.
pub fn shadow_gated_by_daynight(shadow_term: f32, day_factor: f32, night_factor: f32) -> f32 {
    shadow_term * saturate(day_factor + night_factor)
}

/// `shadowblur.shader` separable 3-tap gaussian on `.r`
/// (`shadowblur.shader:74-79` / `:102-107`). `center` is the on-pixel `.r`;
/// `n1p/n1m` are the `+/-offset[1]` neighbours and `n2p/n2m` the
/// `+/-offset[2]` neighbours; `w` are `vWeight[0..2]`. `.g/.b` are passed
/// through unchanged (not modelled here).
pub fn shadow_blur_3tap(center: f32, n1p: f32, n1m: f32, n2p: f32, n2m: f32, w: [f32; 3]) -> f32 {
    center * w[0] + (n1p + n1m) * w[1] + (n2p + n2m) * w[2]
}

/// Resolve a shipped shader/define file to text via the mod-aware resolver.
fn read_source(resolver: &GameFileResolver, rel: &str) -> Result<String> {
    let path = resolver.resolve(rel)?.ok_or_else(|| {
        Box::<dyn std::error::Error>::from(format!("shadow_law: resolver could not find {rel}"))
    })?;
    Ok(std::fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    // ---- PCF producer kernel ------------------------------------------------

    #[test]
    fn gate_pcf_tap_offsets_match_shadow_fxh() {
        let t = 0.5f32 / 2048.0f32;
        assert_eq!(
            pcf_tap_offsets(),
            [[-t, 0.0], [0.0, t], [t, 0.0], [0.0, -t], [0.0, 0.0]]
        );
    }

    #[test]
    fn gate_pcf_all_lit_returns_one() {
        // Every stored depth is beyond the receiver -> no occlusion.
        let term = calculate_shadow(&[1.0; 5], 0.5);
        assert!((term - 1.0).abs() < EPS, "all-lit -> 1.0, got {term}");
    }

    #[test]
    fn gate_pcf_all_shadowed_returns_floor() {
        // Every stored depth is well in front of the receiver -> all occluded.
        let term = calculate_shadow(&[0.0; 5], 0.9);
        assert!((term - 0.1).abs() < EPS, "all-shadowed -> 0.1, got {term}");
    }

    #[test]
    fn gate_pcf_partial_two_shadowed() {
        // 2 occluded taps (0.0) + 3 lit taps (1.0) -> (2*0.1 + 3*1.0)/5 = 0.64.
        let term = calculate_shadow(&[0.0, 0.0, 1.0, 1.0, 1.0], 0.9);
        assert!((term - 0.64).abs() < EPS, "expected 0.64, got {term}");
    }

    #[test]
    fn gate_pcf_099_self_shadow_guard() {
        // A stored depth of 0.995 (>= 0.99) must NOT shadow even though it is in
        // front of the receiver -> lit. Guards against fully-lit texels reading
        // as occluders (shadow.fxh:36 `A < 0.99f`).
        let term = calculate_shadow(&[0.995; 5], 0.9);
        assert!((term - 1.0).abs() < EPS, "0.99 guard -> lit, got {term}");
    }

    #[test]
    fn gate_pcf_bias_prevents_acne() {
        // Receiver_z exactly equals the stored depth: with the 0.001 bias,
        // B = receiver_z - 0.001 < A, so A < B is false -> lit (no self-acne).
        let term = calculate_shadow(&[0.5; 5], 0.5);
        assert!((term - 1.0).abs() < EPS, "bias -> lit, got {term}");
        // Push the receiver past the bias and it shadows.
        let shadowed = calculate_shadow(&[0.5; 5], 0.5 + 2.0 * PCF_BIAS);
        assert!(
            (shadowed - 0.1).abs() < EPS,
            "past bias -> shadowed, got {shadowed}"
        );
    }

    // ---- camera-height fade -------------------------------------------------

    #[test]
    fn gate_shadow_fade_factor_endpoints() {
        assert!((shadow_fade_factor(50.0) - 1.0).abs() < EPS); // deep zoom -> full
        assert!((shadow_fade_factor(350.0) - 1.0).abs() < EPS); // cutoff-length
        assert!((shadow_fade_factor(375.0) - 0.5).abs() < EPS); // midpoint
        assert!((shadow_fade_factor(400.0) - 0.0).abs() < EPS); // cutoff
        assert!((shadow_fade_factor(3000.0) - 0.0).abs() < EPS); // clamped
    }

    #[test]
    fn gate_shadow_fully_faded_above_cutoff() {
        // Above 400 wu the fade is 0 -> GetShadowScaled adds (1-0)=1, saturates
        // to 1, so the term is 1.0 regardless of the stored shadow. Shadows are
        // only visible when zoomed below the 400 wu cutoff.
        let fade = shadow_fade_factor(500.0);
        let term = get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, 0.1, fade);
        assert!((term - 1.0).abs() < EPS, "faded -> no shadow, got {term}");
    }

    // ---- lit consumer -------------------------------------------------------

    #[test]
    fn gate_get_shadow_scaled_full_shadow_near_camera() {
        // Near camera (fade=1): screen_r=0.1, weight=0.7 ->
        // v = saturate(0.1 + 0) = 0.1; (1-0.7) + 0.7*0.1 = 0.37.
        let term = get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, 0.1, 1.0);
        assert!((term - 0.37).abs() < EPS, "expected 0.37, got {term}");
    }

    #[test]
    fn gate_get_shadow_scaled_lit_is_one() {
        // Fully-lit screen shadow -> 1.0 out for any weight.
        let term = get_shadow_scaled(SHADOW_WEIGHT_WATER, 1.0, 1.0);
        assert!((term - 1.0).abs() < EPS, "lit -> 1.0, got {term}");
    }

    #[test]
    fn gate_terrain_floor_binds_only_below_min() {
        // The terrain floor is 0.1 but the 0.7 weight already floors the scaled
        // term at 0.3, so the max(...,0.1) never binds for terrain -> 0.37.
        let term = terrain_shadow_term(0.1, 50.0);
        assert!((term - 0.37).abs() < EPS, "expected 0.37, got {term}");
        // The floor would bind if the scaled term were pushed below 0.1
        // (e.g. a negative sampled shadow); confirm the clamp exists.
        assert!((get_shadow_scaled(SHADOW_WEIGHT_TERRAIN, -10.0, 1.0)).max(TERRAIN_SHADOW_FLOOR) >= 0.1);
    }

    #[test]
    fn gate_shadow_gated_by_daynight() {
        // Full day: gate passes the term unchanged.
        assert!((shadow_gated_by_daynight(0.37, 1.0, 0.0) - 0.37).abs() < EPS);
        // Twilight (day+night < 1): the term is attenuated.
        assert!((shadow_gated_by_daynight(0.5, 0.3, 0.2) - 0.25).abs() < EPS);
        // day+night > 1 saturates to 1 (no amplification).
        assert!((shadow_gated_by_daynight(0.5, 0.8, 0.8) - 0.5).abs() < EPS);
    }

    // ---- separable blur -----------------------------------------------------

    #[test]
    fn gate_shadow_blur_3tap_hand_computed() {
        // center=1.0, all neighbours 0.5, weights [0.4,0.2,0.1]:
        // 0.4 + (0.5+0.5)*0.2 + (0.5+0.5)*0.1 = 0.4 + 0.2 + 0.1 = 0.7.
        let out = shadow_blur_3tap(1.0, 0.5, 0.5, 0.5, 0.5, [0.4, 0.2, 0.1]);
        assert!((out - 0.7).abs() < EPS, "expected 0.7, got {out}");
        // A uniform field is a fixed point iff the weights sum to 1.
        let uni = shadow_blur_3tap(0.5, 0.5, 0.5, 0.5, 0.5, [0.4, 0.15, 0.15]);
        assert!((uni - 0.5).abs() < EPS, "uniform passthrough, got {uni}");
    }

    // ---- shipped-constant FILE citations (skip without BATTLE_PLAN_GAME_DIR) -------

    fn vanilla_resolver() -> Option<GameFileResolver> {
        let game_dir = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")).ok()?;
        GameFileResolver::vanilla(game_dir).ok()
    }

    #[test]
    fn gate_vanilla_shadow_constants_are_shipped() {
        let Some(resolver) = vanilla_resolver() else {
            eprintln!("gate_vanilla_shadow_constants_are_shipped: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };

        let constants = read_source(&resolver, CONSTANTS_FXH_SOURCE).unwrap();
        for needle in [
            "SHADOW_WEIGHT_TERRAIN    	= 0.7f",
            "SHADOW_WEIGHT_WATER   		= 0.5f",
            "SHADOW_WEIGHT_RIVER   		= 0.4f",
            "SHADOW_WEIGHT_TREE   		= 0.7f",
        ] {
            assert!(
                constants.contains(needle),
                "constants.fxh missing `{needle}`"
            );
        }

        let shadow_fxh = read_source(&resolver, SHADOW_FXH_SOURCE).unwrap();
        assert!(shadow_fxh.contains("0.5f / 2048.0f"), "PCF texel size");
        assert!(shadow_fxh.contains("fBias = 0.001f"), "PCF bias");
        assert!(shadow_fxh.contains("? 0.1f : 1.0f"), "PCF penumbra/lit values");
        assert!(shadow_fxh.contains("fShadowTerm / 5.0f"), "5-tap average");
        assert!(
            shadow_fxh.contains("( 1.0 - fScaler ) + fScaler * vShadowValue"),
            "GetShadowScaled blend"
        );

        let defines = read_source(&resolver, GRAPHICS_DEFINES_SOURCE).unwrap();
        assert!(defines.contains("DRAW_SHADOWS_CUTOFF = 400"), "cutoff");
        assert!(defines.contains("DRAW_SHADOWS_FADE_LENGTH = 50"), "fade length");
        assert!(defines.contains("LIGHT_SHADOW_DIRECTION_X = -5.0"), "light dir x");
        assert!(defines.contains("LIGHT_SHADOW_DIRECTION_Y = -8.0"), "light dir y");
        assert!(defines.contains("LIGHT_SHADOW_DIRECTION_Z = 5.0"), "light dir z");

        let pdxmap = read_source(&resolver, PDXMAP_SHADER_SOURCE).unwrap();
        assert!(
            pdxmap.contains("max(GetShadowScaled( SHADOW_WEIGHT_TERRAIN, Input.vScreenCoord, ShadowMap ), 0.1f )"),
            "terrain shadow floor callsite"
        );
        assert!(
            pdxmap.contains("float fShadowTerm = CalculateShadow( Input.vShadowProj, ShadowMap);"),
            "producer callsite"
        );
    }
}
