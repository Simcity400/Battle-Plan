//! Particle-editor function metadata; method ownership is unproven unless stated.
//! Sources below are under portable_replay/native/engine_evidence/.

/// Subsystem comparator RVA; src/xmap20_g7_census_adjudication.rs:325 and func_142080120.decompile.c compare record arguments 2 and 3, including position Y and hidden.
pub const COMPARE_SUBSYSTEMS_RVA: usize = 0x0208_0120;
/// Editor field-registration RVA; src/xmap20_g7_census_adjudication.rs:331 and func_142085a00.decompile.c select a subsystem and bind named editor fields.
pub const REGISTER_FIELDS_RVA: usize = 0x0208_5a00;
/// Attribute comparison helper RVA; src/xmap20_g7_census_adjudication.rs:326 and func_142080120.decompile.c:61 pass the two subsystem +0x3b0 members; original member type unproven.
pub const COMPARE_ATTRIBUTE_RVA: usize = 0x0208_09c0;
/// Attribute editor callback RVA; src/xmap20_g7_census_adjudication.rs:332 and func_142085a00.decompile.c:109 register this callback for position Y.
pub const ATTRIBUTE_CALLBACK_RVA: usize = 0x0207_5940;
/// Attribute registration helper RVA; src/xmap20_g7_census_adjudication.rs:332 and func_142085a00.decompile.c:109 pass the name, member address and callback to it.
pub const REGISTER_ATTRIBUTE_RVA: usize = 0x024a_2a90;
/// Hidden/boolean field registration helper RVA; src/xmap20_g7_census_adjudication.rs:334 and func_142085a00.decompile.c:70 pass the named hidden byte to it.
pub const REGISTER_HIDDEN_RVA: usize = 0x024a_2f70;
pub const HIDDEN_AUXILIARY_DATA_RVA: usize = 0x0270_073d;
