//! Video-device presentation state.

/// Current video-device pointer.
pub const CURRENT_VIDEO_DEVICE_RVA: usize = 0x0343_7c68;
/// Display count, i32.
pub const NUM_DISPLAYS: usize = 0x2b8;
/// Contiguous display-array pointer.
pub const DISPLAYS: usize = 0x2c0;
/// Display-mode-setting flag, i32; skips mode replacement when set.
pub const SETTING_DISPLAY_MODE: usize = 0x2e8;
/// Monitor initialization enumerating primary before secondary and checking display count.
pub const INITIALIZE_MODES_RVA: usize = 0x0210_7fb0;
/// Mode reader copying DEVMODEW into the supplied display-mode record.
pub const GET_DISPLAY_MODE_RVA: usize = 0x0210_7bb0;
/// Mode updater replacing current dimensions from device caps.
pub const UPDATE_DISPLAY_MODE_RVA: usize = 0x0210_8200;
