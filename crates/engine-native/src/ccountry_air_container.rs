//! CCountryAirContainer: one country's pool membership at one CAirBase. It is not the per-country
//! CStrategicAir object in the strategic air manager's country table.

/// Country air-container vtable.
pub const VTABLE_RVA: usize = 0x029d_5000;
/// Allocation size.
pub const SIZE: usize = 0xf8;
/// Inherited supply ratio, signed fixed5.
pub const SUPPLY_RATIO_RAW: usize = 0x30;
/// Paired-u32 reference candidate, initially zero.
pub const REFERENCE_CANDIDATE: usize = 0xa8;
/// Raw country link index.
pub const COUNTRY: usize = 0xb0;
/// Wing-pool membership vector in registration order.
pub const POOLS: usize = 0xb8;
/// Pool count, u32.
pub const POOLS_COUNT: usize = 0xc4;
/// Capacity word, i64, initialized to the raw default.
pub const CAPACITY_RAW: usize = 0xd0;
/// Initial raw capacity value.
#[cfg(any(test, feature = "test-support"))]
pub const INITIAL_CAPACITY_RAW: i64 = 100_000;
/// Per-base sortie capacity factor, signed fixed5.
pub const SORTIE_CAPACITY_FACTOR_RAW: usize = 0xd8;
/// Hourly fuel demand summed from registered wings, supplying the fuel-ratio divisor.
pub const FUEL_DEMAND_RAW: usize = 0xe0;
/// Allocated base/country fuel supplying the fuel-ratio numerator.
pub const FUEL_ALLOCATED_RAW: usize = 0xf0;
