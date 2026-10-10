//! Windows display-mode wrapper with an inline DEVMODEW payload.

/// Wrapper allocation size.
pub const SIZE: usize = 0xdc;
/// Inline DEVMODEW at allocation start.
pub const DEVICE_MODE: usize = 0;
