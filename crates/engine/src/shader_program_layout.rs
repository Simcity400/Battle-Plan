//! Shader-program row used by effect lookup and entity rendering.
//! Presentation metadata only.

/// Shader-program row stride.
pub const STRIDE: usize = 0x38;
/// Fallback shader-program object for negative material indices.
pub const ENTITY_SHADER_FALLBACK_RVA: usize = 0x035C_1970;
