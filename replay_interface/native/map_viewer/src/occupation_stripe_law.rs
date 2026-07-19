//! Occupation / secondary-color diagonal-stripe render law (plan item **B6**,
//! fidelity ledger **#20**).
//!
//! The political map paints a diagonal striped overlay for the "secondary
//! color" layer (occupation / intel-network-target / diplomatic-secondary
//! shading). This module owns the *stripe generation law* — angle, spatial
//! period, camera-height fade, and the blend into the base fill — all derived
//! from the shipped shaders.
//!
//! # Law (all file-cited)
//!
//! `CalculateOccupationMask` (`gfx/FX/standardfuncsgfx.fxh:965-977`):
//! ```text
//! t         = 3.14159 / 8.0                       // diagonal angle = 22.5 deg
//! w         = SEC_MAP_TILE = 6000.0               // constants.fxh:216
//! stripeVal = cos(u*cos(t)*w + v*sin(t)*w)        // diagonal cosine grating
//! camDist   = cam_distance(300.0, 1200.0)         // standardfuncsgfx.fxh:184
//! stripeVal += camDist * 1.5                      // lift toward solid far out
//! stripeVal  = smoothstep(0,1, stripeVal*1.7) * lerp(1.0, 0.3, camDist)
//! ```
//! `secondary_color_mask` (`:979-987`):
//! ```text
//! occ   = CalculateOccupationMask(uv) * colorMask.a   // colorMask = secondary tex
//! bloom = bloom * (1 - occ)
//! color = lerp(color, colorMask.rgb, occ)
//! ```
//!
//! # Classification
//!
//! FILE for the entire stripe RENDER law (angle `pi/8`, period from
//! `SEC_MAP_TILE`, camera-distance fade, `smoothstep`/`lerp` intensity, and the
//! `lerp(base, secondary, occ)` blend that also suppresses bloom). The
//! province-id -> secondary-color-texture (`colorMask.rgb/.a`) MAPPING is the
//! data-fed occupier/secondary color: recording-side (owner != controller and
//! the specific secondary map mode), tracked as ledger #20 UNKNOWN and gated on
//! a capture where the secondary layer is non-zero (the vanilla political
//! secondary SRV is all-transparent, so the mapping validation is still
//! vacuous). This module closes the *shader law*, not the data mapping.

// ---- shipped shader source anchors ----------------------------------------

pub const STANDARDFUNCS_SOURCE: &str = "gfx/FX/standardfuncsgfx.fxh";
pub const CONSTANTS_FXH_SOURCE: &str = "gfx/FX/constants.fxh";

/// Diagonal grating angle `t = 3.14159 / 8` (`standardfuncsgfx.fxh:968`).
pub const STRIPE_ANGLE: f32 = 3.14159 / 8.0;
/// Spatial period scale `SEC_MAP_TILE` (`constants.fxh:216`). Larger => thinner
/// stripes (comment `standardfuncsgfx.fxh:969`).
pub const SEC_MAP_TILE: f32 = 6000.0;
/// Camera-distance fade range for the stripe (`standardfuncsgfx.fxh:972`).
pub const STRIPE_CAM_MIN: f32 = 300.0;
pub const STRIPE_CAM_MAX: f32 = 1200.0;
/// Far-out solid-lift added to the grating (`:973`, `camDist * 1.5`).
pub const STRIPE_CAM_LIFT: f32 = 1.5;
/// Contrast scale inside the `smoothstep` (`:975`, `stripeVal*1.7`).
pub const STRIPE_CONTRAST: f32 = 1.7;
/// Near/far stripe intensity endpoints (`:975`, `lerp(1.0, 0.3, camDist)`).
pub const STRIPE_INTENSITY_NEAR: f32 = 1.0;
pub const STRIPE_INTENSITY_FAR: f32 = 0.3;

#[inline]
fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// `cam_distance(vMin,vMax)` (`standardfuncsgfx.fxh:184-187`):
/// `(clamp(camY,vMin,vMax) - vMin) / (vMax - vMin)`.
pub fn cam_distance(cam_y: f32, v_min: f32, v_max: f32) -> f32 {
    (cam_y.clamp(v_min, v_max) - v_min) / (v_max - v_min)
}

/// HLSL `smoothstep(edge0, edge1, x)` (Hermite), used at `:975`.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = saturate((x - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

/// `CalculateOccupationMask(uv)` (`standardfuncsgfx.fxh:965-977`). `u`/`v` are
/// the province UVs; `cam_y` the camera height. Returns the stripe intensity in
/// `[0,1]` BEFORE the secondary-texture alpha gate.
pub fn occupation_stripe_mask(u: f32, v: f32, cam_y: f32) -> f32 {
    let grating =
        (u * STRIPE_ANGLE.cos() * SEC_MAP_TILE + v * STRIPE_ANGLE.sin() * SEC_MAP_TILE).cos();
    let cam_dist = cam_distance(cam_y, STRIPE_CAM_MIN, STRIPE_CAM_MAX);
    let lifted = grating + cam_dist * STRIPE_CAM_LIFT;
    let intensity =
        STRIPE_INTENSITY_NEAR + (STRIPE_INTENSITY_FAR - STRIPE_INTENSITY_NEAR) * cam_dist;
    smoothstep(0.0, 1.0, lifted * STRIPE_CONTRAST) * intensity
}

