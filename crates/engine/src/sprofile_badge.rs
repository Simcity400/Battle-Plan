//! Profile badge embedded in the human object; payload meanings are
//! unestablished.

/// Badge vtable at human +0x60.
pub const VTABLE_RVA: usize = 0x0270_ad38;
/// First raw dword copied from badge +8 to human +0x68.
pub const FIRST_VALUE: usize = 0x08;
/// Second raw dword copied from badge +0xc to human +0x6c.
pub const SECOND_VALUE: usize = 0x0c;
