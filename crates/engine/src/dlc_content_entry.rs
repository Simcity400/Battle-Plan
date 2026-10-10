//! Inline DLC/mod descriptor payload.

/// Content display-name std::string.
pub const NAME: usize = 8;
/// Content diagnostic-name std::string.
pub const LOGGED_NAME: usize = 0x28;
/// Descriptor std::string.
pub const DESCRIPTOR: usize = 0x48;
/// Descriptor folder-path std::string.
pub const PATH: usize = 0x68;
/// Descriptor archive-path std::string.
pub const ARCHIVE: usize = 0x88;
/// Dependency-string CPdxArray.
pub const DEPENDENCIES: usize = 0xa8;
/// replace_path-string CPdxArray.
pub const REPLACE_PATHS: usize = 0xc0;
/// Base-directory std::string.
pub const BASE_DIRECTORY: usize = 0x118;
/// Content-kind i32 discriminator.
pub const KIND: usize = 0x248;
/// DLC kind value.
pub const KIND_DLC: i32 = 1;
/// DLC report-membership byte.
pub const DLC_REPORTED_ACTIVE: usize = 0x298;
