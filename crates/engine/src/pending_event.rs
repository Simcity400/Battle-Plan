//! Pending-event row with actor and copied scope, no dispatch recipient; scope-country zero means none.

/// Pending-event row stride.
pub const STRIDE: usize = 0x38;
/// Pending-event u32 instance ID.
pub const INSTANCE_ID: usize = 0x00;
/// Event pointer.
pub const EVENT: usize = 0x08;
/// Copied event-scope pointer.
pub const SCOPE: usize = 0x10;
/// Actor country u32 link.
pub const ACTOR_TAG: usize = 0x18;
/// Date primary-vtable pointer.
pub const DATE_VTABLE: usize = 0x20;
/// Absolute signed i32 due hour.
pub const DUE_TICK: usize = 0x28;
/// Date secondary persistent-base vtable pointer.
pub const DATE_PERSISTENT_VTABLE: usize = 0x30;
