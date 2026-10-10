//! Render target passed directly to the clear operation.
//! Presentation metadata; no recorded table or hook is enabled.

/// Color render-target-view pointer, used for clear bit 0.
pub const COLOR_VIEW: usize = 0x10;
/// Depth/stencil-view pointer, used for clear bits 1 and 2.
pub const DEPTH_STENCIL_VIEW: usize = 0x18;
/// Target clear routine.
pub const RENDER_TARGET_CLEAR_RVA: u32 = 0x023F_CA30;
/// Exact target-clear entry sentinel.
pub const RENDER_TARGET_CLEAR_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x40, 0x49,
];
