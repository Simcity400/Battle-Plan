//! State-history entry whose base owns its date and child-entry list.

/// Primary vtable at displacement zero.
pub const VTABLE_RVA: usize = 0x029c_49c8;
/// History-entry allocation size.
pub const SIZE: usize = 0x48;
/// State-template pointer shared with child commands.
pub const STATE_TEMPLATE: usize = 0x40;
/// History-command loader dispatching ownership, control, buildings,
/// victory points and effects into their command types.
pub const LOAD_COMMAND_RVA: usize = 0x0158_f3b0;
