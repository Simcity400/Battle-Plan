//! Inline side-sprite override. The first matching side is selected and its
//! sprite is used only when its string is nonempty.

/// Side-sprite override stride.
pub const STRIDE: usize = 0x50;
/// Override side-token MSVC string.
pub const SIDE: usize = 0x08;
/// Override sprite MSVC string.
pub const GFX: usize = 0x30;

/// Stored override entries retain order, empty values and duplicate side keys.
/// The engine's first-match lookup and empty-value fallback can be derived.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub record: u32,
    pub ordinal: u32,
    pub side: crate::power_balance_definition::Token,
    pub gfx: crate::power_balance_definition::Token,
}
