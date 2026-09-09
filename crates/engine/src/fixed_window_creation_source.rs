//! Source object passed to the CFixedWindow construction wrapper; original class unproven.
//! This source owns a linked list at +0x3b0 and is distinct from CMapApplication.
//! Sources below are under portable_replay/native/engine_evidence/.

/// CFixedWindow creation wrapper RVA; src/xmap20_particle_queue_producer.rs:323 and func_1422e3280.decompile.c:36-40 pass this source object's fields to the constructor.
pub const CREATE_WINDOW_RVA: usize = 0x022e_3280;
pub const COPIED_STRING_LIST: usize = 0x3b0;
/// Preceding source member, semantics unproven; src/xmap20_g7_census_adjudication.rs:338 and func_1422e22b0.decompile.c:58 pass it as constructor argument 22, matching func_1422e3280's source schema.
pub const MEMBER_398: usize = 0x398;
/// Source scalar u32, semantics unproven; src/xmap20_g7_census_adjudication.rs:339 and func_1422e22b0.decompile.c:58 pass it as constructor argument 25, matching func_1422e3280's source schema.
pub const SCALAR_414: usize = 0x414;
