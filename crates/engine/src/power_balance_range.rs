//! Power-balance range definition; the C++ class name is unconfirmed.
//!
//! The old reader captures the engine-selected range, rather than inferring one
//! from thresholds where ranges may share a boundary.

/// Power-balance range identity MSVC string; archived D74 copy_once reads the active range at +0x718.
pub const IDENTITY: usize = 0x718;
