//! Sampler-slot owner; shader-program row identity is not established.
//! Later duplicate physical slots replace earlier entries. Presentation
//! metadata only.

/// Sampler-wrapper pointer array indexed by descriptor physical slot.
pub const SAMPLERS: usize = 0x20;
/// Sampler-array capacity, i32.
pub const CAPACITY: usize = 0x28;
/// Sampler-array count, i32; missing slots are zero-filled.
pub const COUNT: usize = 0x2c;
/// Sampler-array allocator pointer.
pub const ALLOCATOR: usize = 0x30;
/// Sampler-wrapper pointer stride.
pub const ENTRY_STRIDE: usize = 8;
/// Declaration lowering creates wrappers and assigns their physical slots.
pub const POPULATE_RVA: usize = 0x0234_5450;
