//! Rasterizer descriptor retained for presentation. No reader or hook is
//! installed. Antialiasing and API fixed depth clipping are separate from
//! these fields. Offsets are bytes.

/// Descriptor initializer.
pub const INITIALIZE_RVA: usize = 0x023f_7180;
/// Descriptor copy and lowering routine.
pub const LOWER_RVA: usize = 0x023f_d2f0;
/// Cull-mode translator.
pub const LOWER_CULL_MODE_RVA: usize = 0x0241_10d0;
/// Descriptor byte extent copied into the wrapper.
pub const COPIED_BYTES: usize = 0x1c;
/// Fill mode, u32; exactly two selects solid fill.
pub const FILL_MODE: usize = 0x00;
/// Cull mode, u32.
pub const CULL_MODE: usize = 0x04;
/// Front-counterclockwise byte.
pub const FRONT_COUNTER_CLOCKWISE: usize = 0x08;
/// Depth bias, i32.
pub const DEPTH_BIAS: usize = 0x0c;
/// Depth-bias-clamp f32 bits, copied unchanged.
pub const DEPTH_BIAS_CLAMP: usize = 0x10;
/// Slope-scaled-depth-bias f32 bits, copied unchanged.
pub const SLOPE_SCALED_DEPTH_BIAS: usize = 0x14;
/// Scissor-enable byte.
pub const SCISSOR_ENABLED: usize = 0x18;
/// Additional byte initialized to zero; copied but not used during lowering.
pub const UNCLASSIFIED_19: usize = 0x19;
