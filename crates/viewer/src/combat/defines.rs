//! The content's `NDefines` the land combat window reads, in native form.
use installed_content::defines::Defines;
use std::io;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LandDefines {
    /// `COMBAT_{GOOD,SOME}_PIERCING` then `COMBAT_{GOOD,SOME}_ARMOR`, narrowed
    /// to f32 as the define registry stores them (14205B9E0).
    pub(crate) armour_icons: [f32; 4],
    /// `PIERCING_THRESHOLDS`, the piercing buckets in authored order.
    pub(crate) piercing_thresholds: Vec<i64>,
    /// `ENEMY_AIR_SUPERIORITY_DEFENSE_STEEPNESS` and `_DEFENSE`.
    pub(crate) air_superiority: [i64; 2],
    pub(crate) fuel_penalty_start_ratio: i64,
    pub(crate) recon_skill_impact: i32,
    /// `GIE_DIVISION_{ATTACK,DEFENSE}_BONUS_AGAINST_OCCUPIER`, which gate the
    /// attacker's and the defender's occupier badge. The NCountry loaders
    /// clamp a negative value to zero.
    pub(crate) exile_against_occupier: [i64; 2],
}

impl LandDefines {
    pub(crate) fn read(defines: &Defines) -> io::Result<Self> {
        let float = |section: &str, name: &str| -> io::Result<f32> {
            let value = defines.number(section, name)? as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("NDefines.{section}.{name} exceeds native f32"),
                ))
            }
        };
        let exile = |name: &str| defines.fixed5("NCountry", name).map(|value| value.max(0));
        Ok(Self {
            armour_icons: [
                float("NInterface", "COMBAT_GOOD_PIERCING")?,
                float("NInterface", "COMBAT_SOME_PIERCING")?,
                float("NInterface", "COMBAT_GOOD_ARMOR")?,
                float("NInterface", "COMBAT_SOME_ARMOR")?,
            ],
            piercing_thresholds: defines.fixed5_array("NMilitary", "PIERCING_THRESHOLDS")?,
            air_superiority: [
                defines.fixed5("NMilitary", "ENEMY_AIR_SUPERIORITY_DEFENSE_STEEPNESS")?,
                defines.fixed5("NMilitary", "ENEMY_AIR_SUPERIORITY_DEFENSE")?,
            ],
            fuel_penalty_start_ratio: defines.fixed5("NMilitary", "FUEL_PENALTY_START_RATIO")?,
            recon_skill_impact: defines.int32("NMilitary", "RECON_SKILL_IMPACT")?,
            exile_against_occupier: [
                exile("GIE_DIVISION_ATTACK_BONUS_AGAINST_OCCUPIER")?,
                exile("GIE_DIVISION_DEFENSE_BONUS_AGAINST_OCCUPIER")?,
            ],
        })
    }
}
