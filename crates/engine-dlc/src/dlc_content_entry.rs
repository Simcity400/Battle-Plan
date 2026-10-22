//! Inline DLC/mod descriptor payload.

/// Content display-name std::string.
pub const NAME: usize = 8;
/// Descriptor std::string.
pub const DESCRIPTOR: usize = 0x48;
/// Descriptor folder-path std::string.
pub const PATH: usize = 0x68;
/// Descriptor archive-path std::string.
pub const ARCHIVE: usize = 0x88;
/// Content-kind i32 discriminator.
pub const KIND: usize = 0x248;
/// DLC report-membership byte.
pub const DLC_REPORTED_ACTIVE: usize = 0x298;
