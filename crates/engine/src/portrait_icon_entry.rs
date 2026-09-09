//!
//! Source: `extractor/native/lanes/diplomacy_intel_operatives_live/src/a30_operative_portrait_join.rs`.
//! This resolved presentation object is excluded from hourly capture.

/// Icon entry availability byte; confirmed by the old exact-build icon-entry export and read_icon_entry (source:530).
pub const AVAILABLE: usize = 0;
/// Icon entry identity discriminator, native i32; confirmed by read_icon_entry (source:533).
pub const IDENTITY_KIND: usize = 4;
/// Icon entry discriminator for an authored GFX name; confirmed by read_icon_entry's engine representation (source:534).
pub const GFX_NAME: i32 = 0;
/// Icon entry discriminator for a resolved file path; confirmed by read_icon_entry's engine representation (source:535).
pub const FILE_PATH: i32 = 1;
/// Icon entry identity MSVC string; confirmed by read_icon_entry (source:538).
pub const IDENTITY: usize = 8;
