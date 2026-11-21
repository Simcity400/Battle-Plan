
pub const IDENTITY: usize = 0x10;

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
