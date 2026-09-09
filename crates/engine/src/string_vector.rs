//! Engine vector operations used by the CMapApplication MSVC-string queues.
//! Original container template identity is unproven; initializer is shared with other element types.
//! Sources below are under portable_replay/native/engine_evidence/.

/// Header initializer RVA; src/xmap20_g7_census_adjudication.rs:234 and func_14011da30.decompile.c clear the first two qwords and store the allocator at +0x10; func_14208c420 applies it to all three queues.
pub const INITIALIZE_RVA: usize = 0x0011_da30;
/// String-vector append RVA; src/xmap20_particle_reload_queue.rs:205 and func_14208e370.decompile.c:60-63 append a constructed MSVC string and index the last 0x20-byte entry.
pub const PUSH_STRING_RVA: usize = 0x0011_d230;
/// String-range teardown RVA; src/xmap20_particle_reload_queue.rs:207 and func_14208f4b0.decompile.c pass (vector, 0, count) before clearing count; the old runtime DLC vector uses the same helper.
pub const DESTROY_RANGE_RVA: usize = 0x0012_07d0;
