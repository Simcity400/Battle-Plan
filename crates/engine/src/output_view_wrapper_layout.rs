//! Nested output views reached through a shadow-texture target; presentation layouts only.

/// Render-target-view pointer.
pub const RENDER_TARGET_VIEW: usize = 0x00;
/// Optional depth/stencil-view pointer.
pub const DEPTH_STENCIL_VIEW: usize = 0x08;
/// Optional depth-resource pointer.
pub const DEPTH_RESOURCE: usize = 0x10;
