//! Manpower-value entry used by division training, deployment and garrison walks.
//!
//! These readers use the same eight-byte layout with an i32 count at +4.
//! The occupation walk interprets +0 as a source-country link. That meaning
//! in other instances and the concrete C++ entry type remain unproved.

/// Manpower-entry stride; division_training_inputs.rs:177-180 and deployment offsets.rs:455 both advance eight bytes.
pub const STRIDE: usize = 0x08;
/// Manpower-entry source-country link, u32; D66 attach_log.rs:149059-149098 reads the first CStateGarrisonData assigned-manpower row at +0.
pub const SOURCE_COUNTRY_LINK: usize = 0x00;
/// Manpower-entry count, i32; division_training_inputs.rs:178-189 and read_deployment_manpower_container read +4 with different sum policies.
pub const COUNT: usize = 0x04;
