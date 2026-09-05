
use crate::{BorderMaterial, GameFileResolver, Result};

pub mod defines {
    pub const PROVINCE_BORDER_FADE_NEAR: f32 = 200.0;
    pub const PROVINCE_BORDER_FADE_FAR: f32 = 300.0;
    pub const STATE_BORDER_FADE_NEAR: f32 = 400.0;
    pub const STATE_BORDER_FADE_FAR: f32 = 500.0;
    pub const CAMERA_MIN_HEIGHT: f32 = 50.0;
    pub const CAMERA_MAX_HEIGHT: f32 = 3000.0;
}

pub const PROVINCE_FADE_DEFINE_KEYS: (&str, &str) =
    ("PROVINCE_BORDER_FADE_NEAR", "PROVINCE_BORDER_FADE_FAR");
pub const STATE_FADE_DEFINE_KEYS: (&str, &str) =
    ("STATE_BORDER_FADE_NEAR", "STATE_BORDER_FADE_FAR");

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderFadeWindow {
    pub near: f32,
    pub far: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderFadeWindows {
    pub province: BorderFadeWindow,
    pub state: BorderFadeWindow,
}

impl BorderFadeWindows {
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

    pub fn window_for(&self, material: BorderMaterial) -> Option<BorderFadeWindow> {
        match material {
            BorderMaterial::Province => Some(self.province),
            BorderMaterial::State => Some(self.state),
            BorderMaterial::Country
            | BorderMaterial::Impassable
            | BorderMaterial::SeaCoast
            | BorderMaterial::SeaRegion => None,
        }
    }
}

pub fn border_fade_windows_from_resolver(resolver: &GameFileResolver) -> Result<BorderFadeWindows> {
    let (sources, _) = crate::xmap09_lighting_law::effective_define_sources(resolver, "effective defines")?;
    let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;
    let window = |keys: (&str, &str)| -> Result<BorderFadeWindow> {
        let near = evaluated.scalar("NGraphics", keys.0)?;
        let far = evaluated.scalar("NGraphics", keys.1)?;
        if near < 0.0 || far <= near { return Err("invalid border fade window".into()); }
        Ok(BorderFadeWindow { near, far })
    };
    Ok(BorderFadeWindows { province: window(PROVINCE_FADE_DEFINE_KEYS)?, state: window(STATE_FADE_DEFINE_KEYS)? })
}

pub fn border_transparency(
    windows: &BorderFadeWindows,
    material: BorderMaterial,
    camera_height: f32,
) -> Result<f32> {
    if !camera_height.is_finite() {
        return Err(format!(
            "border fade law blocked: camera height {camera_height} is not finite; the \
             {material:?} fade cannot be placed on the ramp and an opaque fallback is never \
             substituted"
        )
        .into());
    }
    let Some(window) = windows.window_for(material) else {
        return Ok(1.0);
    };
    let span = window.far - window.near;
    if span <= 0.0 {
        return Ok(if camera_height <= window.near {
            1.0
        } else {
            0.0
        });
    }
    let clamped_height = camera_height.clamp(window.near, window.far);
    Ok(1.0 - (clamped_height - window.near) / span)
}

pub const fn border_diffuse_source(material: BorderMaterial) -> &'static str {
    match material {
        BorderMaterial::Province => "map/terrain/border_province_0.dds",
        BorderMaterial::State => "map/terrain/border_state_0.dds",
        BorderMaterial::Country => "map/terrain/border_country_0.dds",
        BorderMaterial::SeaCoast => "map/terrain/border_sea_0.dds",
        BorderMaterial::SeaRegion => "map/terrain/border_sea_region_0.dds",
        BorderMaterial::Impassable => "map/terrain/border_impassable_0.dds",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(windows: &BorderFadeWindows, material: BorderMaterial, height: f32) -> f32 {
        border_transparency(windows, material, height).expect("finite height is on the ramp")
    }

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
        assert_eq!(ramp(&w, m, 50.0), 1.0);
        assert_eq!(ramp(&w, m, 200.0), 1.0);
        assert_eq!(ramp(&w, m, 300.0), 0.0);
        assert_eq!(ramp(&w, m, 3000.0), 0.0);
        let mid = ramp(&w, m, 250.0);
        assert!((mid - 0.5).abs() < 1e-6, "province midpoint {mid}");
        assert!((ramp(&w, m, 225.0) - 0.75).abs() < 1e-6);
        assert!((ramp(&w, m, 275.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn state_ramp_endpoints_and_midpoint() {
        let w = BorderFadeWindows::vanilla();
        let m = BorderMaterial::State;
        assert_eq!(ramp(&w, m, 400.0), 1.0);
        assert_eq!(ramp(&w, m, 500.0), 0.0);
        let mid = ramp(&w, m, 450.0);
        assert!((mid - 0.5).abs() < 1e-6, "state midpoint {mid}");
        assert_eq!(ramp(&w, m, 300.0), 1.0);
    }

    #[test]
    fn opaque_classes_never_fade() {
        let w = BorderFadeWindows::vanilla();
        for material in [
            BorderMaterial::Country,
            BorderMaterial::Impassable,
            BorderMaterial::SeaCoast,
            BorderMaterial::SeaRegion,
        ] {
            assert!(
                w.window_for(material).is_none(),
                "{material:?} has no window"
            );
            for height in [50.0, 250.0, 450.0, 600.0, 3000.0] {
                assert_eq!(
                    ramp(&w, material, height),
                    1.0,
                    "{material:?} must stay opaque at height {height}"
                );
            }
        }
    }

    #[test]
    fn impassable_is_opaque_independent_of_synthesis_rank() {
        let w = BorderFadeWindows::vanilla();
        let m = BorderMaterial::Impassable;
        assert_eq!(w.window_for(m), None);
        assert_eq!(ramp(&w, m, 400.0), 1.0);
        assert_eq!(ramp(&w, m, 500.0), 1.0);
        assert_eq!(ramp(&w, m, 3000.0), 1.0);
    }

    #[test]
    fn ramp_is_monotone_non_increasing_in_height() {
        let w = BorderFadeWindows::vanilla();
        for material in [BorderMaterial::Province, BorderMaterial::State] {
            let mut prev = ramp(&w, material, 0.0);
            let mut height = 0.0;
            while height <= 3000.0 {
                let t = ramp(&w, material, height);
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
    fn non_finite_height_is_a_typed_refusal() {
        let w = BorderFadeWindows::vanilla();
        for height in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for material in [
                BorderMaterial::Province,
                BorderMaterial::State,
                BorderMaterial::Country,
            ] {
                let err = border_transparency(&w, material, height)
                    .expect_err("non-finite height must refuse");
                let message = err.to_string();
                assert!(
                    message.contains("border fade law blocked") && message.contains("not finite"),
                    "unexpected error for {material:?} at {height}: {message}"
                );
            }
        }
    }

    #[test]
    fn three_validation_heights_bracket_the_windows() {
        let w = BorderFadeWindows::vanilla();
        for h in [150.0f32, 350.0, 600.0] {
            assert!(h >= defines::CAMERA_MIN_HEIGHT && h <= defines::CAMERA_MAX_HEIGHT);
        }
        assert!(defines::CAMERA_MIN_HEIGHT < defines::PROVINCE_BORDER_FADE_NEAR);
        assert!(defines::STATE_BORDER_FADE_FAR < defines::CAMERA_MAX_HEIGHT);
        assert_eq!(ramp(&w, BorderMaterial::Province, 150.0), 1.0);
        assert_eq!(ramp(&w, BorderMaterial::State, 150.0), 1.0);
        assert_eq!(ramp(&w, BorderMaterial::Country, 150.0), 1.0);
        assert_eq!(ramp(&w, BorderMaterial::Province, 350.0), 0.0);
        assert_eq!(ramp(&w, BorderMaterial::State, 350.0), 1.0);
        let state_fading = ramp(&w, BorderMaterial::State, 450.0);
        assert!(state_fading > 0.0 && state_fading < 1.0);
        assert_eq!(ramp(&w, BorderMaterial::Province, 600.0), 0.0);
        assert_eq!(ramp(&w, BorderMaterial::State, 600.0), 0.0);
        assert_eq!(ramp(&w, BorderMaterial::Country, 600.0), 1.0);
    }

    #[test]
    fn fade_windows_parse_from_graphics_defines_body() {
        let body = "NDefines = {\n NGraphics = {\n  \
             PROVINCE_BORDER_FADE_NEAR = 200,\n  PROVINCE_BORDER_FADE_FAR = 300,\n  \
             STATE_BORDER_FADE_NEAR = 400,\n  STATE_BORDER_FADE_FAR = 500,\n }\n}\n";
        let windows = crate::test_support::with_define_fixture(body, border_fade_windows_from_resolver).expect("evaluate windows");
        assert_eq!(windows, BorderFadeWindows::vanilla());
    }

    #[test]
    fn inverted_fade_window_fails_closed() {
        let body = "PROVINCE_BORDER_FADE_NEAR = 300,\nPROVINCE_BORDER_FADE_FAR = 200,\n\
             STATE_BORDER_FADE_NEAR = 400,\nSTATE_BORDER_FADE_FAR = 500,\n";
        let err = crate::test_support::with_define_fixture(
            &format!("NDefines = {{ NGraphics = {{ {body} }} }}"), border_fade_windows_from_resolver,
        ).expect_err("far <= near must fail closed");
        assert!(
            err.to_string().contains("invalid border fade window"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn missing_fade_define_fails_closed() {
        let body = "PROVINCE_BORDER_FADE_NEAR = 200,\nPROVINCE_BORDER_FADE_FAR = 300,\n";
        assert!(crate::test_support::with_define_fixture(
            &format!("NDefines = {{ NGraphics = {{ {body} }} }}"), border_fade_windows_from_resolver,
        ).is_err());
    }

    #[test]
    fn diffuse_source_is_a_shipped_border_dds_per_class() {
        for material in BorderMaterial::ALL {
            let src = border_diffuse_source(material);
            assert!(src.starts_with("map/terrain/border_"));
            assert!(src.ends_with("_0.dds"));
        }
    }
}
