//! CMapApplication asset-reload queues for hoi4.exe 1.19.2.0.a729.
//! The constructor stamps CMapApplication; these presentation queues are not campaign state.
//! Sources below are under portable_replay/native/engine_evidence/.

/// Allocation bytes; artifacts/func_142089670.decompile.c:1189,1204 allocates 0x3f0 then calls the class constructor.
pub const SIZE: usize = 0x3f0;
/// Constructor RVA; src/xmap20_g7_census_adjudication.rs:226 and func_14208c420.decompile.c stamp CMapApplication vtables.
pub const CONSTRUCTOR_RVA: usize = 0x0208_c420;
/// Startup RVA; src/xmap20_particle_queue_producer.rs:267 and func_142089670.decompile.c confirm allocation and reload-callback registration.
pub const STARTUP_RVA: usize = 0x0208_9670;
/// Mesh std::string vector; func_14208c420.decompile.c initializes qword index 0x70, and func_14208e370.decompile.c:90 appends here.
pub const MESH_RELOAD_QUEUE: usize = 0x380;
/// Mesh vector count i32; src/xmap20_particle_reload_queue.rs:131 and func_14208f4b0.decompile.c:12-13 destroy then clear it.
pub const MESH_RELOAD_COUNT: usize = 0x38c;
/// Animation std::string vector; func_14208c420.decompile.c initializes qword index 0x73, and func_14208e370.decompile.c:75 appends here.
pub const ANIMATION_RELOAD_QUEUE: usize = 0x398;
/// Animation vector count i32; src/xmap20_particle_reload_queue.rs:132 and func_14208f4b0.decompile.c:14-15 destroy then clear it.
pub const ANIMATION_RELOAD_COUNT: usize = 0x3a4;
/// Particle std::string vector; src/xmap20_g7_census_adjudication.rs:235 qword index 0x76 is byte 0x3b0, confirmed by func_14208e370.decompile.c:61.
pub const PARTICLE_RELOAD_QUEUE: usize = 0x3b0;
/// Particle vector count i32; src/xmap20_particle_reload_queue.rs:138 and func_14208f4b0.decompile.c:20-21 destroy then clear it.
pub const PARTICLE_RELOAD_COUNT: usize = 0x3bc;
pub const TRAILING_STRING: usize = 0x3c8;
/// Final qword, meaning unproven; func_14208c420.decompile.c stores param_4 at index 0x7d, ending exactly at the 0x3f0 allocation.
pub const FINAL_ARGUMENT: usize = 0x3e8;
/// Reload callback RVA; src/xmap20_particle_reload_queue.rs:120 and func_14208e370.decompile.c select mesh/anim/particle vector from the filename extension.
pub const APPEND_RELOAD_PATH_RVA: usize = 0x0208_e370;
/// Per-frame drain RVA; src/xmap20_particle_reload_queue.rs:123 and func_14208f4b0.decompile.c consume and clear the three vectors.
pub const DRAIN_RELOAD_QUEUES_RVA: usize = 0x0208_f4b0;
/// Particle consumer RVA; src/xmap20_particle_reload_queue.rs:145 and func_14208f4b0.decompile.c:19 pass this application and its particle vector.
pub const PARTICLE_RELOAD_CONSUMER_RVA: usize = 0x0208_f200;
