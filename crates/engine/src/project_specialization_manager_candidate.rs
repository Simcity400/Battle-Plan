//! Candidate specialization manager; no vtable or full layout is established.

/// Candidate-manager getter.
pub const GETTER_RVA: usize = 0x0049_4ff0;
/// Manager-pointer global.
pub const GLOBAL_POINTER_RVA: usize = 0x0331_6c98;
/// Definition-token enumeration in stored vector order.
pub const ENUMERATE_TOKENS_RVA: usize = 0x00aa_bad0;
/// Definition-pointer array.
pub const DEFINITIONS: usize = 0x60;
/// Signed i32 definition count.
pub const DEFINITIONS_COUNT: usize = 0x6c;
/// Definition-pointer element stride.
pub const DEFINITION_POINTER_STRIDE: usize = 0x08;
