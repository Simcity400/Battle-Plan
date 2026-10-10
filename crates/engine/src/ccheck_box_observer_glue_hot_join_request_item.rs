//! CCheckBoxObserverGlue<CHotJoinRequestItem> presentation callback layout.

/// Hot-join checkbox observer vtable used by both embedded observers.
pub const VTABLE_RVA: usize = 0x029e_a238;
/// Owning hot-join item pointer.
pub const OWNER: usize = 0x08;
/// Observer callback function pointer.
pub const CALLBACK: usize = 0x10;
