
use crate::{ResolvedLayers, defines::Defines as NumericDefines};
use std::io;

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

const SILHOUETTE_PLANE_KEYS: [&str; 6] = [
    "SHIP_SILHOUETTE_VALUE_PLANES_CAPITAL",
    "SHIP_SILHOUETTE_VALUE_PLANES_SCREEN",
    "SHIP_SILHOUETTE_VALUE_PLANES_CARRIER",
    "SHIP_SILHOUETTE_VALUE_PLANES_SUPPORT",
    "SHIP_SILHOUETTE_VALUE_PLANES_CONVOY",
    "SHIP_SILHOUETTE_VALUE_PLANES_SUBMARINE",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Defines {
    pub summary_fixed: [i64; 14],
    pub summary_integer: [i32; 3],
    pub silhouette_planes: [i32; 6],
    pub screen_cap_reduction_factor: i64,
    pub plane_stacking_penalty_effect: i64,
    pub sortie_stance_ratios: Vec<i64>,
    pub base_carrier_sortie_efficiency: i64,
    pub selected_sortie_initial_time: i32,
    pub piercing: Result<Piercing, String>,
    pub positioning_impacts: [i64; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Piercing {
    pub thresholds: Vec<i64>,
    pub critical_values: Vec<i64>,
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
    let piercing = (|| -> io::Result<Piercing> {
        Ok(Piercing {
            thresholds: numbers.fixed5_array("NNavy", "NAVY_PIERCING_THRESHOLDS")?,
            critical_values: numbers
                .fixed5_array("NNavy", "NAVY_PIERCING_THRESHOLD_CRITICAL_VALUES")?,
        })
    })()
    .map_err(|error| error.to_string());
    let mut summary_fixed = [0; 14];
    for (index, (out, (section, key))) in
        summary_fixed.iter_mut().zip(SUMMARY_FIXED_KEYS).enumerate()
    {
        *out = native_summary_fixed(index, numbers.fixed5(section, key)?);
    }
    let mut summary_integer = [0; 3];
    for (index, (out, (section, key))) in summary_integer
        .iter_mut()
        .zip(SUMMARY_INTEGER_KEYS)
        .enumerate()
    {
        let value = integer(numbers, section, key)?;
        *out = if index == 0 { value.max(0) } else { value };
    }
    let mut silhouette_planes = [0; 6];
    for (out, key) in silhouette_planes.iter_mut().zip(SILHOUETTE_PLANE_KEYS) {
        *out = integer(numbers, "NNavy", key)?;
    }

    let sortie_stance_ratios =
        numbers.fixed5_array("NNavy", "CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO")?;
    if sortie_stance_ratios.is_empty() {
        return Err(invalid("empty carrier sortie ratio array"));
    }

    Ok(Defines {
        summary_fixed,
        summary_integer,
        silhouette_planes,
        screen_cap_reduction_factor: numbers.fixed5("NNavy", "SCREEN_CAP_REDUCTION_FACTOR")?,
        plane_stacking_penalty_effect: numbers
            .fixed5("NNavy", "NAVAL_COMBAT_PLANE_STACKING_PENALTY_EFFECT")?,
        base_carrier_sortie_efficiency: numbers
            .fixed5("NNavy", "BASE_CARRIER_SORTIE_EFFICIENCY")?,
        selected_sortie_initial_time: integer(numbers, "NNavy", "SELECTED_SORTIE_INITIAL_TIME")?
            .max(0),
        piercing,
        positioning_impacts: [
            numbers.fixed5("NNavy", "DAMAGE_PENALTY_ON_MINIMUM_POSITIONING")?,
            numbers.fixed5("NNavy", "AA_EFFICIENCY_PENALTY_ON_MINIMUM_POSITIONING")?,
            numbers.fixed5("NNavy", "SUBMARINE_REVEAL_ON_MINIMUM_POSITIONING")?,
        ],
        sortie_stance_ratios,
    })
}

fn native_summary_fixed(index: usize, value: i64) -> i64 {
    match index {
        1..=7 | 13 => value.max(0),
        8..=11 => value.max(1_000),
        _ => value,
    }
}

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
            silhouette_planes: [0; 6],
            screen_cap_reduction_factor: 0,
            plane_stacking_penalty_effect: 0,
            sortie_stance_ratios: vec![0, 25_000],
            base_carrier_sortie_efficiency: 50_000,
            selected_sortie_initial_time: 24,
            piercing: Ok(Piercing {
                thresholds: vec![100_000, 50_000],
                critical_values: vec![50_000, 0],
            }),
            positioning_impacts: [0; 3],
        };
        assert_eq!(definitions.mission_factor(1), Some(25_000));
        assert_eq!(definitions.mission_factor(2), None);
        assert_eq!(definitions.mission_factor(-1), None);
    }

    fn without_piercing() -> String {
        let mut script = String::from("NDefines = { NNavy = {}, NAir = {} }\n");
        for &(section, key) in SUMMARY_FIXED_KEYS.iter().chain(&SUMMARY_INTEGER_KEYS) {
            script.push_str(&format!("NDefines.{section}.{key} = 1\n"));
        }
        for key in SILHOUETTE_PLANE_KEYS {
            script.push_str(&format!("NDefines.NNavy.{key} = 1\n"));
        }
        for key in [
            "SCREEN_CAP_REDUCTION_FACTOR",
            "NAVAL_COMBAT_PLANE_STACKING_PENALTY_EFFECT",
            "BASE_CARRIER_SORTIE_EFFICIENCY",
            "SELECTED_SORTIE_INITIAL_TIME",
            "DAMAGE_PENALTY_ON_MINIMUM_POSITIONING",
            "AA_EFFICIENCY_PENALTY_ON_MINIMUM_POSITIONING",
            "SUBMARINE_REVEAL_ON_MINIMUM_POSITIONING",
        ] {
            script.push_str(&format!("NDefines.NNavy.{key} = 1\n"));
        }
        script.push_str("NDefines.NNavy.CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO = { 1 }\n");
        script
    }

    #[test]
    fn piercing_arrays_are_needed_only_by_their_explanation() {
        let numbers = NumericDefines::new().unwrap();
        numbers
            .load("common/defines/test.lua", without_piercing().as_bytes())
            .unwrap();
        let read = from_numbers(&numbers).unwrap();
        assert!(read.piercing.is_err());
        assert_eq!(read.summary_integer, [1; 3]);
        let numbers = NumericDefines::new().unwrap();
        let script = without_piercing()
            + "NDefines.NNavy.NAVY_PIERCING_THRESHOLDS = {}\n\
               NDefines.NNavy.NAVY_PIERCING_THRESHOLD_CRITICAL_VALUES = {}\n";
        numbers
            .load("common/defines/test.lua", script.as_bytes())
            .unwrap();
        assert_eq!(
            from_numbers(&numbers).unwrap().piercing,
            Ok(Piercing::default())
        );
    }

    #[test]
    fn silhouette_and_stacking_defines_follow_authored_values() {
        let numbers = NumericDefines::new().unwrap();
        let mut script = String::from("NDefines = { NNavy = {}, NAir = {} }\n");
        for &(section, key) in &SUMMARY_FIXED_KEYS {
            script.push_str(&format!("NDefines.{section}.{key} = 1\n"));
        }
        for &(section, key) in &SUMMARY_INTEGER_KEYS {
            script.push_str(&format!("NDefines.{section}.{key} = 1\n"));
        }
        for (index, key) in SILHOUETTE_PLANE_KEYS.iter().enumerate() {
            script.push_str(&format!("NDefines.NNavy.{key} = {}\n", index + 2));
        }
        script.push_str(
            "NDefines.NNavy.SCREEN_CAP_REDUCTION_FACTOR = 0.03\n\
             NDefines.NNavy.NAVAL_COMBAT_PLANE_STACKING_PENALTY_EFFECT = 0.004\n\
             NDefines.NNavy.CARRIER_OFFENSIVE_STANCE_SORTIE_RATIO = { 1 }\n\
             NDefines.NNavy.BASE_CARRIER_SORTIE_EFFICIENCY = 1\n\
             NDefines.NNavy.SELECTED_SORTIE_INITIAL_TIME = 1\n\
             NDefines.NNavy.NAVY_PIERCING_THRESHOLDS = { 2, 1 }\n\
             NDefines.NNavy.NAVY_PIERCING_THRESHOLD_CRITICAL_VALUES = { 2, 1 }\n\
             NDefines.NNavy.DAMAGE_PENALTY_ON_MINIMUM_POSITIONING = 0.1\n\
             NDefines.NNavy.AA_EFFICIENCY_PENALTY_ON_MINIMUM_POSITIONING = 0.2\n\
             NDefines.NNavy.SUBMARINE_REVEAL_ON_MINIMUM_POSITIONING = 0.3\n",
        );
        numbers
            .load("common/defines/test.lua", script.as_bytes())
            .unwrap();
        let read = from_numbers(&numbers).unwrap();
        assert_eq!(read.silhouette_planes, [2, 3, 4, 5, 6, 7]);
        assert_eq!(read.screen_cap_reduction_factor, 3_000);
        assert_eq!(read.plane_stacking_penalty_effect, 400);
        assert_eq!(read.positioning_impacts, [10_000, 20_000, 30_000]);
    }
}
