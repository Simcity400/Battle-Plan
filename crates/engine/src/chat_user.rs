//! Unnamed CChat user record; a concrete C++ type name is not proven.

/// CChat user-record stride.
pub const STRIDE: usize = 0x58;
/// Chat user ID i32.
pub const ID: usize = 0;
/// Chat user name std::string.
pub const NAME: usize = 0x08;
