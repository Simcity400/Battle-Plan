//! Numerical F49710/F4B6E0 operands. Native controls never enter the codec.
use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportAirModifiers {
    pub direct: [Option<i64>; 2],
    pub positive: [Option<u32>; 2],
    /// Native key order belongs to transport_air_normalization::KEYS.
    pub effective: [Option<i64>; 16],
}
value_struct!(TransportAirModifiers {
    direct: [Option<i64>; 2], positive: [Option<u32>; 2], effective: [Option<i64>; 16],
});
impl From<engine::transport_air_normalization::Modifiers> for TransportAirModifiers {
    fn from(value: engine::transport_air_normalization::Modifiers) -> Self {
        Self {
            direct: value.direct,
            positive: value.positive,
            effective: value.effective,
        }
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportAirCombatModifier {
    pub lane: u8,
    pub raw: Option<i64>,
}
value_struct!(TransportAirCombatModifier { lane: u8, raw: Option<i64> });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirAce {
    pub assigned_aircraft: Option<Option<i32>>,
    pub modifiers: Option<TransportAirModifiers>,
    pub type_modifiers: Option<TransportAirModifiers>,
    pub owner_effectiveness: Option<i64>,
    /// Ordered raw occurrences: multiply and truncate each before adding.
    pub combat: Option<Vec<TransportAirCombatModifier>>,
}
value_struct!(TransportAirAce {
    assigned_aircraft: Option<Option<i32>>, modifiers: Option<TransportAirModifiers>,
    type_modifiers: Option<TransportAirModifiers>, owner_effectiveness: Option<i64>,
    combat: Option<Vec<TransportAirCombatModifier>>,
});
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportAirEnvironment {
    pub weather: [Option<i64>; 2],
    pub night_factor: Option<i64>,
    pub night: [Option<i64>; 2],
    pub target_region: Option<i32>,
}
value_struct!(TransportAirEnvironment {
    weather: [Option<i64>; 2], night_factor: Option<i64>,
    night: [Option<i64>; 2], target_region: Option<i32>,
});
impl From<engine::transport_air_normalization::Environment> for TransportAirEnvironment {
    fn from(value: engine::transport_air_normalization::Environment) -> Self {
        Self {
            weather: value.weather,
            night_factor: value.night_factor,
            night: value.night,
            target_region: value.target_region,
        }
    }
}
impl From<TransportAirEnvironment> for engine::transport_air_normalization::Environment {
    fn from(value: TransportAirEnvironment) -> Self {
        Self {
            weather: value.weather,
            night_factor: value.night_factor,
            night: value.night,
            target_region: value.target_region,
        }
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirNormalization {
    pub attack: Option<i64>,
    pub defence: Option<i64>,
    pub speed: Option<i64>,
    pub enabled_missions: u32,
    pub wing: Option<TransportAirModifiers>,
    pub country: Option<TransportAirModifiers>,
    pub ace: Option<Option<TransportAirAce>>,
    pub environment: Option<Option<TransportAirEnvironment>>,
    pub night_stat: Option<i64>,
    pub base_region: Option<i32>,
    pub carrier_mission: Option<Option<i32>>,
}
value_struct!(TransportAirNormalization {
    attack: Option<i64>, defence: Option<i64>, speed: Option<i64>, enabled_missions: u32,
    wing: Option<TransportAirModifiers>, country: Option<TransportAirModifiers>,
    ace: Option<Option<TransportAirAce>>, environment: Option<Option<TransportAirEnvironment>>,
    night_stat: Option<i64>, base_region: Option<i32>, carrier_mission: Option<Option<i32>>,
});
impl TransportAirNormalization {
    /// Conversion runs after native pointer retirement; only numerical spans
    /// are copied, using the callback's retained arena without another read.
    pub fn from_native(
        value: engine::transport_air_normalization::Source,
        captured: &engine::transport_air_normalization::Capture,
    ) -> std::io::Result<Self> {
        let ace = match value.ace {
            None => None,
            Some(None) => Some(None),
            Some(Some(ace)) => {
                let combat = match ace.modifiers.and_then(|modifiers| modifiers.combat) {
                    None => None,
                    Some(span) => match captured.combat(span) {
                        Err(_) => None,
                        Ok(entries) => {
                            let mut output = engine::allocation::vec(entries.len())?;
                            output.extend(entries.iter().map(|entry| TransportAirCombatModifier {
                                lane: entry.lane,
                                raw: entry.raw,
                            }));
                            Some(output)
                        }
                    },
                };
                Some(Some(TransportAirAce {
                    assigned_aircraft: ace.assigned_aircraft,
                    modifiers: ace.modifiers.map(Into::into),
                    type_modifiers: ace.type_modifiers.map(Into::into),
                    owner_effectiveness: ace.owner_effectiveness,
                    combat,
                }))
            }
        };
        Ok(Self {
            attack: value.attack,
            defence: value.defence,
            speed: value.speed,
            enabled_missions: value.enabled_missions,
            wing: value.wing.map(Into::into),
            country: value.country.map(Into::into),
            ace,
            environment: value.environment.map(|value| value.map(Into::into)),
            night_stat: value.night_stat,
            base_region: value.base_region,
            carrier_mission: value.carrier_mission,
        })
    }
}
