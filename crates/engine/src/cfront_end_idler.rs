//! CFrontEndIdler fields used by the excluded camera presentation capture.

/// CFrontEndIdler vtable.
pub const VTABLE_RVA: usize = 0x0293_3710;
/// Embedded frontend camera root; owner plus displacement must equal root.
pub const CAMERA_ROOT: usize = 0x610;
/// CFrontEndIdler camera event-list pointer slot +0x5f0.
pub const CAMERA_EVENT_LIST: usize = 0x5f0;
/// Frontend action-source pointer, required nonnull for event production.
pub const CAMERA_ACTION_SOURCE: usize = 0x508;
/// CFrontEndIdler cursor-source pointer +0x500.
pub const CAMERA_CURSOR_SOURCE: usize = 0x500;
