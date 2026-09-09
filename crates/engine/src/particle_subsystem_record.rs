//! Authored particle subsystem record selected by the particle editor; original class unproven.
//! This record is distinct from the runtime type descriptor and registration definition.
//! Sources below are under portable_replay/native/engine_evidence/.

/// Emitter position-Y attribute; src/xmap20_g7_census_adjudication.rs:332 and func_142085a00.decompile.c:109 register pe_emitter_attr_posy at this offset; func_142080120 compares the same member.
pub const POSITION_Y_ATTRIBUTE: usize = 0x3b0;
/// Emitter position-Y variation attribute; func_142085a00.decompile.c:111 names pe_emitter_attr_posy_var, and src/xmap20_g7_census_adjudication.rs:344-345 / func_14232ecc0.decompile.c:64 compile the base/variation pair together.
pub const POSITION_Y_VARIATION: usize = 0x3f8;
/// Hidden flag byte; src/xmap20_g7_census_adjudication.rs:334 and func_142085a00.decompile.c:70 register pe_subsystem_hidden; func_142080120.decompile.c:42 compares both records' bytes.
pub const HIDDEN: usize = 0x74e;
