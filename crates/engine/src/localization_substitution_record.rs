//! Localization substitution records used during formatting.
//! These presentation operands are excluded from hourly tables.

/// Substitution-array element stride.
pub const STRIDE: usize = 0x68;
/// Substitution u32 kind, checked before selecting the payload.
pub const TYPE: usize = 0;
/// Substitution argument-name pointer.
pub const NAME: usize = 8;
/// Substitution signed i64 fixed-point payload with scale 100000.
pub const FIXED5_VALUE: usize = 0x50;
/// Substitution kind selecting the i64 fixed-point payload.
pub const FIXED5_TYPE: u32 = 9;
/// Formatter entry receiving output, key, argument records and their count.
pub const CONSUMER_RVA: usize = 0x0223_0f20;
/// Formatter entry bytes checked before installing an observer.
pub const CONSUMER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48,
];
/// Reduced research-penalty localization key, taking four arguments.
pub const RESEARCH_REDUCED_KEY_RVA: usize = 0x0292_dd80;
/// Unreduced research-penalty localization key, taking two arguments.
pub const RESEARCH_UNREDUCED_KEY_RVA: usize = 0x0292_dda0;
/// Research penalty argument-name pointer in the first record of either key.
pub const RESEARCH_VALUE_NAME_RVA: usize = 0x0272_4f3c;
/// Reduced research-penalty argument-name pointer in the second record.
pub const RESEARCH_RED_NAME_RVA: usize = 0x0292_dd78;
/// Research reduction-years argument-name pointer in the third record.
pub const RESEARCH_REDUCED_YEARS_NAME_RVA: usize = 0x0292_dd68;
/// Research ahead-years argument-name pointer in either key's final record.
pub const RESEARCH_YEARS_NAME_RVA: usize = 0x0292_dd5c;
