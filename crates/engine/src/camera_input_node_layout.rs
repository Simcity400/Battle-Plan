//! Unidentified linked camera input node from the excluded presentation hooks.
//! Traversal refuses cycles and unreadable nodes. The concrete class and
//! lifetime are unknown; no safe hourly traversal has been established.

/// Embedded event; its observed bytes are copied unchanged.
pub const EVENT: usize = 0x48;
/// Node kind, i32, read independently of the embedded event type.
pub const KIND: usize = 0x7c;
/// Consumed byte, read before and after event production.
pub const CONSUMED: usize = 0x80;
/// Next pointer, followed until null with cycle detection.
pub const NEXT: usize = 0x90;
