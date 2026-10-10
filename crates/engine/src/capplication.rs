//! CApplication frame counter and process-wide stamp publication sites.
//! This metadata does not establish a viewer frame-lifetime policy.

/// Application constructor.
pub const CONSTRUCT_RVA: usize = 0x0221_64e0;
/// Frame entry/exit owning counter updates.
pub const FRAME_RVA: usize = 0x0221_9f70;
/// Application frame counter, u64.
pub const FRAME_COUNTER: usize = 0x330;
/// Process-wide published frame stamp, u32.
pub const FRAME_STAMP_RVA: usize = 0x0346_8304;
/// Frame-stamp publication instruction, reached before event dispatch.
pub const PUBLISH_FRAME_STAMP_SITE_RVA: usize = 0x0221_a036;
