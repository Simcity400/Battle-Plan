//! Anonymous anchor-selector hash bucket; layout name is descriptive, not RTTI.

/// Anchor-selector bucket stride; old attach_log.rs:140728 and select_secondary advances without wrapping.
pub const STRIDE: usize = 0x18;
/// Probe marker, u8; old attach_log.rs:140742 and select_secondary consumes bucket+4.
pub const MARKER: usize = 0x04;
/// Province selector key, u32; old attach_log.rs:140743 and select_secondary consumes bucket+8.
pub const KEY: usize = 0x08;
/// Selector value, native u64 tested for nonzero; old attach_log.rs:140744 and select_secondary consumes bucket+0x10.
pub const VALUE: usize = 0x10;
