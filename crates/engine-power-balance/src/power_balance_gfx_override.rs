
pub const STRIDE: usize = 0x50;
pub const SIDE: usize = 0x08;
pub const GFX: usize = 0x30;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub record: u32,
    pub ordinal: u32,
    pub side: crate::power_balance_definition::Token,
    pub gfx: crate::power_balance_definition::Token,
}
