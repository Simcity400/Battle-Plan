

pub const STANDARDFUNCS_SOURCE: &str = "gfx/FX/standardfuncsgfx.fxh";
pub const CONSTANTS_FXH_SOURCE: &str = "gfx/FX/constants.fxh";

pub const STRIPE_ANGLE: f32 = 3.14159 / 8.0;
pub const SEC_MAP_TILE: f32 = 6000.0;
pub const STRIPE_CAM_MIN: f32 = 300.0;
pub const STRIPE_CAM_MAX: f32 = 1200.0;
pub const STRIPE_CAM_LIFT: f32 = 1.5;
pub const STRIPE_CONTRAST: f32 = 1.7;
pub const STRIPE_INTENSITY_NEAR: f32 = 1.0;
pub const STRIPE_INTENSITY_FAR: f32 = 0.3;


#[inline]
pub(crate) fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

pub fn cam_distance(cam_y: f32, v_min: f32, v_max: f32) -> f32 {
    (cam_y.clamp(v_min, v_max) - v_min) / (v_max - v_min)
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = saturate((x - edge0) / (edge1 - edge0));
    t * t * (3.0 - 2.0 * t)
}

pub fn occupation_stripe_mask(u: f32, v: f32, cam_y: f32) -> f32 {
    let grating =
        (u * STRIPE_ANGLE.cos() * SEC_MAP_TILE + v * STRIPE_ANGLE.sin() * SEC_MAP_TILE).cos();
    let cam_dist = cam_distance(cam_y, STRIPE_CAM_MIN, STRIPE_CAM_MAX);
    let lifted = grating + cam_dist * STRIPE_CAM_LIFT;
    let intensity =
        STRIPE_INTENSITY_NEAR + (STRIPE_INTENSITY_FAR - STRIPE_INTENSITY_NEAR) * cam_dist;
    smoothstep(0.0, 1.0, lifted * STRIPE_CONTRAST) * intensity
}

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
        assert!((STRIPE_ANGLE - 0.392_698_75).abs() < 1e-6);
        assert_eq!(SEC_MAP_TILE, 6000.0);
    }

    #[test]
    fn gate_cam_distance_clamped_ramp() {
        assert!((cam_distance(300.0, 300.0, 1200.0) - 0.0).abs() < EPS);
        assert!((cam_distance(1200.0, 300.0, 1200.0) - 1.0).abs() < EPS);
        assert!((cam_distance(750.0, 300.0, 1200.0) - 0.5).abs() < EPS);
        assert!((cam_distance(50.0, 300.0, 1200.0) - 0.0).abs() < EPS);
        assert!((cam_distance(3000.0, 300.0, 1200.0) - 1.0).abs() < EPS);
    }

    #[test]
    fn gate_stripe_crest_near_camera_is_full() {
        let m = occupation_stripe_mask(0.0, 0.0, STRIPE_CAM_MIN);
        assert!((m - 1.0).abs() < EPS, "crest near -> 1.0, got {m}");
    }

    #[test]
    fn gate_stripe_trough_near_camera_is_zero() {
        let u = std::f32::consts::PI / (STRIPE_ANGLE.cos() * SEC_MAP_TILE);
        let m = occupation_stripe_mask(u, 0.0, STRIPE_CAM_MIN);
        assert!((m - 0.0).abs() < 1e-4, "trough near -> 0.0, got {m}");
    }

    #[test]
    fn gate_stripe_far_camera_solid_at_reduced_intensity() {
        let crest = occupation_stripe_mask(0.0, 0.0, STRIPE_CAM_MAX);
        assert!((crest - 0.3).abs() < EPS, "far crest -> 0.3, got {crest}");
    }

    #[test]
    fn gate_secondary_blend_and_bloom_suppression() {
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
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_stripe_law_is_shipped() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
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
