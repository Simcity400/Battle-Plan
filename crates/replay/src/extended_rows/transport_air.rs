//! F96470's original strategic-air root/region domain and stored mission leaves.
//! No pointer, calculated sortie count or historical mission type is recorded.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirInputs {
    pub sample_hour: i32,
    /// Complete original physical CStrategicAir manager slots, including nulls.
    pub countries: Option<Vec<TransportAirCountry>>,
}
value_struct!(TransportAirInputs { sample_hour: i32, countries: Option<Vec<TransportAirCountry>> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirCountry {
    pub country: Option<u32>,
    pub slot: u32,
    pub present: bool,
    /// Every +98 region header, indexed by its original physical ID. None
    /// refuses the original domain. Each failed header is independently None.
    pub regions: Option<Vec<TransportAirRegion>>,
    /// Original +E0 records, strideA0, raw signed +80 values. F96470 uses
    /// the original region runtime slot, separately from its region ID.
    pub factors: Option<Vec<Option<i64>>>,
}
value_struct!(TransportAirCountry {
    country: Option<u32>, slot: u32, present: bool, regions: Option<Vec<TransportAirRegion>>,
    factors: Option<Vec<Option<i64>>>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirRegion {
    pub region: u32,
    pub count: Option<i32>,
    /// Original order, including repeated wings and native null pointers.
    /// Native nulls refuse a reached F96470 dereference; no entry is dropped.
    pub missions: Option<Vec<Option<TransportAirMission>>>,
}
value_struct!(TransportAirRegion {
    region: u32, count: Option<i32>, missions: Option<Vec<Option<TransportAirMission>>>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportAirMission {
    pub wing: u64,
    pub current_type: Option<u32>,
    pub aircraft: Option<i32>,
    pub disruption: Option<i64>,
    pub detection_13: Option<i64>,
    pub detection_14: Option<i64>,
    /// F49710/F4B6E0's raw producer graph. None refuses a reached producer.
    pub normalization: Option<TransportAirNormalization>,
}
value_struct!(TransportAirMission {
    wing: u64, current_type: Option<u32>, aircraft: Option<i32>, disruption: Option<i64>,
    detection_13: Option<i64>, detection_14: Option<i64>,
    normalization: Option<TransportAirNormalization>,
});
