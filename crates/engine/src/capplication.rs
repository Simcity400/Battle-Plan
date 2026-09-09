//! CApplication frame counter and image-global publication sites in a729.
//! Names and ownership are retained from map_viewer/src/application_frame.rs;
//! this metadata does not implement the old viewer's frame-lifetime policy.

/// CApplication constructor; application_frame.rs:1-5 identifies the native construction entry.
pub const CONSTRUCT_RVA: usize = 0x0221_64e0;
/// CApplication frame entry/exit; application_frame.rs:4 pins the native counter's invocation custody.
pub const FRAME_RVA: usize = 0x0221_9f70;
/// CApplication counter u64; application_frame.rs:78 describes native Application+330 custody, distinct from the viewer wrapper.
pub const FRAME_COUNTER: usize = 0x330;
/// Image-global u32 published stamp; application_frame.rs:5,44 identifies process-wide native publication custody.
pub const FRAME_STAMP_RVA: usize = 0x0346_8304;
/// Stamp publication instruction; application_frame.rs:212 confirms publication occurs before event dispatch.
pub const PUBLISH_FRAME_STAMP_SITE_RVA: usize = 0x0221_a036;
