use super::*;

/// One occurrence in CAirBase's dense registered-container vector. Preserve
/// native order and repeated raw references; slot is authenticated against the
/// corresponding country-indexed registration, never inferred from raw id.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirBaseCountryOccurrence {
    pub country: u32,
    pub slot: u32,
}
value_struct!(AirBaseCountryOccurrence {
    country: u32,
    slot: u32
});

/// Same-hour carrier base facts. Each optional field fails independently.
/// A successfully read empty dense vector or empty name remains Some(empty).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirBaseCarrierInputs {
    pub registered_countries: Option<Vec<AirBaseCountryOccurrence>>,
    /// Exact retained CShip name bytes bound to this row's full host reference.
    /// Native null/stale host is Some(empty); acquisition failure is None.
    /// Display applies the recorded-language and UTF-8 guards.
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
pub struct CarrierAirPool {
    /// Signed +0x60 grouping key, distinct from content database index +0x58C.
    pub type_index: i32,
    /// Authenticated +0x58C content database index; a genuine null singleton
    /// is zero. This proves the held definition's name, including empty pools.
    pub subunit_index: u32,
    pub wing_start: u32,
    pub wing_count: u32,
    /// Independent passive simulation-query results, before daily scaling.
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

/// Same-hour indexed-country registration facts. None means a local read or
/// capacity refusal, not zero fuel or an empty pool inventory. The predecessor
/// capacity/wing fields remain independently usable when this packet fails.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
