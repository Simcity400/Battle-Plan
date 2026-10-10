//! Original hourly CConvoys observation. No daily stock or cache retry joins.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportConvoys {
    pub sample_hour: i32,
    pub priority: Option<i32>,
    /// Independent signed CCountry+1288 branch of BEFC40.
    pub unreserved: Option<i32>,
    /// None refuses the original cache attempt. Dirty requires `pool` below.
    pub cache: Option<TransportConvoyCache>,
    /// Complete unfiltered native order, including zero/negative quantities.
    pub pool: Option<Vec<TransportConvoyEntry>>,
    pub reservations: Option<Vec<TransportConvoyReservation>>,
    /// C740(1) selects the first NONZERO quantity, without classifier filtering.
    /// Some(None) proves an empty selection; None refuses its identity.
    /// Selected variant stat failures remain independent optional leaves.
    pub first_variant: Option<Option<TransportConvoyVariant>>,
}
value_struct!(TransportConvoys {
    sample_hour: i32, priority: Option<i32>, unreserved: Option<i32>, cache: Option<TransportConvoyCache>,
    pool: Option<Vec<TransportConvoyEntry>>, reservations: Option<Vec<TransportConvoyReservation>>,
    first_variant: Option<Option<TransportConvoyVariant>>,
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportConvoyCache {
    pub valid: u8,
    pub raw: Option<i64>,
}
value_struct!(TransportConvoyCache { valid: u8, raw: Option<i64> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportConvoyEntry {
    pub variant: Option<u64>,
    pub flags: Option<u64>,
    pub quantity: i64,
}
value_struct!(TransportConvoyEntry { variant: Option<u64>, flags: Option<u64>, quantity: i64 });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportConvoyReservation {
    pub priority: i32,
    pub quantity: i32,
}
value_struct!(TransportConvoyReservation {
    priority: i32,
    quantity: i32
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportConvoyVariant {
    pub variant: u64,
    /// Native signed cached stats +198/+1A0/+1A8/+208. None refuses only
    /// that original leaf; F97BF0 demands visibility alone.
    pub surface_detection: Option<i64>,
    pub submarine_detection: Option<i64>,
    pub visibility: Option<i64>,
    pub speed: Option<i64>,
}
value_struct!(TransportConvoyVariant {
    variant: u64, surface_detection: Option<i64>, submarine_detection: Option<i64>, visibility: Option<i64>, speed: Option<i64>,
});
opaque_value!(Option<TransportConvoys>);

#[cfg(test)]
#[path = "transport_convoys_tests.rs"]
mod tests;
