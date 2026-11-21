
pub const IDENTITY: usize = 0x718;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub record: u32,
    pub ordinal: u32,
    pub identity: crate::power_balance_definition::Token,
}
