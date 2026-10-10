//! Video-display presentation record.

/// Display-array stride.
pub const STRIDE: usize = 0x68;
/// Duplicated UTF-8 display-name pointer.
pub const NAME: usize = 0;
/// Display-mode count, i32.
pub const NUM_DISPLAY_MODES: usize = 0x0c;
/// Display-mode array pointer.
pub const DISPLAY_MODES: usize = 0x10;
/// Inline desktop mode.
pub const DESKTOP_MODE: usize = 0x18;
/// Inline current mode.
pub const CURRENT_MODE: usize = 0x30;
/// Orientation, i32; unknown or unchanged values are rejected before storing.
pub const ORIENTATION: usize = 0x48;
/// Video-device backpointer.
pub const DEVICE: usize = 0x58;
/// Monitor driver-data pointer used for device-name and handle updates.
pub const DRIVER_DATA: usize = 0x60;
/// Current-mode setter copying a supplied record to +0x30.
pub const SET_CURRENT_MODE_RVA: usize = 0x020d_8170;
/// Desktop-mode setter copying a supplied record to +0x18.
pub const SET_DESKTOP_MODE_RVA: usize = 0x020d_8180;
