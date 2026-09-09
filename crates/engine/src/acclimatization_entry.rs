//! Native category/weight entry in the division acclimatization vector.

/// Division acclimatization-entry stride; division_presentation_inputs.rs:949-951 walks 16-byte entries.
pub const STRIDE: usize = 0x10;
/// Division acclimatization-entry registered category pointer; division_presentation_inputs.rs:952.
pub const CATEGORY: usize = 0x00;
/// Division acclimatization-entry raw i64 weight; division_presentation_inputs.rs:979.
pub const WEIGHT: usize = 0x08;
