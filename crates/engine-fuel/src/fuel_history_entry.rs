//! Pending fuel-history record and hourly ring entries. Hourly country fuel
//! update copies pending state to the ring and clears it. CountryFuel sums
//! each component except the fuel level, which is current state rather than
//! an amount per update.

/// History entry stride.
pub const STRIDE: usize = 0x50;
/// Fuel i64 /32768.
pub const FUEL: usize = 0x00;
/// Fuel gain i64 /100000.
pub const GAIN: usize = 0x08;
/// Consumed fuel i64 /32768.
pub const CONSUMED: usize = 0x10;
/// Requested fuel, three i64 /100000.
pub const REQUESTED: usize = 0x18;
/// Other fuel i64 /32768.
pub const OTHER: usize = 0x30;
/// Received fuel, three i64 /100000.
pub const RECEIVED: usize = 0x38;
/// Values per requested or received list.
pub const CONSUMER_VALUES: usize = 3;
