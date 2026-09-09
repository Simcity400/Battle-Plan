//! NAcclimatization::CData embedded division state; hourly consistency remains unproved.

/// NAcclimatization::CData vtable; division_presentation_inputs.rs:913-916 authenticates the embedded object.
pub const VTABLE_RVA: usize = 0x027c_7b30;
/// NAcclimatization::CData category/weight vector data; division_presentation_inputs.rs:924.
pub const DATA: usize = 0x08;
/// NAcclimatization::CData u32 vector capacity; division_presentation_inputs.rs:925.
pub const CAPACITY: usize = 0x10;
/// NAcclimatization::CData u32 vector count; division_presentation_inputs.rs:926.
pub const COUNT: usize = 0x14;
/// NAcclimatization::CData maximum, i64 engine raw value; division_presentation_inputs.rs:934.
pub const MAXIMUM: usize = 0x38;
