//! Unidentified linked camera input node from the excluded presentation hooks.
//!
//! Source: `production_render_submit_live/src/a729_hook.rs:6561–6610,6690–6701`.
//! The old walk rejected cycles and unreadable nodes. Actual class and lifetime
//! are unknown; no safe hourly traversal has been established.

/// Input node embedded 16-byte event at +0x48; capture_camera_input_nodes copies these bytes unchanged.
pub const EVENT: usize = 0x48;
/// Input node kind i32 at +0x7c; capture_camera_input_nodes reads it independently of the embedded event type.
pub const KIND: usize = 0x7c;
/// Input node consumed u8 at +0x80; old hook reads it before and after the event producer.
pub const CONSUMED: usize = 0x80;
/// Input node next pointer at +0x90; capture_camera_input_nodes follows it until null with cycle detection.
pub const NEXT: usize = 0x90;
