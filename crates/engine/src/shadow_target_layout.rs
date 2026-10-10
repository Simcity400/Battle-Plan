//! Shadow texture target reached through the shadow owner, separate from
//! the direct clear-operation receiver. Presentation metadata only.

/// Shader-resource-view pointer.
pub const SHADER_RESOURCE_VIEW: usize = 0x00;
/// Texture-resource pointer.
pub const RESOURCE: usize = 0x08;
/// Output-view wrapper pointer, distinct from direct clear target +0x10.
pub const OUTPUT_VIEW_WRAPPER: usize = 0x10;
