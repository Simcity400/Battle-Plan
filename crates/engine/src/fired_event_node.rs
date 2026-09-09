//! One node in the engine's fired-event hash set; exact node type unknown.

/// Fired-event node CEvent pointer; old fired_event_popups_live/a729_bridge.rs:878 reads it.
pub const EVENT: usize = 0x00;
/// Fired-event node next pointer; old a729_bridge.rs:886 follows the chain.
pub const NEXT: usize = 0x08;
