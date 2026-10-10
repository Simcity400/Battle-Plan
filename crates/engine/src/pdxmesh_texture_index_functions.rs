//! Pdxmesh texture-index layouts; descriptors do not traverse files or enable capture.

/// Model-directory enumeration entry.
pub const QUEUE_DIRECTORY_SCAN_RVA: usize = 0x0225_1650;
/// Recursive basename indexing with exact extension filtering and stored spelling.
pub const BUILD_RECURSIVE_RVA: usize = 0x0225_1f00;
/// Whole authored-path hashing; only an exact hit replaces the provisional path.
pub const LOOKUP_TEXTURE_RVA: usize = 0x0225_1230;
/// Model-directory bytes read from the installed executable.
pub const ROOT: &str = "gfx/models";
/// Texture suffixes tested in exact stored order.
pub const EXTENSIONS: [&str; 2] = ["dds", "tga"];