/// `secondary_color_mask` (`standardfuncsgfx.fxh:979-987`): blend the secondary
/// color over the base fill by the stripe mask gated by the secondary-texture
/// alpha, and suppress bloom under the stripe. Returns `(out_rgb, out_bloom)`.
pub fn apply_secondary_color(
    base_rgb: [f32; 3],
    mask_rgb: [f32; 3],
    mask_alpha: f32,
    u: f32,
    v: f32,
    cam_y: f32,
    bloom_alpha: f32,
) -> ([f32; 3], f32) {
    let occ = occupation_stripe_mask(u, v, cam_y) * mask_alpha;
    let out_bloom = bloom_alpha * (1.0 - occ);
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        out[i] = base_rgb[i] + (mask_rgb[i] - base_rgb[i]) * occ;
    }
    (out, out_bloom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_resolver::GameFileResolver;

    const EPS: f32 = 1e-6;

    #[test]
    fn gate_stripe_constants() {
        assert!((STRIPE_ANGLE - 0.392_698_75).abs() < 1e-6); // pi/8
        assert_eq!(SEC_MAP_TILE, 6000.0);
    }

    #[test]
    fn gate_cam_distance_clamped_ramp() {
        assert!((cam_distance(300.0, 300.0, 1200.0) - 0.0).abs() < EPS);
        assert!((cam_distance(1200.0, 300.0, 1200.0) - 1.0).abs() < EPS);
        assert!((cam_distance(750.0, 300.0, 1200.0) - 0.5).abs() < EPS);
        assert!((cam_distance(50.0, 300.0, 1200.0) - 0.0).abs() < EPS); // clamp low
        assert!((cam_distance(3000.0, 300.0, 1200.0) - 1.0).abs() < EPS); // clamp high
    }

    #[test]
    fn gate_stripe_crest_near_camera_is_full() {
        // uv origin -> grating = cos(0) = 1; near camera camDist=0 -> lifted=1;
        // smoothstep(0,1, 1.7)=1 (clamped) * lerp(1,0.3,0)=1 -> 1.0.
        let m = occupation_stripe_mask(0.0, 0.0, STRIPE_CAM_MIN);
        assert!((m - 1.0).abs() < EPS, "crest near -> 1.0, got {m}");
    }

    #[test]
    fn gate_stripe_trough_near_camera_is_zero() {
        // Choose u so the grating hits a trough (cos = -1): arg = pi.
        // u * cos(t) * 6000 = pi  =>  u = pi / (cos(t)*6000).
        let u = std::f32::consts::PI / (STRIPE_ANGLE.cos() * SEC_MAP_TILE);
        let m = occupation_stripe_mask(u, 0.0, STRIPE_CAM_MIN);
        assert!((m - 0.0).abs() < 1e-4, "trough near -> 0.0, got {m}");
    }

    #[test]
    fn gate_stripe_far_camera_solid_at_reduced_intensity() {
        // Far camera camDist=1: every grating value >= -1 lifts to >= 0.5, and
        // even a trough (-1 + 1.5 = 0.5) * 1.7 = 0.85 -> smoothstep ~ 0.94, so
        // the stripe goes nearly solid but at 0.3 intensity. Crest -> 0.3 exact.
        let crest = occupation_stripe_mask(0.0, 0.0, STRIPE_CAM_MAX);
        assert!((crest - 0.3).abs() < EPS, "far crest -> 0.3, got {crest}");
    }

    #[test]
    fn gate_secondary_blend_and_bloom_suppression() {
        // Full stripe (near crest) with secondary alpha 1: color becomes the
        // secondary color and bloom is fully suppressed.
        let (rgb, bloom) = apply_secondary_color(
            [0.2, 0.2, 0.2],
            [0.9, 0.1, 0.1],
            1.0,
            0.0,
            0.0,
            STRIPE_CAM_MIN,
            0.8,
        );
        assert!((rgb[0] - 0.9).abs() < EPS && (rgb[1] - 0.1).abs() < EPS);
        assert!((bloom - 0.0).abs() < EPS, "bloom suppressed, got {bloom}");

        // Zero secondary alpha: base color and bloom pass through untouched
        // (this is why vanilla's all-transparent secondary SRV shows no stripes).
        let (rgb2, bloom2) = apply_secondary_color(
            [0.2, 0.3, 0.4],
            [1.0, 1.0, 1.0],
            0.0,
            0.0,
            0.0,
            STRIPE_CAM_MIN,
            0.8,
        );
        assert_eq!(rgb2, [0.2, 0.3, 0.4]);
        assert!((bloom2 - 0.8).abs() < EPS);
    }

    #[test]
    fn gate_vanilla_stripe_law_is_shipped() {
        let Ok(game_dir) = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")) else {
            eprintln!("gate_vanilla_stripe_law_is_shipped: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let funcs =
            std::fs::read_to_string(resolver.resolve(STANDARDFUNCS_SOURCE).unwrap().unwrap())
                .unwrap();
        assert!(funcs.contains("float t = 3.14159 / 8.0;"), "stripe angle");
        assert!(funcs.contains("float w = SEC_MAP_TILE;"), "period scale");
        assert!(
            funcs.contains("cam_distance( 300.0, 1200.0 )"),
            "stripe cam range"
        );
        assert!(
            funcs.contains("smoothstep(0.0, 1.0, stripeVal*1.7) * lerp(1.0, 0.3, camDist)"),
            "stripe intensity"
        );
        let constants =
            std::fs::read_to_string(resolver.resolve(CONSTANTS_FXH_SOURCE).unwrap().unwrap())
                .unwrap();
        assert!(
            constants.contains("SEC_MAP_TILE = 6000.0f"),
            "SEC_MAP_TILE value"
        );
    }
}
