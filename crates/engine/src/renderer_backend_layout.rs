//! Backend reached through backbuffer renderer +0x268.
//! Presentation metadata; no recorded table or hook is enabled.

/// D3D device pointer.
pub const DEVICE: usize = 0x20;
/// Swapchain pointer.
pub const SWAPCHAIN: usize = 0x30;
/// Backbuffer render-target-view pointer.
pub const BACKBUFFER_VIEW: usize = 0x40;
/// Readiness, u32; one is required after recreation.
pub const READY: usize = 0x25c;
