//! Camera pan/zoom interpolation law (plan item **B6**/E2, fidelity ledger
//! **#17**).
//!
//! CAMDEF1 already landed the camera *clamps* and the wheel-zoom grounding from
//! the shipped defines; the open residual was the interpolation FUNCTION — the
//! easing shape the camera uses to approach a pan/zoom target. This module
//! closes that shape from the shipped defines.
//!
//! # Law (file-cited, `common/defines/00_graphics.lua`)
//!
//! - `CAMERA_INTERPOLATION_SPEED = 0.19` (`:1498`) — the per-tick exponential
//!   smoothing fraction. The engine eases the camera toward its target with
//!   `current = lerp(current, target, CAMERA_INTERPOLATION_SPEED)` each tick, so
//!   the remaining distance decays geometrically as `0.81^n`. This is an
//!   exponential ease-out (asymptotic approach), NOT a fixed-time or
//!   cubic-bezier curve.
//! - `CAMERA_SPEED_IN_MENUS = 0.1` (`:1497`) — the menu-scene variant.
//! - `CAMERA_MIN_HEIGHT = 50.0` / `CAMERA_MAX_HEIGHT = 3000.0`
//!   (`:1495-1496`) — the height clamp (re-cited from CAMDEF1).
//! - Zoom SPEED is distance-proportional: `CAMERA_ZOOM_SPEED = 50` (`:1187`),
//!   `CAMERA_ZOOM_KEY_SCALE = 0.02` (`:1188`),
//!   `CAMERA_ZOOM_SPEED_DISTANCE_MULT = 6.0` (`:1189`, "when camera is max
//!   zoomed out, the zooming in speed will get 100% of
//!   CAMERA_ZOOM_SPEED_DISTANCE_MULT zooming speed"). The zoom step scales with
//!   current height, giving a geometric (constant-perceptual-rate) zoom.
//!
//! # Classification
//!
//! FILE for the interpolation SHAPE (exponential ease-out, coefficient 0.19)
//! and the zoom distance-proportionality. The residual RUNTIME piece is the
//! frame-rate / fixed-tick normalization of the per-tick fraction (engine C++
//! `dt` handling) — that rescales the time-constant, not the curve family — and
//! the exact interpolation of the zoom distance multiplier. Those are the only
//! remaining #17 gaps.

pub const GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

/// Per-tick exponential-smoothing fraction (`00_graphics.lua:1498`).
pub const CAMERA_INTERPOLATION_SPEED: f32 = 0.19;
/// Menu-scene interpolation fraction (`:1497`).
pub const CAMERA_SPEED_IN_MENUS: f32 = 0.1;
/// Height clamp (`:1495-1496`).
pub const CAMERA_MIN_HEIGHT: f32 = 50.0;
pub const CAMERA_MAX_HEIGHT: f32 = 3000.0;
/// Zoom-speed defines (`:1187-1189`).
pub const CAMERA_ZOOM_SPEED: f32 = 50.0;
pub const CAMERA_ZOOM_KEY_SCALE: f32 = 0.02;
pub const CAMERA_ZOOM_SPEED_DISTANCE_MULT: f32 = 6.0;

/// One interpolation tick: ease `current` toward `target` by `speed`
/// (`current = lerp(current, target, speed)`).
pub fn camera_interp_step(current: f32, target: f32, speed: f32) -> f32 {
    current + (target - current) * speed
}

/// Fraction of the initial pan/zoom distance still remaining after `n` ticks of
/// exponential smoothing at `speed`: `(1 - speed)^n`.
pub fn interp_remaining_fraction(n: u32, speed: f32) -> f32 {
    (1.0 - speed).powi(n as i32)
}

/// Clamp a camera height to `[CAMERA_MIN_HEIGHT, CAMERA_MAX_HEIGHT]`.
pub fn clamp_height(h: f32) -> f32 {
    h.clamp(CAMERA_MIN_HEIGHT, CAMERA_MAX_HEIGHT)
}

