//! CContextTrigger switches the event scope before checking its child triggers.

/// Event-target descriptor pointer.
pub const EVENT_TARGET: usize = 0x58;
/// Scope-switching check; invalid event targets return false.
pub const CHECK_RVA: usize = 0x0138_f160;
/// Accepted-scope mask.
pub const ACCEPTED_SCOPE_MASK: u32 = 0x3ffe;

/// Constructor installing the vtable and separately allocating its target descriptor.
pub const CONSTRUCTOR_RVA: usize = 0x0138_c3a0;
/// Accepted-scope mask implementation.
pub const ACCEPTED_SCOPE_RVA: usize = 0x003e_e800;
