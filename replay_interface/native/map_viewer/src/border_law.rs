//! BORD2 — the border render law (files-first, capture-validated).
//!
//! This module owns the *camera-height fade law* the game thin-border pass
//! consumes, plus the class → shipped-texture and class → fade-window mappings.
//! It is the analytic core of the `--borders-from-files` render: the GPU pass
//! ([`crate::border_pass`]) uploads the synthesized border mesh once and, per
//! frame, feeds each border class the transparency this module computes for the
//! current camera height into the shipped border pixel shader's global alpha
//! multiplier.
//!
//! # Where the law comes from (shipped files, source of truth)
//!
//! ## `gfx/FX/border.shader` (the pixel shader's alpha)
//!
//! The shipped border PS returns (border.shader `PixelShader` MainCode):
//!
//! ```hlsl
//! return float4( vColor.rgb, max( vColor.a, vPulseFactor - 0.2f ) * vTime_Transparency.y );
//! ```
//!
//! `vTime_Transparency` is `ConstantBuffer( 2, 48 ){ float4 vSelectionColor;
//! float2 vTime_Transparency; }`. Component `.x` is time (drives the selection
//! pulse `sin( vTime_Transparency.x * 2.5f )`); component `.y` is a GLOBAL alpha
//! MULTIPLIER applied to the whole border fragment. The shader itself does NOT
//! compute a camera-height fade — it consumes `vTime_Transparency.y` as an
//! already-computed transparency. The engine's C++ sets that value per border
//! draw from the camera height and the fade defines; that CPU computation is
//! what this module reconstructs (see the tracked gap below).
//!
//! ## `common/defines/00_graphics.lua` (the fade windows)
//!
//! Only two border fade windows are defined in the shipped file:
//!
//! * `PROVINCE_BORDER_FADE_NEAR = 200`, `PROVINCE_BORDER_FADE_FAR = 300`
//! * `STATE_BORDER_FADE_NEAR = 400`,   `STATE_BORDER_FADE_FAR = 500`
//!
//! There is NO `COUNTRY_BORDER_FADE_*`, no `IMPASSABLE_BORDER_FADE_*`, and no
//! coast/sea-region fade define anywhere in `00_graphics.lua` (verified by
//! grepping every `FADE` line). Class → window mapping (see
//! [`BorderFadeWindows::window_for`] and the [`BorderFadeWindow`] docs):
//!
//! * `province` → province window (200–300);
//! * `state` → state window (400–500);
//! * `impassable` → state window (400–500), because the synthesis ranks
//!   impassable as `State`; the missing `IMPASSABLE_BORDER_FADE_*` define means
//!   only that impassable has no SEPARATE window, not that it is opaque
//!   (tracked approximation);
//! * `country`, `sea_coast`, `sea_region` → no window, fully opaque
//!   (`transparency = 1.0`) at every camera height.
//!
//! The camera-height clamp these windows live inside is `CAMERA_MIN_HEIGHT = 50`,
//! `CAMERA_MAX_HEIGHT = 3000` (`00_defines.lua`), so the province window
//! (200–300) and state window (400–500) both sit in the lower/zoomed-in portion
//! of the range.
//!
//! # Fade direction
//!
//! Fine borders are visible when zoomed IN (low camera height) and disappear
//! when zoomed OUT (high camera height): in the game the province grid vanishes as
//! you pull the camera back, then the state grid, leaving only country borders.
//! So transparency is `1.0` for `height <= NEAR`, falls to `0.0` at
//! `height >= FAR`, and interpolates in between.
//!
//! # Tracked gap — the ramp SHAPE is an approximation
//!
//! The two fade windows `[NEAR, FAR]` are files-derived and exact, but the
//! interpolation curve the engine applies BETWEEN the endpoints is NOT present
//! in any shipped shader or define — it lives in the engine's C++ map-render
//! code. [`border_transparency`] implements a **linear** ramp over `[NEAR, FAR]`
//! as the documented default. This is an APPROXIMATION: the true engine ramp
//! could be a `smoothstep` (the border PS itself uses `smoothstep` for the
//! selection pulse) or another easing. The endpoints (`1.0` at/below `NEAR`,
//! `0.0` at/above `FAR`) are certain; only the mid-window values are the
//! approximation, and only a live `borders_pass` capture that records
//! `vTime_Transparency.y` at a known camera height can settle the exact curve.
//! The ramp is deliberately isolated in the single function
//! [`border_transparency`] so that swap is a one-line change once that evidence
//! exists.

