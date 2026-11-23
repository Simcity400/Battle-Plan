use super::*;

impl OpaqueValue for MapEventFamilyHealth {}
impl OpaqueValue for MapEvent {}

typed_codec!(MapEventsRow, 0, |_row| 0, |_key| Self::default(), {
    0 => (epoch): U64,
    1 => (from_hour): I32,
    2 => (to_hour): I32,
    3 => (first_sequence): U64,
    4 => (next_sequence): U64,
    5 => (health): Opaque,
    6 => (events): Opaque,
}, |row| row.validate());
