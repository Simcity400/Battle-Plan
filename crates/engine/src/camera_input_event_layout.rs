//! Unidentified input event embedded in a camera input node.
//!
//! Source: `production_render_submit_live/src/a729_hook.rs:6572–6601`.
//! These are engine byte offsets, not offsets into the old replay row.

/// Embedded event's 16-byte observed span; capture_camera_input_nodes preserves it verbatim.
pub const OBSERVED_BYTES: usize = 16;
/// Embedded event type i32 at +0x04; old capture decodes bytes 4..8 as little-endian signed integer.
pub const EVENT_TYPE: usize = 0x04;
/// Embedded event signed delta i32 at +0x08; old capture decodes bytes 8..12 without float conversion.
pub const SIGNED_DELTA: usize = 0x08;
