//! CNationalFocus definition fields retained from state and presentation readers.

/// CNationalFocus MSVC string token.
pub const TOKEN: usize = 0x18;
/// CNationalFocus historical-marker byte.
pub const HISTORICAL_MARKER: usize = 0x5b8;
/// CNationalFocus authored X, i32.
pub const POSITION_X: usize = 0x118;
/// CNationalFocus authored Y, i32.
pub const POSITION_Y: usize = 0x11c;
/// CNationalFocus root X contribution, i32.
pub const ROOT_POSITION_X: usize = 0x120;
/// CNationalFocus root Y contribution, i32.
pub const ROOT_POSITION_Y: usize = 0x124;
/// National-focus position getter.
pub const GET_POSITION_RVA: usize = 0x002d_2330;
