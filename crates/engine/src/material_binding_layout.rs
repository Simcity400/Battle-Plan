//! Entity material-binding record paired with a mesh draw.
//! These presentation layouts do not enable a recorded table or hook.

/// Material-binding row stride.
pub const STRIDE: usize = 0x60;
/// Signed i32 shader index for each pass.
pub const PASS_SHADER_INDICES: usize = 0x0c;
/// Shader-index stride.
pub const SHADER_INDEX_STRIDE: usize = 0x04;
/// Variant-path signed i32 shader index.
pub const VARIANT_SHADER_INDEX: usize = 0x2c;
