//! Resolved portrait icon-entry presentation object, excluded from hourly capture.

/// Icon-entry availability byte.
pub const AVAILABLE: usize = 0;
/// Icon-entry signed i32 identity discriminator.
pub const IDENTITY_KIND: usize = 4;
/// Authored-sprite identity discriminator.
pub const GFX_NAME: i32 = 0;
/// Resolved-file-path identity discriminator.
pub const FILE_PATH: i32 = 1;
/// Icon-entry identity MSVC string.
pub const IDENTITY: usize = 8;
