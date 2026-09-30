//! Authored naval-combat presentation operands for the pinned game build.
//!
//! These values are loaded from `NDefines` by the game, then read by the
//! naval-combat window. They are not properties of a combat, country, recorder
//! viewpoint, or local render cache. The caller must supply the recording's
//! matching content layers. Missing or unsupported Lua is unavailable rather
//! than silently replaced with the shipped game's defaults.

use crate::{ResolvedLayers, defines::Defines as NumericDefines};
use std::io;

/// The authored operands the viewer's ports of the native positioning
/// (141609460), screening (14160A7B0/14160C430), carrier-air (14160C940) and
/// anti-air (141608FF0) binders read, in those binders' order. The names are
/// from each global's pinned define-loader xref. A define joins these lists
/// only with its consumer: every read is mandatory, so an unconsumed define
/// that failed to parse would make all of these binders unavailable.
const SUMMARY_FIXED_KEYS: [(&str, &str); 14] = [
    ("NNavy", "BASE_POSITIONING"),
    ("NNavy", "RELATIVE_SURFACE_DETECTION_TO_POSITIONING_FACTOR"),
    ("NNavy", "MAX_POSITIONING_BONUS_FROM_SURFACE_DETECTION"),
    ("NNavy", "HIGHER_SHIP_RATIO_POSITIONING_PENALTY_FACTOR"),
    ("NNavy", "MAX_POSITIONING_PENALTY_FROM_HIGHER_SHIP_RATIO"),
    ("NNavy", "HIGHER_CARRIER_RATIO_POSITIONING_PENALTY_FACTOR"),
    ("NNavy", "MAX_CARRIER_RATIO_POSITIONING_PENALTY_FACTOR"),
    (
        "NNavy",
        "SCREENING_EFFICIENCY_PENALTY_ON_MINIMUM_POSITIONING",
    ),
    ("NNavy", "SCREEN_RATIO_FOR_FULL_SCREENING_FOR_CAPITALS"),
    ("NNavy", "SCREEN_RATIO_FOR_FULL_SCREENING_FOR_CONVOYS"),
    ("NNavy", "CAPITAL_RATIO_FOR_FULL_SCREENING_FOR_CARRIERS"),
    ("NNavy", "CAPITAL_RATIO_FOR_FULL_SCREENING_FOR_CONVOYS"),
    ("NAir", "CARRIER_PLANES_AMOUNT_FOR_POSITIONING"),
    ("NNavy", "SHIP_TO_FLEET_ANTI_AIR_RATIO"),
];

const SUMMARY_INTEGER_KEYS: [(&str, &str); 3] = [
    ("NNavy", "MIN_SHIPS_FOR_HIGHER_SHIP_RATIO_PENALTY"),
    ("NNavy", "COMBAT_INITIAL_DURATION"),
    ("NNavy", "NAVAL_COMBAT_PLANE_MIN_STACKING_PENALTY"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    pub summary_fixed: [i64; 14],
    pub summary_integer: [i32; 3],
    /// Native `CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO`, indexed by the live
    /// task force's mission/stance value. A missing index remains unavailable.
    pub sortie_stance_ratios: Vec<i64>,
    pub base_carrier_sortie_efficiency: i64,
    pub selected_sortie_initial_time: i32,
}

impl Defines {
    pub fn mission_factor(&self, mission: i32) -> Option<i64> {
        usize::try_from(mission)
            .ok()
            .and_then(|index| self.sortie_stance_ratios.get(index).copied())
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Defines> {
    from_numbers(&crate::defines::read(layers)?)
}

pub fn from_numbers(numbers: &NumericDefines) -> io::Result<Defines> {
    let mut summary_fixed = [0; 14];
    for (index, (out, (section, key))) in
        summary_fixed.iter_mut().zip(SUMMARY_FIXED_KEYS).enumerate()
    {
        // 14205D560 converts the number; each define registry helper applies
        // its own lower bound after conversion.
        *out = native_summary_fixed(index, numbers.fixed5(section, key)?);
    }
    let mut summary_integer = [0; 3];
    for (index, (out, (section, key))) in summary_integer
        .iter_mut()
        .zip(SUMMARY_INTEGER_KEYS)
        .enumerate()
    {
        // The integer loader is 14205B690 -> 1421978C0. It truncates the
        // Lua double to i64, then stores the low signed 32 bits.
        let value = integer(numbers, section, key)?;
        *out = if index == 0 { value.max(0) } else { value };
    }

    // 14092C600 -> 142059CF0 converts each array member to signed fixed5.
    // Native 141963700 uses the live task force's mission as this array index.
    let sortie_stance_ratios =
        numbers.fixed5_array("NNavy", "CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO")?;
    if sortie_stance_ratios.is_empty() {
        return Err(invalid("empty carrier sortie ratio array"));
    }

    Ok(Defines {
        summary_fixed,
        summary_integer,
        base_carrier_sortie_efficiency: numbers
            .fixed5("NNavy", "BASE_CARRIER_SORTIE_EFFICIENCY")?,
        selected_sortie_initial_time: integer(numbers, "NNavy", "SELECTED_SORTIE_INITIAL_TIME")?
            .max(0),
        sortie_stance_ratios,
    })
}

fn native_summary_fixed(index: usize, value: i64) -> i64 {
    // 1409516C0..140958840 clamp the positioning factors; 140953D40,
    // 140954000, 14092BBB0 and 14092BE70 clamp four screening ratios to
    // 0.01; 140955530 clamps the fleet antiair ratio. Other loaders in this
    // ordered set leave the signed fixed5 result unchanged.
    match index {
        1..=7 | 13 => value.max(0),
        8..=11 => value.max(1_000),
        _ => value,
    }
}

/// The native integer loader (`14205B690` -> `1421978C0`) through [`NumericDefines::int32`].
fn integer(numbers: &NumericDefines, section: &str, key: &str) -> io::Result<i32> {
    numbers.int32(section, key)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_post_load_bounds_apply_only_to_the_specified_defines() {
        assert_eq!(native_summary_fixed(0, -5), -5);
        for index in 1..=7 {
            assert_eq!(native_summary_fixed(index, -5), 0);
        }
        for index in 8..=11 {
            assert_eq!(native_summary_fixed(index, 999), 1_000);
        }
        assert_eq!(native_summary_fixed(12, -5), -5);
        assert_eq!(native_summary_fixed(13, -5), 0);
    }

    #[test]
    fn absent_mission_index_is_not_an_authored_zero() {
        let definitions = Defines {
            summary_fixed: [0; 14],
            summary_integer: [0; 3],
            sortie_stance_ratios: vec![0, 25_000],
            base_carrier_sortie_efficiency: 50_000,
            selected_sortie_initial_time: 24,
        };
        assert_eq!(definitions.mission_factor(1), Some(25_000));
        assert_eq!(definitions.mission_factor(2), None);
        assert_eq!(definitions.mission_factor(-1), None);
    }
}
