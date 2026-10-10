//! CHotJoinRequestItem presentation layout; no hourly walker is established.
//! Pending accept/decline requests are distinct from player-roster transitions.

/// CHotJoinRequestItem primary vtable.
pub const VTABLE_RVA: usize = 0x029e_a278;
/// CHotJoinRequestItem secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x029e_a318;
/// CHotJoinRequestItem secondary base.
pub const SECONDARY_BASE: usize = 0x18;
/// Hot-join item owner pointer; concrete owner class remains unknown.
pub const OWNER: usize = 0x10;
/// CHotJoinRequestItem accept-checkbox observer subobject.
pub const ACCEPT_OBSERVER: usize = 0x20;
/// Second hot-join checkbox observer; label meaning remains unconfirmed.
pub const SECOND_CHECKBOX_OBSERVER: usize = 0x90;
/// CHotJoinRequestItem name string.
pub const NAME: usize = 0x100;
/// CHotJoinRequestItem second string.
pub const SECOND_STRING: usize = 0x120;
/// CHotJoinRequestItem source pointer.
pub const SOURCE: usize = 0x140;
/// CHotJoinRequestItem constructor.
pub const CONSTRUCTOR_RVA: usize = 0x0179_a5d0;
/// CHotJoinRequestItem refresh.
pub const REFRESH_RVA: usize = 0x0179_c0d0;
/// CHotJoinRequestItem accept observer callback.
pub const ACCEPT_OBSERVER_CALLBACK_RVA: usize = 0x0179_b230;
/// CHotJoinRequestItem second checkbox callback.
pub const SECOND_CHECKBOX_OBSERVER_CALLBACK_RVA: usize = 0x0179_b6a0;
/// External owner name-child dispatch displacement, not an item field;
/// concrete owner class remains unknown.
pub const OWNER_TEXT_GETTER_VTABLE_OFFSET: usize = 0x78;
