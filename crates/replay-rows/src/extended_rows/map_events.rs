//! Lossless codec for the hourly map event interval.
use super::*;

opaque_value!([MapEventFamilyHealth; MapEventFamily::COUNT]);
opaque_value!(Vec<MapEvent>);

typed_codec!(MapEventsRow, 0, |_row| 0, |_key| Self::default(), {
    0 => (epoch): U64,
    1 => (from_hour): I32,
    2 => (to_hour): I32,
    3 => (first_sequence): U64,
    4 => (next_sequence): U64,
    5 => (health): Opaque,
    6 => (events): Opaque,
}, |row| row.validate());
