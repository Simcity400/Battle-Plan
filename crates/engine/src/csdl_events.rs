//! Event singleton and drop-file callback layout.

/// Singleton initializer allocating and constructing the dispatcher.
pub const INITIALIZE_SINGLETON_RVA: usize = 0x0223_91d0;
/// Singleton getter returning the initialized global pointer.
pub const GET_SINGLETON_RVA: usize = 0x0223_91c0;
/// Singleton pointer populated by initialization.
pub const SINGLETON_RVA: usize = 0x0343_a208;
/// Dispatcher constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0234_ccf0;
/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_bb98;
/// Callback-registration virtual-dispatch byte offset.
pub const REGISTER_DROP_CALLBACK_SLOT: usize = 0x18;
/// Event-dispatch virtual-dispatch byte offset.
pub const DISPATCH_EVENTS_SLOT: usize = 0x08;
/// Callback registration enabling drop-file events and storing owner/callback.
pub const REGISTER_DROP_CALLBACK_RVA: usize = 0x0234_ddb0;
/// Registered owner pointer supplied to dispatch.
pub const DROP_CALLBACK_OWNER: usize = 0x28;
/// Registered callback pointer invoked by dispatch.
pub const DROP_CALLBACK: usize = 0x30;
/// Dispatcher calling the registered callback with owner and drop-file event.
pub const DISPATCH_EVENTS_RVA: usize = 0x0234_cf00;
