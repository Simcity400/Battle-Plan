
pub const STRIDE: usize = 8;
pub const PROVINCE_ID: usize = 0;
pub const LEVEL: usize = 4;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub from_province: u32,
    pub to_province: u32,
    pub level: i32,
}
