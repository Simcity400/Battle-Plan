//! DLC enumeration output ABI. This buffer size does not establish the
//! size of every array container.

/// Enumerated string element pointer.
pub const DATA: usize = 0;
/// Output capacity, u32, at +8.
pub const CAPACITY: usize = 8;
/// Output count, u32, at +0xc.
pub const COUNT: usize = 0xc;
/// Allocator pointer initialized before enumeration.
pub const ALLOCATOR: usize = 0x10;
/// Allocator auxiliary qword initialized from its companion global.
pub const ALLOCATOR_AUX: usize = 0x18;
/// Output ABI byte size on x64.
pub const SIZE: usize = 0x20;
