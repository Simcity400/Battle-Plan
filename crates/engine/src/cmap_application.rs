//! CMapApplication asset-reload queues. These presentation queues are
//! not campaign state.

/// Allocation bytes.
pub const SIZE: usize = 0x3f0;
/// Constructor RVA.
pub const CONSTRUCTOR_RVA: usize = 0x0208_c420;
/// Startup RVA.
pub const STARTUP_RVA: usize = 0x0208_9670;
/// Mesh std::string vector.
pub const MESH_RELOAD_QUEUE: usize = 0x380;
/// Mesh vector count i32.
pub const MESH_RELOAD_COUNT: usize = 0x38c;
/// Animation std::string vector.
pub const ANIMATION_RELOAD_QUEUE: usize = 0x398;
/// Animation vector count i32.
pub const ANIMATION_RELOAD_COUNT: usize = 0x3a4;
/// Particle std::string vector.
pub const PARTICLE_RELOAD_QUEUE: usize = 0x3b0;
/// Particle vector count i32.
pub const PARTICLE_RELOAD_COUNT: usize = 0x3bc;
/// Trailing std::string.
pub const TRAILING_STRING: usize = 0x3c8;
/// Final qword, meaning unproven.
pub const FINAL_ARGUMENT: usize = 0x3e8;
/// Reload callback RVA.
pub const APPEND_RELOAD_PATH_RVA: usize = 0x0208_e370;
/// Per-frame drain RVA.
pub const DRAIN_RELOAD_QUEUES_RVA: usize = 0x0208_f4b0;
/// Particle consumer RVA.
pub const PARTICLE_RELOAD_CONSUMER_RVA: usize = 0x0208_f200;
