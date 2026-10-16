//! Power-balance definition.

/// Power-balance definition identity MSVC string.
pub const IDENTITY: usize = 0x10;
/// Power-balance display-token getter.
pub const DISPLAY_TOKEN_RVA: usize = 0x00a7_e7b0;
/// Stored-order power-balance side lookup.
pub const FIND_SIDE_RVA: usize = 0x00a7_e960;

/// An exact byte string in the caller-owned census arena. No UTF-8 conversion
/// or fixed token-size limit is imposed on an engine identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub len: usize,
}

impl Token {
    pub fn bytes(self, arena: &[u8]) -> Option<&[u8]> {
        arena.get(self.start..self.start.checked_add(self.len)?)
    }
}
