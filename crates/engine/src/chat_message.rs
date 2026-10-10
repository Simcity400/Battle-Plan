//! Unnamed CChat message record; a concrete C++ type name is not proven.

/// Chat message row stride.
pub const STRIDE: usize = 0x48;
/// Chat message text std::string.
pub const TEXT: usize = 0;
/// Chat message sender std::string.
pub const SENDER: usize = 0x20;
/// Signed message-display reference; exact semantics remain unproven.
pub const DISPLAY_REFERENCE_A: usize = 0x40;
/// Second signed message-display reference; exact semantics remain unproven.
pub const DISPLAY_REFERENCE_B: usize = 0x44;
/// Chat message append receives the channel array header, not the chat root.
pub const APPEND_RVA: usize = 0x0131_8090;
