use crate::{GameUiError, Result};
use map_viewer::{EffectiveDefines, GameFileResolver};
use replay_core::ObserverDesignerFamily;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignerIntelThresholds {
    pub ship_raw: i64,
    pub tank_raw: i64,
    pub plane_raw: i64,
}

impl DesignerIntelThresholds {
    pub fn from_resolver(resolver: &GameFileResolver) -> Result<Self> {
        let defines = EffectiveDefines::from_resolver(resolver)
            .map_err(|error| GameUiError::missing(format!("XUI13 admitted defines: {error}")))?;
        Self::from_defines(&defines)
    }

    fn from_defines(defines: &EffectiveDefines) -> Result<Self> {
        let read = |key| {
            let value = defines.table_number("NDefines", "NIntel", key)
                .map_err(|error| GameUiError::missing(format!("XUI13 intel threshold: {error}")))?;
            lua_fixed(value)
        };
        Ok(Self {
            ship_raw: read("NAVY_SHIP_TYPE_COUNT_INTEL_MAX")?,
            tank_raw: read("ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX")?,
            plane_raw: read("AIR_WING_TYPE_COUNT_INTEL_MAX")?,
        })
    }

    pub fn admits_recorded_foreign_country(&self, frame: &replay_core::WorldFrame,
        catalog: &replay_core::design_catalog::NativeDesignerInputs, family: ObserverDesignerFamily,
        owner: &str, target: &str,
    ) -> std::result::Result<bool, replay_core::designer_intel::DesignerIntelRefusal> {
        frame.designer_intel_lane2(catalog, owner, target)
            .map(|raw| self.admits_foreign_country(family, raw))
    }

    pub fn admits_foreign_country(&self, family: ObserverDesignerFamily, lane2_raw: i64) -> bool {
        let raw = match family {
            ObserverDesignerFamily::Ship => self.ship_raw,
            ObserverDesignerFamily::Tank => self.tank_raw,
            ObserverDesignerFamily::Plane => self.plane_raw,
        };
        lane2_raw > raw.wrapping_mul(10_000_000) / 100_000
    }
}

fn lua_fixed(value: f64) -> Result<i64> {
    let value = value as f32;
    // 14205d560 narrows Lua numbers before the three ordered float operations.
    let scaled = ((if value >= 0.0 { 1.0f32 } else { 0.0 }) + value * 100_000.0f32) - 0.5f32;
    if !scaled.is_finite() || scaled < i64::MIN as f32 || scaled >= -(i64::MIN as f32) {
        return Err(GameUiError::missing("XUI13 intel threshold cannot be represented as engine fixed point"));
    }
    Ok(scaled.trunc() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_evaluate_effective_lua_and_use_count_defines() {
        let defines = EffectiveDefines::from_sources(&[
            ("common/defines/base.lua".into(), "NDefines = { NIntel = { NAVY_SHIP_TYPE_COUNT_INTEL_MAX = .7, ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX = .6, AIR_WING_TYPE_COUNT_INTEL_MAX = .5, NAVY_MIN_INTEL_TO_SHOW_SHIP_DESIGN_DETAILS = .99 } }".into()),
            ("common/defines/mod.lua".into(), "local i = NDefines.NIntel; i.ARMY_TEMPLATE_UNIT_COUNT_INTEL_MAX = i.AIR_WING_TYPE_COUNT_INTEL_MAX / 2".into()),
        ]).unwrap();
        let thresholds = DesignerIntelThresholds::from_defines(&defines).unwrap();
        assert_eq!(thresholds, DesignerIntelThresholds { ship_raw: 70_000, tank_raw: 25_000, plane_raw: 50_000 });
        assert!(!thresholds.admits_foreign_country(ObserverDesignerFamily::Ship, 7_000_000));
        assert!(thresholds.admits_foreign_country(ObserverDesignerFamily::Ship, 7_000_001));
        assert!(!thresholds.admits_foreign_country(ObserverDesignerFamily::Tank, 2_500_000));
        assert!(thresholds.admits_foreign_country(ObserverDesignerFamily::Tank, 2_500_001));
    }

    #[test]
    fn fixed_conversion_preserves_sign_zero_and_rejects_nonfinite_or_overflow() {
        assert_eq!(lua_fixed(0.0).unwrap(), 0);
        assert_eq!(lua_fixed(-0.0).unwrap(), 0);
        assert_eq!(lua_fixed(0.000005).unwrap(), 1);
        assert_eq!(lua_fixed(-0.000005).unwrap(), -1);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            assert!(lua_fixed(value).is_err());
        }
    }

    #[test]
    fn missing_or_invalid_authored_threshold_is_not_defaulted() {
        for value in ["nil", "{}", "'0.7'"] {
            let defines = EffectiveDefines::from_sources(&[("common/defines/mod.lua".into(), format!("NDefines = {{ NIntel = {{ NAVY_SHIP_TYPE_COUNT_INTEL_MAX = {value} }} }}"))]).unwrap();
            assert!(DesignerIntelThresholds::from_defines(&defines).is_err());
        }
    }
    #[test]
    fn signed_threshold_comparison_is_strict_and_wraps_as_engine_i64() {
        let thresholds = DesignerIntelThresholds { ship_raw: -50_000, tank_raw: i64::MAX, plane_raw: 0 };
        assert!(!thresholds.admits_foreign_country(ObserverDesignerFamily::Ship, -5_000_000));
        assert!(thresholds.admits_foreign_country(ObserverDesignerFamily::Ship, -4_999_999));
        assert!(!thresholds.admits_foreign_country(ObserverDesignerFamily::Plane, i64::MIN));
        assert!(thresholds.admits_foreign_country(ObserverDesignerFamily::Plane, i64::MAX));
        assert!(!thresholds.admits_foreign_country(ObserverDesignerFamily::Tank, -100));
        assert!(thresholds.admits_foreign_country(ObserverDesignerFamily::Tank, -99));
    }

}
