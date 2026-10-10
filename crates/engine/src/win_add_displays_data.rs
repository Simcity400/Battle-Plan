//! Monitor-enumeration callback context.

/// Video-device pointer supplied by the enumeration caller.
pub const VIDEO_DEVICE: usize = 0;
/// Send-event flag, i32, initialized to zero and forwarded with its low 32 bits.
pub const SEND_EVENT: usize = 8;
/// Primary-monitor selection flag, i32, set to one then zero and compared with monitor flags.
pub const WANT_PRIMARY: usize = 0x0c;
/// Monitor callback reading this context through its fourth Win32 argument.
pub const ADD_DISPLAYS_CALLBACK_RVA: usize = 0x0210_76f0;
