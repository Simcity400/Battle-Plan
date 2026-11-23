use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirRegion {
    pub region: u32,
    pub count: Option<i32>,
    pub missions: Option<Vec<Option<TransportAirMission>>>,
}
value_struct!(TransportAirRegion {
    region: u32, count: Option<i32>, missions: Option<Vec<Option<TransportAirMission>>>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportAirMission {
    pub wing: u64,
    pub current_type: Option<u32>,
    pub aircraft: Option<i32>,
    pub disruption: Option<i64>,
    pub detection_13: Option<i64>,
    pub detection_14: Option<i64>,
    pub normalization: Option<TransportAirNormalization>,
}
value_struct!(TransportAirMission {
    wing: u64, current_type: Option<u32>, aircraft: Option<i32>, disruption: Option<i64>,
    detection_13: Option<i64>, detection_14: Option<i64>,
    normalization: Option<TransportAirNormalization>,
});
