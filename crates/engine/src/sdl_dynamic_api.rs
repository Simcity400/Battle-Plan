//! SDL dynamic-API and event-name metadata.
//! These sites do not establish object ownership.

/// Event-state thunk called with event 0x1000 and enable value one.
pub const EVENT_STATE_RVA: usize = 0x020a_16c0;
/// Dynamic-API initializer preceding indirect dispatch.
pub const INITIALIZE_RVA: usize = 0x020a_8760;
/// Event-name lookup.
pub const EVENT_NAME_RVA: usize = 0x020c_3060;
/// File-drop event ID.
pub const DROP_FILE_EVENT: u32 = 0x1000;
/// One-argument event-value thunk; disposal semantics are not established.
pub const RETURN_EVENT_VALUE_RVA: usize = 0x020a_42b0;
/// Dynamic-API function-pointer slot.
pub const RETURN_EVENT_VALUE_SLOT_RVA: usize = 0x0309_f190;