/// Distance-proportional zoom step magnitude at a given normalized height
/// `t in [0,1]` (0 = fully zoomed in, 1 = fully zoomed out): the base speed
/// scaled toward `CAMERA_ZOOM_SPEED_DISTANCE_MULT` as the camera pulls out.
/// Shape only — the engine's exact `t` mapping is the runtime residual.
pub fn zoom_step_speed(height_t: f32) -> f32 {
    let t = height_t.clamp(0.0, 1.0);
    CAMERA_ZOOM_SPEED * (1.0 + (CAMERA_ZOOM_SPEED_DISTANCE_MULT - 1.0) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    #[test]
    fn gate_interp_step_eases_toward_target() {
        // From 100 toward 300 at 0.19: 100 + 200*0.19 = 138.
        let n = camera_interp_step(100.0, 300.0, CAMERA_INTERPOLATION_SPEED);
        assert!((n - 138.0).abs() < 1e-3, "expected 138, got {n}");
        // Fixed point at the target.
        assert!((camera_interp_step(300.0, 300.0, CAMERA_INTERPOLATION_SPEED) - 300.0).abs() < EPS);
    }

    #[test]
    fn gate_exponential_decay_shape() {
        // After 1 tick, 0.81 of the distance remains; the curve is (0.81)^n.
        assert!((interp_remaining_fraction(1, CAMERA_INTERPOLATION_SPEED) - 0.81).abs() < EPS);
        assert!((interp_remaining_fraction(2, CAMERA_INTERPOLATION_SPEED) - 0.6561).abs() < 1e-5);
        assert!((interp_remaining_fraction(0, CAMERA_INTERPOLATION_SPEED) - 1.0).abs() < EPS);
        // Monotonically decreasing, never negative (ease-out, no overshoot).
        let mut prev = 1.0;
        for k in 1..40 {
            let f = interp_remaining_fraction(k, CAMERA_INTERPOLATION_SPEED);
            assert!(f >= 0.0 && f < prev, "non-monotone at {k}");
            prev = f;
        }
    }

    #[test]
    fn gate_height_clamp() {
        assert_eq!(clamp_height(10.0), 50.0);
        assert_eq!(clamp_height(9000.0), 3000.0);
        assert_eq!(clamp_height(800.0), 800.0);
    }

    #[test]
    fn gate_zoom_is_distance_proportional() {
        // Fully zoomed in: base speed. Fully out: base * DISTANCE_MULT.
        assert!((zoom_step_speed(0.0) - CAMERA_ZOOM_SPEED).abs() < EPS);
        assert!(
            (zoom_step_speed(1.0) - CAMERA_ZOOM_SPEED * CAMERA_ZOOM_SPEED_DISTANCE_MULT).abs()
                < EPS
        );
        // Monotone increasing with height.
        assert!(zoom_step_speed(0.25) < zoom_step_speed(0.75));
    }

    #[test]
    fn gate_vanilla_camera_defines_are_shipped() {
        let Ok(game_dir) = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")) else {
            eprintln!("gate_vanilla_camera_defines_are_shipped: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };
        let resolver = crate::mod_resolver::GameFileResolver::vanilla(game_dir).expect("resolver");
        let defines =
            std::fs::read_to_string(resolver.resolve(GRAPHICS_DEFINES_SOURCE).unwrap().unwrap())
                .unwrap();
        assert!(
            defines.contains("CAMERA_INTERPOLATION_SPEED = 0.19"),
            "interpolation speed"
        );
        assert!(defines.contains("CAMERA_MIN_HEIGHT = 50.0"), "min height");
        assert!(defines.contains("CAMERA_MAX_HEIGHT = 3000.0"), "max height");
        assert!(defines.contains("CAMERA_ZOOM_SPEED = 50"), "zoom speed");
        assert!(
            defines.contains("CAMERA_ZOOM_SPEED_DISTANCE_MULT = 6.0"),
            "zoom distance mult"
        );
    }
}