use crate::{
    parse_lua_number_define, read_latin1, BorderMaterial, GameFileResolver, Result,
    BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
};

/// Shipped `00_graphics.lua` border fade defines (byte-for-byte from the file).
///
/// Kept as constants so a validation test fails loudly if a mod or a game patch
/// changes them. The runtime renderer parses the LIVE file through the resolver
/// ([`border_fade_windows_from_graphics_defines`]); these mirror the vanilla
/// values used to derive and unit-test the law.
pub mod defines {
    /// `PROVINCE_BORDER_FADE_NEAR` — camera height at/below which province
    /// borders are fully opaque.
    pub const PROVINCE_BORDER_FADE_NEAR: f32 = 200.0;
    /// `PROVINCE_BORDER_FADE_FAR` — camera height at/above which province
    /// borders are fully faded out.
    pub const PROVINCE_BORDER_FADE_FAR: f32 = 300.0;
    /// `STATE_BORDER_FADE_NEAR` — camera height at/below which state borders are
    /// fully opaque.
    pub const STATE_BORDER_FADE_NEAR: f32 = 400.0;
    /// `STATE_BORDER_FADE_FAR` — camera height at/above which state borders are
    /// fully faded out.
    pub const STATE_BORDER_FADE_FAR: f32 = 500.0;
    /// `CAMERA_MIN_HEIGHT` (`00_defines.lua`) — floor of the camera-height range
    /// the fade windows live inside.
    pub const CAMERA_MIN_HEIGHT: f32 = 50.0;
    /// `CAMERA_MAX_HEIGHT` (`00_defines.lua`) — ceiling of the camera-height
    /// range.
    pub const CAMERA_MAX_HEIGHT: f32 = 3000.0;
}

/// The `00_graphics.lua` keys the runtime renderer reads for the two fade
/// windows, in `(near_key, far_key)` order per window.
pub const PROVINCE_FADE_DEFINE_KEYS: (&str, &str) =
    ("PROVINCE_BORDER_FADE_NEAR", "PROVINCE_BORDER_FADE_FAR");
/// State-window define keys, `(near, far)`.
pub const STATE_FADE_DEFINE_KEYS: (&str, &str) =
    ("STATE_BORDER_FADE_NEAR", "STATE_BORDER_FADE_FAR");

/// The camera-height fade window `[near, far]` for a border material, or `None`
/// when the material has no fade define and stays opaque at every height.
///
/// Files-derived mapping (see the module docs): the two shipped windows are
/// `province` and `state`; `province` uses the province window, `state` and
/// `impassable` use the state window (see below), and `country`, `sea_coast`
/// and `sea_region` have no fade define and stay opaque.
///
/// # Impassable (documented approximation — tracked)
///
/// The synthesis classifies impassable edges with `rank = State`
/// (`border_world_synthesis.rs` `classify_border_edge`, both the adjacency
/// override and the wasteland heuristic). There is no `IMPASSABLE_BORDER_FADE_*`
/// define, which proves only that impassable has no SEPARATE window — not that
/// it is opaque. Because the synthesis already ranks impassable as State, the
/// defensible files-first default is to fade impassable on the STATE window
/// (400/500), i.e. it fades in lockstep with the state grid it shares a rank
/// with. The alternative — leaving impassable permanently opaque, matching the
/// in-game read that mountain/wasteland edges never visibly fade — is the other
/// plausible reading. Only a live `borders_pass` capture that records
/// `vTime_Transparency.y` for an impassable draw at a known camera height can
/// settle which is correct; this module takes the state-window reading to stay
/// consistent with the synthesis rank and documents the open question.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderFadeWindow {
    pub near: f32,
    pub far: f32,
}

