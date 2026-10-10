//! Windows monitor presentation record.

/// Monitor driver-data allocation size.
pub const SIZE: usize = 0x50;
/// Device-name UTF-16 buffer at allocation start.
pub const DEVICE_NAME: usize = 0;
/// Device-name byte span: 32 UTF-16 code units.
pub const DEVICE_NAME_BYTES: usize = 0x40;
/// Monitor handle, HMONITOR.
pub const MONITOR_HANDLE: usize = 0x40;
/// Validity, i32; successfully registered monitors use one.
pub const IS_VALID: usize = 0x48;
