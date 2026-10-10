//! Particle-builder packet for normal and ribbon arrays; presentation layouts only.

/// Particle world matrix, sixteen f32 words.
pub const WORLD_TRANSFORM: usize = 0x00;
/// Live particle u32 count.
pub const LIVE_COUNT: usize = 0x40;
/// GPU instance-data pointer.
pub const RECORDS: usize = 0x48;
/// GPU instance u32 stride.
pub const RECORD_STRIDE: usize = 0x50;
/// Particle u32 capacity.
pub const CAPACITY: usize = 0x54;
/// Elapsed f32 time.
pub const ELAPSED: usize = 0x58;
/// Packet u16 flags.
pub const FLAGS: usize = 0x5c;
/// Subsystem u32 selector for shader and atlas rows.
pub const SUBSYSTEM_SELECTOR: usize = 0x60;
/// Particle packet size.
pub const PACKET_BYTES: usize = 0x70;
/// GPU instance-row stride.
pub const GPU_INSTANCE_STRIDE: u32 = 0x30;