/// Runtime fade windows, read from the live `00_graphics.lua` through the
/// resolver so a mod's overridden values are honoured. The province and state
/// windows are the only two the file defines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderFadeWindows {
    pub province: BorderFadeWindow,
    pub state: BorderFadeWindow,
}

impl BorderFadeWindows {
    /// The vanilla windows from [`defines`]. Used as the unit-test oracle and as
    /// the value a caller compares the live-parsed windows against.
    pub fn vanilla() -> Self {
        Self {
            province: BorderFadeWindow {
                near: defines::PROVINCE_BORDER_FADE_NEAR,
                far: defines::PROVINCE_BORDER_FADE_FAR,
            },
            state: BorderFadeWindow {
                near: defines::STATE_BORDER_FADE_NEAR,
                far: defines::STATE_BORDER_FADE_FAR,
            },
        }
    }

    /// The fade window for a border material, or `None` if the material has no
    /// fade define (renders opaque at every camera height).
    ///
    /// `Impassable` maps to the STATE window because the synthesis ranks it as
    /// `State` (see the [`BorderFadeWindow`] docs for the tracked open question).
    /// `Country`, `SeaCoast` and `SeaRegion` have no shipped fade define and stay
    /// opaque.
    pub fn window_for(&self, material: BorderMaterial) -> Option<BorderFadeWindow> {
        match material {
            BorderMaterial::Province => Some(self.province),
            BorderMaterial::State | BorderMaterial::Impassable => Some(self.state),
            BorderMaterial::Country | BorderMaterial::SeaCoast | BorderMaterial::SeaRegion => None,
        }
    }
}

/// Read the two border fade windows from the LIVE `common/defines/00_graphics.lua`
/// through the resolver, so a mod that overrides them is honoured (files-first
/// mandate). Fails closed if any of the four keys is missing or non-positive, or
/// if a window is inverted (`far <= near`) — the shipped file is well-formed, so
/// a violation is a real data problem, not something to paper over.
pub fn border_fade_windows_from_resolver(resolver: &GameFileResolver) -> Result<BorderFadeWindows> {
    let path = resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let text = read_latin1(&path)?;
    border_fade_windows_from_graphics_defines(&text)
}

/// Parse the four fade defines out of an already-loaded `00_graphics.lua` body.
/// Split from [`border_fade_windows_from_resolver`] so the parse is unit-testable
/// without a resolver.
pub fn border_fade_windows_from_graphics_defines(text: &str) -> Result<BorderFadeWindows> {
    let province = read_fade_window(text, PROVINCE_FADE_DEFINE_KEYS)?;
    let state = read_fade_window(text, STATE_FADE_DEFINE_KEYS)?;
    Ok(BorderFadeWindows { province, state })
}

fn read_fade_window(text: &str, keys: (&str, &str)) -> Result<BorderFadeWindow> {
    let (near_key, far_key) = keys;
    let near = parse_lua_number_define(text, near_key)?;
    let far = parse_lua_number_define(text, far_key)?;
    if !near.is_finite() || !far.is_finite() || near < 0.0 {
        return Err(format!(
            "border fade law blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} {near_key}/{far_key} has \
             invalid value near={near} far={far}"
        )
        .into());
    }
    if far <= near {
        return Err(format!(
            "border fade law blocked: {BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} {far_key}={far} must exceed \
             {near_key}={near}"
        )
        .into());
    }
    Ok(BorderFadeWindow { near, far })
}

