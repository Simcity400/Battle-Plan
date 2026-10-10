//! Chat root. The singleton points directly to the root vtable; messages are
//! reached through its channel array. These layouts supply no hourly chat walker.

/// Chat singleton pointer.
pub const SINGLETON_RVA: usize = 0x0337_11b0;
/// Primary chat vtable.
pub const VTABLE_RVA: usize = 0x0299_6f98;
/// Secondary chat vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x0299_7020;
/// Secondary subobject displacement.
pub const SECONDARY_SUBOBJECT: usize = 0x28;
/// Constructor initializing user and channel arrays separately.
pub const CONSTRUCTOR_RVA: usize = 0x0131_a880;
/// Destructor releasing owned arrays.
pub const DESTRUCTOR_RVA: usize = 0x0131_d790;
/// Local user id, i32, resolved in the user array.
pub const LOCAL_USER_ID: usize = 0x60;
/// User-record array pointer.
pub const USERS: usize = 0x80;
/// User capacity, i32.
pub const USERS_CAPACITY: usize = 0x88;
/// User count, i32.
pub const USERS_COUNT: usize = 0x8c;
/// Channel-record array pointer.
pub const CHANNELS: usize = 0xb0;
/// Channel capacity, i32.
pub const CHANNELS_CAPACITY: usize = 0xb8;
/// Channel count, i32.
pub const CHANNELS_COUNT: usize = 0xbc;
/// Selected channel id, i32.
pub const SELECTED_CHANNEL_ID: usize = 0xc8;
/// UI update resolving the selected channel and stored messages.
pub const UPDATE_RVA: usize = 0x0132_c3a0;
