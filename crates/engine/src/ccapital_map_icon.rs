//! State-capital presentation icon. Existence depends on live camera and map
//! mode. Numeric primary and secondary vtable identities are unavailable.

/// Constructor installing derived vtables.
pub const CONSTRUCT_RVA: usize = 0x018a_5390;
/// Allocation size.
pub const SIZE: usize = 0xd8;
/// Map-icon manager bucket.
pub const ICON_BUCKET: u32 = 0x0e;
/// Secondary vptr displacement.
pub const SECONDARY_VPTR: usize = 0x80;
/// Cached state pointer checked against the current holder's state.
pub const CACHED_STATE: usize = 0x98;
/// Cached member-pointer array; member type is unknown.
pub const CACHED_MEMBERS: usize = 0xa0;
/// Cached member count, i32.
pub const CACHED_MEMBER_COUNT: usize = 0xac;
/// State refresh when cached state or member ownership changes.
pub const REFRESH_STATE_RVA: usize = 0x018b_abe0;
