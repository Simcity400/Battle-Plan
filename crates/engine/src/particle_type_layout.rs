//! Particle runtime type descriptor with matched shader and atlas arrays; presentation layouts only.

/// Program-index array pointer.
pub const PROGRAM_INDICES: usize = 0x48;
/// Program-index u32 count.
pub const PROGRAM_COUNT: usize = 0x54;
/// Atlas-row array pointer.
pub const ATLAS_ROWS: usize = 0x60;
/// Atlas-row u32 count, required to match program count.
pub const ATLAS_COUNT: usize = 0x6c;
