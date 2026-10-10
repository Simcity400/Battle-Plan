//! Unidentified input event embedded in a camera input node.
//! Offsets are relative to the embedded event.

/// Embedded event's observed byte span, preserved verbatim.
pub const OBSERVED_BYTES: usize = 16;
/// Event type, little-endian i32.
pub const EVENT_TYPE: usize = 0x04;
/// Signed event delta, little-endian i32 without float conversion.
pub const SIGNED_DELTA: usize = 0x08;
