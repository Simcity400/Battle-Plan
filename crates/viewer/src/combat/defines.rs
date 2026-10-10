//! The content's `NDefines` the land combat window reads, in native form.
use installed_content::defines::Defines;
use std::io;

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// delayed armour text: deflection factors (fixed5) and the
/// signed integer dice sizes it divides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ArmourHover {
    pub(crate) organisation_deflection: i64,
    pub(crate) strength_deflection: i64,
    pub(crate) organisation_armour_dice: i32,
    pub(crate) strength_armour_dice: i32,
    pub(crate) organisation_die: i32,
    pub(crate) strength_die: i32,
}

/// reserve reinforcement operands, fixed5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reinforce {
    pub(crate) over_width_penalty: i64,
    pub(crate) over_width_penalty_max: i64,
    pub(crate) chance: i64,
    pub(crate) initiative_factor: i64,
    pub(crate) speed_bonus: i64,
    pub(crate) slowest_speed: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LandDefines {
    /// `COMBAT_{GOOD,SOME}_PIERCING` then `COMBAT_{GOOD,SOME}_ARMOR`, narrowed
    /// to f32 as the define registry stores them.
    pub(crate) armour_icons: [f32; 4],
    /// `PIERCING_THRESHOLDS` and `PIERCING_THRESHOLD_DAMAGE_VALUES`, parallel.
    pub(crate) piercing_thresholds: Vec<i64>,
    pub(crate) piercing_damage: Vec<i64>,
    /// `ENEMY_AIR_SUPERIORITY_DEFENSE_STEEPNESS` and `_DEFENSE`.
    pub(crate) air_superiority: [i64; 2],
    /// `NAir.ANTI_AIR_ATTACK_TO_DAMAGE_REDUCTION_FACTOR` and
    /// `ANTI_AIR_MAXIMUM_DAMAGE_REDUCTION_FACTOR`.
    pub(crate) anti_air: [i64; 2],
    /// `LAND_AIR_COMBAT_MAX_PLANES_PER_ENEMY_WIDTH`.
    pub(crate) planes_per_width: i64,
    pub(crate) fuel_penalty_start_ratio: i64,
    pub(crate) recon_skill_impact: i32,
    /// `GIE_DIVISION_{ATTACK,DEFENSE}_BONUS_AGAINST_OCCUPIER` and `_ON_CORE`;
    /// the NCountry loaders clamp a negative value to zero.
    pub(crate) exile_against_occupier: [i64; 2],
    pub(crate) exile_on_core: [i64; 2],
    /// `NPolitics.LEADER_TRAITS_XP_SHOW`, the learning-trait display floor.
    pub(crate) learning_threshold: i64,
    pub(crate) armour_hover: ArmourHover,
    pub(crate) reinforce: Reinforce,
    /// Country, general and field-marshal preferred-tactic weight factors
    pub(crate) preference_factors: [i64; 3],
}

impl LandDefines {
    pub(crate) fn read(defines: &Defines) -> io::Result<Self> {
        let float = |section: &str, name: &str| -> io::Result<f32> {
            let value = defines.number(section, name)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(invalid(format!(
                    "NDefines.{section}.{name} exceeds native f32"
                )))
            }
        };
        let fixed = |section: &str, name: &str| defines.fixed5(section, name);
        let military = |name: &str| defines.fixed5("NMilitary", name);
        let dice = |name: &str| defines.int32("NMilitary", name);
        let exile = |name: &str| defines.fixed5("NCountry", name).map(|value| value.max(0));
        let piercing_thresholds = defines.fixed5_array("NMilitary", "PIERCING_THRESHOLDS")?;
        let piercing_damage =
            defines.fixed5_array("NMilitary", "PIERCING_THRESHOLD_DAMAGE_VALUES")?;
        if piercing_thresholds.len() != piercing_damage.len() || piercing_thresholds.len() < 2 {
            return Err(invalid(
                "NMilitary piercing thresholds and damage values need the same length, at least two"
                    .into(),
            ));
        }
        Ok(Self {
            armour_icons: [
                float("NInterface", "COMBAT_GOOD_PIERCING")?,
                float("NInterface", "COMBAT_SOME_PIERCING")?,
                float("NInterface", "COMBAT_GOOD_ARMOR")?,
                float("NInterface", "COMBAT_SOME_ARMOR")?,
            ],
            piercing_thresholds,
            piercing_damage,
            air_superiority: [
                military("ENEMY_AIR_SUPERIORITY_DEFENSE_STEEPNESS")?,
                military("ENEMY_AIR_SUPERIORITY_DEFENSE")?,
            ],
            anti_air: [
                fixed("NAir", "ANTI_AIR_ATTACK_TO_DAMAGE_REDUCTION_FACTOR")?,
                fixed("NAir", "ANTI_AIR_MAXIMUM_DAMAGE_REDUCTION_FACTOR")?,
            ],
            planes_per_width: military("LAND_AIR_COMBAT_MAX_PLANES_PER_ENEMY_WIDTH")?,
            fuel_penalty_start_ratio: military("FUEL_PENALTY_START_RATIO")?,
            recon_skill_impact: dice("RECON_SKILL_IMPACT")?,
            exile_against_occupier: [
                exile("GIE_DIVISION_ATTACK_BONUS_AGAINST_OCCUPIER")?,
                exile("GIE_DIVISION_DEFENSE_BONUS_AGAINST_OCCUPIER")?,
            ],
            exile_on_core: [
                exile("GIE_DIVISION_ATTACK_BONUS_ON_CORE")?,
                exile("GIE_DIVISION_DEFENSE_BONUS_ON_CORE")?,
            ],
            learning_threshold: fixed("NPolitics", "LEADER_TRAITS_XP_SHOW")?,
            armour_hover: ArmourHover {
                organisation_deflection: military("LAND_COMBAT_ORG_ARMOR_DEFLECTION_FACTOR")?,
                strength_deflection: military("LAND_COMBAT_STR_ARMOR_DEFLECTION_FACTOR")?,
                organisation_armour_dice: dice("LAND_COMBAT_ORG_ARMOR_ON_SOFT_DICE_SIZE")?,
                strength_armour_dice: dice("LAND_COMBAT_STR_ARMOR_ON_SOFT_DICE_SIZE")?,
                organisation_die: dice("LAND_COMBAT_ORG_DICE_SIZE")?,
                strength_die: dice("LAND_COMBAT_STR_DICE_SIZE")?,
            },
            reinforce: Reinforce {
                over_width_penalty: military("COMBAT_OVER_WIDTH_PENALTY")?,
                over_width_penalty_max: military("COMBAT_OVER_WIDTH_PENALTY_MAX")?,
                chance: military("REINFORCE_CHANCE")?,
                initiative_factor: military("ARMY_INITIATIVE_REINFORCE_FACTOR")?,
                speed_bonus: military("SPEED_REINFORCEMENT_BONUS")?,
                slowest_speed: military("SLOWEST_SPEED")?,
            },
            preference_factors: [
                military("COUNTRY_PREFERRED_TACTIC_WEIGHT_FACTOR")?,
                military("ARMY_GENERAL_PREFERRED_TACTIC_WEIGHT_FACTOR")?,
                military("FIELD_MARSHAL_PREFERRED_TACTIC_WEIGHT_FACTOR")?,
            ],
        })
    }
}
