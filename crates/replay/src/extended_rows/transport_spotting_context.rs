//! Current numerical sources of F97BF0 radar, reciprocal navy intel and weather.
//! Complete original arrays are factorized by owner and region. No selected
//! country pair, native pointer or carried sliced-intel observation is stored.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportSpottingContext {
    pub sample_hour: i32,
    pub owners_complete: bool,
    pub regions_complete: bool,
    /// Authenticated original physical country roots, unique by canonical slot.
    /// Null physical slots and aliases add no substitute country object.
    pub owners: Vec<TransportSpottingOwner>,
    /// Authentic native fallback country; physical null slot zero is distinct.
    pub reserved: Option<TransportSpottingOwner>,
    pub regions: Vec<TransportSpottingRegion>,
}
value_struct!(TransportSpottingContext {
    sample_hour: i32, owners_complete: bool, regions_complete: bool,
    owners: Vec<TransportSpottingOwner>, reserved: Option<TransportSpottingOwner>,
    regions: Vec<TransportSpottingRegion>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportSpottingOwner {
    pub owner_slot: u32,
    pub owner_raw_id: u32,
    /// CEE3A0's signed target +484 gate, independently observed this hour.
    pub target_owned_states: Option<i32>,
    pub radar: Option<TransportRadarCounts>,
    pub navy_intel: Option<TransportNavyIntel>,
}
value_struct!(TransportSpottingOwner {
    owner_slot: u32, owner_raw_id: u32, target_owned_states: Option<i32>,
    radar: Option<TransportRadarCounts>, navy_intel: Option<TransportNavyIntel>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportRadarCounts {
    /// 141091B70 tests equality to zero, not signed nonpositive.
    pub signed_count: i32,
    /// None refuses the original body, including unsupported negative counts.
    pub counts: Option<Vec<i32>>,
}
value_struct!(TransportRadarCounts { signed_count: i32, counts: Option<Vec<i32>> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportNavyIntel {
    pub owner_raw_id: Option<u32>,
    /// Native canonical resolution before its signed-positive owner gate.
    /// Raw owner zero does not demand this field.
    pub owner_slot: Option<u32>,
    pub signed_count: Option<i32>,
    /// Original positional +10 vector, raw signed64 NAVY lane at stride20.
    /// Nonpositive count supplies an empty body without a pointer demand.
    pub lanes: Option<Vec<i64>>,
}
value_struct!(TransportNavyIntel {
    owner_raw_id: Option<u32>, owner_slot: Option<u32>, signed_count: Option<i32>,
    lanes: Option<Vec<i64>>,
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportSpottingRegion {
    pub region_id: u32,
    pub runtime_slot: Option<i32>,
    /// Stored signed region +24, from the same original +18 header as sample.
    pub signed_province_count: Option<i32>,
    /// F96470's original (hour-43800000)%count CProvince*, authenticated by ID.
    pub sampled_province: Option<u32>,
    /// None is refusal; Some(None) is native null; Some(Some(v)) is effective2A.
    pub weather_detection: Option<Option<i64>>,
    /// Independent weather/night operands from that same original selector.
    pub air_environment: Option<TransportAirEnvironment>,
}
value_struct!(TransportSpottingRegion {
    region_id: u32, runtime_slot: Option<i32>, signed_province_count: Option<i32>,
    sampled_province: Option<u32>, weather_detection: Option<Option<i64>>,
    air_environment: Option<TransportAirEnvironment>,
});

#[cfg(test)]
#[path = "transport_spotting_context_tests.rs"]
mod tests;
