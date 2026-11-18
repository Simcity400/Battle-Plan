use super::*;

/// One occurrence in an air base's dense registered-container vector. Preserve native
/// order and repeated raw references; slot is authenticated against the corresponding
/// country-indexed registration, never inferred from raw id.
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

/// Air-base summary facts. State bases carry dense country order under the
/// registration writers; host bases observe it with the existing carrier census.
/// Each optional field fails independently.
/// A successfully read empty dense vector or empty name remains Some(empty).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirBaseCarrierInputs {
    pub registered_countries: Option<Vec<AirBaseCountryOccurrence>>,
    /// Absent for a state base. Exact retained ship name bytes bound to a host base's
    /// full host reference. Native
    /// null/stale host is Some(empty); acquisition failure is None. Display applies the
    /// recorded-language and UTF-8 guards.
    pub host_name: Option<Vec<u8>>,
}
value_struct!(AirBaseCarrierInputs {
    registered_countries: Option<Vec<AirBaseCountryOccurrence>>,
    host_name: Option<Vec<u8>>
});
opaque_value!(Option<AirBaseCarrierInputs>);

/// One pool in the country's native ordered inventory, including empty pools.
/// All ranges partition the registration's existing flattened wing vector in
/// order; the consumer checks every range and the final consumed length.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CarrierAirPool {
    /// Signed grouping key, distinct from the content database index.
    pub type_index: i32,
    /// Authenticated content database index; a genuine null singleton is zero. This proves the held
    /// definition's name, including empty pools.
    pub subunit_index: u32,
    pub wing_start: u32,
    pub wing_count: u32,
    /// Independent host-base passive simulation-query results, before daily
    /// scaling. State pools leave both absent and derive them in playback.
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

/// Indexed-country metadata and host-base same-hour fuel. State container
/// fuel uses the registration row's independent numeric columns. None means a local read or
/// capacity refusal, not zero fuel or an empty pool inventory. The predecessor
/// capacity/wing fields remain independently usable when this packet fails.
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
opaque_value!(Option<AirBaseRegistrationCarrierInputs>);

#[cfg(test)]
#[path = "carrier_air/tests.rs"]
mod tests;
