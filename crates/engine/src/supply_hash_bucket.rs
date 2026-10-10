//! Province-to-i64 supply hash bucket. The disrupted-supply bucket uses
//! a separate two-word-key layout.

/// Bucket stride.
pub const STRIDE: usize = 0x18;
/// Probe marker, u8; zero is empty and 0xff terminal.
pub const MARKER: usize = 0x04;
/// Province key, u32.
pub const PROVINCE_ID: usize = 0x08;
/// Supply value, i64, without narrowing.
pub const VALUE: usize = 0x10;