/// THE ramp function (see module docs — the ONE place the fade curve lives).
///
/// Returns the border pixel shader's global alpha multiplier
/// (`vTime_Transparency.y`) for a material at a given camera height:
///
/// * materials with no fade window ([`BorderFadeWindows::window_for`] `== None`)
///   return `1.0` (opaque) at every height;
/// * for a material with window `[near, far]`: `1.0` for `height <= near`,
///   `0.0` for `height >= far`, and a **linear** interpolation `1 -> 0` for
///   `near < height < far`.
///
/// The linear interpolation between the endpoints is the documented
/// approximation (module docs, tracked gap): swap this single expression when a
/// live capture reveals the engine's true easing. `far > near` is guaranteed by
/// the shipped defines; if a mod supplied a degenerate `far <= near` window the
/// ramp collapses to a hard step at `near` (still monotone, still bounded).
pub fn border_transparency(
    windows: &BorderFadeWindows,
    material: BorderMaterial,
    camera_height: f32,
) -> f32 {
    let Some(window) = windows.window_for(material) else {
        return 1.0;
    };
    if !camera_height.is_finite() {
        // A non-finite height cannot be placed on the ramp; fail visible rather
        // than emit NaN into the constant buffer.
        return 1.0;
    }
    if camera_height <= window.near {
        return 1.0;
    }
    if camera_height >= window.far {
        return 0.0;
    }
    let span = window.far - window.near;
    if span <= 0.0 {
        // Degenerate mod window: hard step at `near` (height is already > near
        // here, so it is fully faded).
        return 0.0;
    }
    // Linear ramp 1 -> 0 across (near, far). APPROXIMATION — see module docs.
    (window.far - camera_height) / span
}

/// The shipped `map/terrain/border_<class>_0.dds` file the pass binds as
/// `BorderDiffuse` for a material.
///
/// # Files-first source of truth (tracked)
///
/// The one `borders_pass` capture (`diag23b`) is PROVEN contaminated: its
/// recorded 32×32 `R8G8B8A8_UNORM` runtime `BorderDiffuse` is ALL-ZERO and its
/// pre/post render targets are byte-identical (the draw was a no-op over a
/// mis-recorded terrain grid — see `border_capture_evidence.rs` and
/// BORDER_LAW.md). It therefore does NOT reveal the runtime border texture. The
/// shipped per-class DDS files (64×32…256×128, mixed DXT5 / uncompressed) are the
/// appearance ground truth: each class samples its own authored border stripe,
/// whose decoded alpha cross-section and colour are the law in
/// [`crate::border_texture_law`]. Whether the engine composites these into one
/// runtime atlas at draw time is the only open piece, pending a clean capture.
pub fn border_diffuse_source(material: BorderMaterial) -> &'static str {
    match material {
        BorderMaterial::Province => "map/terrain/border_province_0.dds",
        BorderMaterial::State => "map/terrain/border_state_0.dds",
        BorderMaterial::Country => "map/terrain/border_country_0.dds",
        BorderMaterial::SeaCoast => "map/terrain/border_sea_0.dds",
        BorderMaterial::SeaRegion => "map/terrain/border_sea_region_0.dds",
        BorderMaterial::Impassable => "map/terrain/border_impassable_0.dds",
    }
}

