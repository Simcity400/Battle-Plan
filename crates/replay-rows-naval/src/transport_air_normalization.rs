use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirModifiers {
    pub direct: [Option<i64>; 2],
    pub positive: [Option<u32>; 2],
    pub effective: [Option<i64>; 16],
}
value_struct!(TransportAirModifiers {
    direct: [Option<i64>; 2], positive: [Option<u32>; 2], effective: [Option<i64>; 16],
});
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirCombatModifier {
    pub lane: u8,
    pub raw: Option<i64>,
}
value_struct!(TransportAirCombatModifier { lane: u8, raw: Option<i64> });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirAce {
    pub assigned_aircraft: Option<Option<i32>>,
    pub modifiers: Option<TransportAirModifiers>,
    pub type_modifiers: Option<TransportAirModifiers>,
    pub owner_effectiveness: Option<i64>,
    pub combat: Option<Vec<TransportAirCombatModifier>>,
}
value_struct!(TransportAirAce {
    assigned_aircraft: Option<Option<i32>>, modifiers: Option<TransportAirModifiers>,
    type_modifiers: Option<TransportAirModifiers>, owner_effectiveness: Option<i64>,
    combat: Option<Vec<TransportAirCombatModifier>>,
});
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
