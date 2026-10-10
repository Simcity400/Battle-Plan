//! Chat channel record, distinct from the root chat object.

/// CChat channel record stride.
pub const STRIDE: usize = 0xc0;
/// Channel's owning CChat pointer.
pub const CHAT: usize = 0;
/// Channel ID i32.
pub const ID: usize = 0x44;
/// Channel message array pointer.
pub const MESSAGES: usize = 0xa8;
/// Channel message array capacity i32.
pub const MESSAGES_CAPACITY: usize = 0xb0;
/// Channel message array count i32.
pub const MESSAGES_COUNT: usize = 0xb4;
/// Channel message preparation.
pub const PREPARE_MESSAGE_RVA: usize = 0x0131_fa20;
