//! Genuine hourly attack operands. No selection, target or preview result.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionAttackInputs {
    pub sample_hour: i32,
    /// Some(0): null CArmy+3B8. Nonzero requires type52 registry->same wrapper.
    pub template: Option<u64>,
    /// Authenticated stat block. Entries fail separately; do not zero-fill.
    pub stats: AttackStats,
    /// Live terrain database domain, NOT province domain. Native one-based
    /// terrain identity; each value is the narrow 141004CE0 attack operand.
    /// None refuses the whole domain; an entry None refuses that terrain only.
    pub terrain: Option<Vec<AttackTerrain>>,
    /// Fresh140607FA0 factor, independent of current-region attrition demand.
    pub acclimatization: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackStats {
    /// [CArmy+138]+390/+398, same cached block, independently observed.
    pub river: [Option<i64>; 2],
    /// [CArmy+138]+368/+370, NOT movement378.
    pub fort: [Option<i64>; 2],
    /// +3E0, NOT movement3F0.
    pub snow: Option<i64>,
    /// +340, NOT a country or actual-combat modifier row.
    pub night: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttackTerrain {
    pub terrain: i32,
    pub attack: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrdersGroupAttackInputs {
    pub sample_hour: i32,
    /// None: selector/authentication refusal. Some(None): actual null +88.
    /// 1403336A0 only; no parent/field-marshal inference.
    pub commander: Option<Option<CommanderAttackInputs>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommanderAttackInputs {
    /// Authenticated full CArmyLeader reference; no raw pointer persists.
    pub leader: u64,
    /// +FB8/+FE0/+1030/+270; each failure is independent.
    pub fort: Option<i64>,
    pub river: Option<i64>,
    pub snow: Option<i64>,
    pub night: Option<i64>,
    /// Native terrain index→+10, including exact entry0 fallback.
    /// Domain is the live terrain database. A zero live leader-vector count
    /// does NOT remove readable retained entry0. Preserve native fallback.
    pub terrain: Option<Vec<AttackTerrain>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeatherRegionAttackInputs {
    pub sample_hour: i32,
    /// Same-hour effective140551820 values from SWeatherPerRegion+58.
    /// None means failure; zero is an authenticated absent/default modifier.
    pub attack: Option<i64>, // built-in0xAD
    pub breakthrough: Option<i64>, // built-in0x1DD
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryAttackInputs {
    pub sample_hour: i32,
    /// Effective own-country0x103. Consumer chooses expeditionary raw+1DC
    /// when signed-positive, otherwise roster raw+1D8; no owner substitution.
    pub land_night_attack: Option<i64>,
    /// Own-country effective key272, independently refused.
    /// Controllers of fortified provinces are included in country admission.
    pub land_bunker_effectiveness_factor: Option<i64>,
    // No generic country tree, enemy-air85/86 capture or new table is proposed.
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProvinceAttackInputs {
    pub sample_hour: i32,
    /// SAME current-hour healthy*template308 wrapping i32 sum the existing
    /// cprovince fort_positive acquisition computes; retain before bool collapse.
    /// No second building walk. None is a counted suffix read refusal.
    pub fort_sum: Option<i32>,
    /// Target state country-specific key272, queried with actual resolved
    /// controller object's stored+8 raw ID (not guessed from input alias).
    /// Some(0) includes actual null-state case. None refuses this value only.
    /// When known fort_sum<=0 this value is undemanded; native output cannot
    /// insert a fort row. Consumer MUST test fort_sum before requiring it.
    pub land_bunker_effectiveness_factor: Option<i64>,
}

value_struct!(AttackStats { river: [Option<i64>;2], fort: [Option<i64>;2], snow: Option<i64>, night: Option<i64> });

value_struct!(AttackTerrain { terrain: i32, attack: Option<i64> });

value_struct!(DivisionAttackInputs { sample_hour: i32, template: Option<u64>, stats: AttackStats, terrain: Option<Vec<AttackTerrain>>, acclimatization: Option<i64> });

value_struct!(OrdersGroupAttackInputs { sample_hour: i32, commander: Option<Option<CommanderAttackInputs>> });

value_struct!(CommanderAttackInputs { leader: u64, fort: Option<i64>, river: Option<i64>, snow: Option<i64>, night: Option<i64>, terrain: Option<Vec<AttackTerrain>> });

value_struct!(WeatherRegionAttackInputs { sample_hour: i32, attack: Option<i64>, breakthrough: Option<i64> });

value_struct!(CountryAttackInputs { sample_hour: i32, land_night_attack: Option<i64>, land_bunker_effectiveness_factor: Option<i64> });

value_struct!(ProvinceAttackInputs { sample_hour: i32, fort_sum: Option<i32>, land_bunker_effectiveness_factor: Option<i64> });
opaque_value!(DivisionAttackInputs);
opaque_value!(OrdersGroupAttackInputs);
opaque_value!(WeatherRegionAttackInputs);
opaque_value!(CountryAttackInputs);
opaque_value!(ProvinceAttackInputs);
opaque_value!(Option<CountryAttackInputs>);

/// Shared runtime scalar family. Every Option is an independent read refusal.
/// The sample hour comes from the same accepted pass as these six native reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoliticalRuntimeScalars {
    pub sample_hour: i32,
    pub river_small: Option<i64>,
    pub river_large: Option<i64>,
    pub night: Option<i64>,
    pub fort: Option<i64>,
    pub move_strength_threshold: Option<i64>,
    pub move_organisation_threshold: Option<i64>,
}
value_struct!(PoliticalRuntimeScalars { sample_hour: i32, river_small: Option<i64>,
    river_large: Option<i64>, night: Option<i64>, fort: Option<i64>,
    move_strength_threshold: Option<i64>, move_organisation_threshold: Option<i64> });
opaque_value!(PoliticalRuntimeScalars);

pub(crate) fn check_attack_terrains(values: &Option<Vec<AttackTerrain>>) -> io::Result<()> {
    if let Some(values) = values {
        let mut previous = 0;
        for value in values {
            if value.terrain <= previous {
                return Err(invalid(
                    "attack terrain keys are not positive and increasing",
                ));
            }
            previous = value.terrain;
        }
    }
    Ok(())
}
impl DivisionAttackInputs {
    pub(crate) fn check(&self) -> io::Result<()> {
        if self.template.is_some_and(|handle| {
            handle != 0
                && (handle as u32 != engine::creferenced_division_template::REFERENCE_TYPE
                    || handle >> 32 == 0)
        }) {
            return Err(invalid(
                "attack template reference has invalid type or identity",
            ));
        }
        check_attack_terrains(&self.terrain)
    }
}
impl OrdersGroupAttackInputs {
    pub(crate) fn check(&self) -> io::Result<()> {
        if let Some(Some(commander)) = &self.commander {
            if commander.leader as u32 != engine::cunit_leader::REFERENCE_TYPE
                || commander.leader >> 32 == 0
            {
                return Err(invalid(
                    "attack commander reference has invalid type or identity",
                ));
            }
            check_attack_terrains(&commander.terrain)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{codec::Decoder, value::Value};
    #[test]
    fn runtime_scalars_exact_signed_values_refusal_and_freshness_round_trip() {
        let values = PoliticalRuntimeScalars {
            sample_hour: -17,
            river_small: Some(i64::MIN),
            river_large: None,
            night: Some(0),
            fort: Some(i64::MAX),
            move_strength_threshold: Some(-1),
            move_organisation_threshold: Some(1),
        };
        let mut bytes = Vec::new();
        Value::encode(&values, &mut bytes).unwrap();
        assert_eq!(Value::encoded_bytes(&values).unwrap(), bytes.len());
        let mut input = Decoder::new(&bytes);
        assert_eq!(PoliticalRuntimeScalars::decode(&mut input).unwrap(), values);
        input.finish().unwrap();
        // Change one strict Option tag, beyond the sample hour.
        bytes[4] = 2;
        assert!(PoliticalRuntimeScalars::decode(&mut Decoder::new(&bytes)).is_err());
    }
    #[test]
    fn terrain_and_reference_certificates_reject_ambiguous_identity() {
        let mut packet = DivisionAttackInputs {
            template: Some((7_u64 << 32) | 52),
            terrain: Some(vec![AttackTerrain {
                terrain: 1,
                attack: Some(0),
            }]),
            ..Default::default()
        };
        packet.check().unwrap();
        packet.terrain.as_mut().unwrap().push(AttackTerrain {
            terrain: 1,
            attack: None,
        });
        assert!(packet.check().is_err());
        packet.terrain = None;
        packet.template = Some(52);
        assert!(packet.check().is_err());
        packet.template = Some(0);
        packet.check().unwrap();
    }
}
