use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirBaseCountryOccurrence {
    pub country: u32,
    pub slot: u32,
}
value_struct!(AirBaseCountryOccurrence {
    country: u32,
    slot: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirBaseCarrierInputs {
    pub registered_countries: Option<Vec<AirBaseCountryOccurrence>>,
    pub host_name: Option<Vec<u8>>,
}
value_struct!(AirBaseCarrierInputs {
    registered_countries: Option<Vec<AirBaseCountryOccurrence>>,
    host_name: Option<Vec<u8>>
});
impl OpaqueValue for AirBaseCarrierInputs {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CarrierAirPool {
    pub type_index: i32,
    pub subunit_index: u32,
    pub wing_start: u32,
    pub wing_count: u32,
    pub current_fuel_raw: Option<i64>,
    pub maximum_fuel_raw: Option<i64>,
}
value_struct!(CarrierAirPool {
    type_index: i32,
    subunit_index: u32,
    wing_start: u32,
    wing_count: u32,
    current_fuel_raw: Option<i64>,
    maximum_fuel_raw: Option<i64>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirBaseRegistrationCarrierInputs {
    pub country: u32,
    pub fuel_demand_raw: Option<i64>,
    pub fuel_allocated_raw: Option<i64>,
    pub pools: Option<Vec<CarrierAirPool>>,
}
value_struct!(AirBaseRegistrationCarrierInputs {
    country: u32,
    fuel_demand_raw: Option<i64>,
    fuel_allocated_raw: Option<i64>,
    pools: Option<Vec<CarrierAirPool>>
});
impl OpaqueValue for AirBaseRegistrationCarrierInputs {}

#[cfg(test)]
#[path = "carrier_air/tests.rs"]
mod tests;