/// Draw order of the border pass relative to the other production-map passes,
/// derived from the one captured session that recorded a `borders_pass`
/// (`diag23b/map_capture_session.json`): the pass sequence is
/// `terrain → water → rivers → borders → trees → cities → overlays`. The border
/// pass therefore draws AFTER water and rivers and BEFORE trees, over the
/// terrain color/depth targets. This constant documents the matched order; the
/// renderer enforces it by placing the synthesized border pass at the same slot
/// in [`crate::NativeD3d11PreparedProductionMap::dependent_passes_in_order`].
pub const BORDER_PASS_DRAW_ORDER_EVIDENCE: &str =
    "diag23b/map_capture_session.json: terrain,water,rivers,borders,trees,cities,overlays";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_windows_match_shipped_defines() {
        let w = BorderFadeWindows::vanilla();
        assert_eq!(w.province.near, 200.0);
        assert_eq!(w.province.far, 300.0);
        assert_eq!(w.state.near, 400.0);
        assert_eq!(w.state.far, 500.0);
    }

    #[test]
    fn province_ramp_endpoints_and_midpoint() {
        let w = BorderFadeWindows::vanilla();
        let m = BorderMaterial::Province;
        // Fully opaque at/below NEAR (200), including well below.
        assert_eq!(border_transparency(&w, m, 50.0), 1.0);
        assert_eq!(border_transparency(&w, m, 200.0), 1.0);
        // Fully faded at/above FAR (300), including well above.
        assert_eq!(border_transparency(&w, m, 300.0), 0.0);
        assert_eq!(border_transparency(&w, m, 3000.0), 0.0);
        // Linear midpoint: height 250 -> 0.5.
        let mid = border_transparency(&w, m, 250.0);
        assert!((mid - 0.5).abs() < 1e-6, "province midpoint {mid}");
        // Quarter points confirm linearity.
        assert!((border_transparency(&w, m, 225.0) - 0.75).abs() < 1e-6);
        assert!((border_transparency(&w, m, 275.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn state_ramp_endpoints_and_midpoint() {
        let w = BorderFadeWindows::vanilla();
        let m = BorderMaterial::State;
        assert_eq!(border_transparency(&w, m, 400.0), 1.0);
        assert_eq!(border_transparency(&w, m, 500.0), 0.0);
        let mid = border_transparency(&w, m, 450.0);
        assert!((mid - 0.5).abs() < 1e-6, "state midpoint {mid}");
        // At the province FAR (300) a state border is still fully opaque: the
        // two windows are staggered, which is what produces the "province grid
        // vanishes first, then the state grid" zoom-out behaviour.
        assert_eq!(border_transparency(&w, m, 300.0), 1.0);
    }

    #[test]
    fn opaque_classes_never_fade() {
        let w = BorderFadeWindows::vanilla();
        for material in [
            BorderMaterial::Country,
            BorderMaterial::SeaCoast,
            BorderMaterial::SeaRegion,
        ] {
            assert!(
                w.window_for(material).is_none(),
                "{material:?} has no window"
            );
            for height in [50.0, 250.0, 450.0, 600.0, 3000.0] {
                assert_eq!(
                    border_transparency(&w, material, height),
                    1.0,
                    "{material:?} must stay opaque at height {height}"
                );
            }
        }
    }

    #[test]
    fn impassable_fades_on_the_state_window() {
        // Impassable is rank=State in the synthesis, so it shares the state fade
        // window (400/500) rather than staying opaque. (Tracked approximation.)
        let w = BorderFadeWindows::vanilla();
        let m = BorderMaterial::Impassable;
        assert_eq!(w.window_for(m), Some(w.state));
        assert_eq!(border_transparency(&w, m, 400.0), 1.0);
        assert_eq!(border_transparency(&w, m, 500.0), 0.0);
        // Same curve as state at the midpoint.
        assert_eq!(
            border_transparency(&w, m, 450.0),
            border_transparency(&w, BorderMaterial::State, 450.0)
        );
        // Still fully opaque at the province FAR (300), like state.
        assert_eq!(border_transparency(&w, m, 300.0), 1.0);
    }

    #[test]
    fn ramp_is_monotone_non_increasing_in_height() {
        let w = BorderFadeWindows::vanilla();
        for material in [BorderMaterial::Province, BorderMaterial::State] {
            let mut prev = border_transparency(&w, material, 0.0);
            let mut height = 0.0;
            while height <= 3000.0 {
                let t = border_transparency(&w, material, height);
                assert!(
                    t <= prev + 1e-6,
                    "{material:?} transparency rose from {prev} to {t} at height {height}"
                );
                assert!((0.0..=1.0).contains(&t), "{material:?} out of range: {t}");
                prev = t;
                height += 5.0;
            }
        }
    }

    #[test]
    fn non_finite_height_fails_visible_not_nan() {
        let w = BorderFadeWindows::vanilla();
        let t = border_transparency(&w, BorderMaterial::Province, f32::NAN);
        assert_eq!(t, 1.0);
        assert!(t.is_finite());
    }

    #[test]
    fn three_validation_heights_bracket_the_windows() {
        // The three dump heights the validation harness renders must exercise
        // all three regimes the spec asks for. This pins the intent in a test.
        let w = BorderFadeWindows::vanilla();
        // All three dump heights sit inside the shipped camera-height clamp
        // [CAMERA_MIN_HEIGHT, CAMERA_MAX_HEIGHT] and above the province window,
        // i.e. they are reachable camera positions in-game.
        for h in [150.0f32, 350.0, 600.0] {
            assert!(h >= defines::CAMERA_MIN_HEIGHT && h <= defines::CAMERA_MAX_HEIGHT);
        }
        assert!(defines::CAMERA_MIN_HEIGHT < defines::PROVINCE_BORDER_FADE_NEAR);
        assert!(defines::STATE_BORDER_FADE_FAR < defines::CAMERA_MAX_HEIGHT);
        // close (150): province+state+country all fully visible.
        assert_eq!(
            border_transparency(&w, BorderMaterial::Province, 150.0),
            1.0
        );
        assert_eq!(border_transparency(&w, BorderMaterial::State, 150.0), 1.0);
        assert_eq!(border_transparency(&w, BorderMaterial::Country, 150.0), 1.0);
        // mid (350): province fully faded, state fading (still opaque here since
        // state NEAR is 400), country opaque.
        assert_eq!(
            border_transparency(&w, BorderMaterial::Province, 350.0),
            0.0
        );
        assert_eq!(border_transparency(&w, BorderMaterial::State, 350.0), 1.0);
        // A height inside the state window shows partial state fade.
        let state_fading = border_transparency(&w, BorderMaterial::State, 450.0);
        assert!(state_fading > 0.0 && state_fading < 1.0);
        // far (600): province and state fully faded, only country/opaque remain.
        assert_eq!(
            border_transparency(&w, BorderMaterial::Province, 600.0),
            0.0
        );
        assert_eq!(border_transparency(&w, BorderMaterial::State, 600.0), 0.0);
        assert_eq!(border_transparency(&w, BorderMaterial::Country, 600.0), 1.0);
    }

    #[test]
    fn fade_windows_parse_from_graphics_defines_body() {
        // A minimal 00_graphics.lua-shaped body with the four fade keys.
        let body = "NDefines_Graphics = {\n NGraphics = {\n  \
             PROVINCE_BORDER_FADE_NEAR = 200,\n  PROVINCE_BORDER_FADE_FAR = 300,\n  \
             STATE_BORDER_FADE_NEAR = 400,\n  STATE_BORDER_FADE_FAR = 500,\n }\n}\n";
        let windows = border_fade_windows_from_graphics_defines(body).expect("parse windows");
        assert_eq!(windows, BorderFadeWindows::vanilla());
    }

    #[test]
    fn inverted_fade_window_fails_closed() {
        let body = "PROVINCE_BORDER_FADE_NEAR = 300,\nPROVINCE_BORDER_FADE_FAR = 200,\n\
             STATE_BORDER_FADE_NEAR = 400,\nSTATE_BORDER_FADE_FAR = 500,\n";
        let err = border_fade_windows_from_graphics_defines(body)
            .expect_err("far <= near must fail closed");
        assert!(
            err.to_string().contains("must exceed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn missing_fade_define_fails_closed() {
        let body = "PROVINCE_BORDER_FADE_NEAR = 200,\nPROVINCE_BORDER_FADE_FAR = 300,\n";
        // State keys absent -> parse_lua_number_define errors.
        assert!(border_fade_windows_from_graphics_defines(body).is_err());
    }

    #[test]
    fn diffuse_source_is_a_shipped_border_dds_per_class() {
        for material in [
            BorderMaterial::Province,
            BorderMaterial::State,
            BorderMaterial::Country,
            BorderMaterial::SeaCoast,
            BorderMaterial::SeaRegion,
            BorderMaterial::Impassable,
        ] {
            let src = border_diffuse_source(material);
            assert!(src.starts_with("map/terrain/border_"));
            assert!(src.ends_with("_0.dds"));
        }
    }
}
