//! CCountryDecryptionState inline rows in CCryptology.

/// Decryption-state vtable.
pub const VTABLE_RVA: usize = 0x029a_64f0;
/// Decryption-state array stride.
pub const STRIDE: usize = 0x38;
/// Target country reference, u32.
pub const TARGET_COUNTRY: usize = 0x08;
/// Activated-cipher bonus duration, i32.
pub const BONUS_DAYS: usize = 0x0c;
/// Activated-cipher bonus byte, set to one on activation.
pub const BONUS_ACTIVE: usize = 0x10;
/// Decrypting byte, interpreted as nonzero.
pub const DECRYPTING: usize = 0x12;
/// Amount, signed i64 fixed5, without narrowing.
pub const AMOUNT: usize = 0x18;
/// Element serializer.
pub const SAVE_RVA: usize = 0x013e_3c70;
/// Element loader.
pub const LOAD_RVA: usize = 0x013e_38b0;
/// Always-serialized target token.
pub const TOKEN_TARGET: u32 = 0x006b;
/// Decrypting token emitted when its byte is nonzero.
pub const TOKEN_DECRYPTING: u32 = 0x32f2;
/// Amount token emitted when the amount is nonzero.
pub const TOKEN_AMOUNT: u32 = 0x01a1;
/// Bonus-active token emitted when its byte is nonzero.
pub const TOKEN_BONUS_ACTIVE: u32 = 0x2c7e;
/// Bonus-days token emitted when days differ from -1.
pub const TOKEN_BONUS_DAYS: u32 = 0x296d;
/// Extra token emitted when its unclassified byte is nonzero.
pub const TOKEN_EXTRA_2A7: u32 = 0x02a7;
/// Date token emitted when the date sentinel differs.
pub const TOKEN_DATE: u32 = 0x284a;
/// Date sentinel word offset.
pub const DATE_SENTINEL_OFFSET: usize = 0x28;
/// Serialized engine-hour date offset.
pub const DATE_HOUR_OFFSET: usize = 0x30;
/// Omitted-date sentinel.
pub const DATE_SENTINEL: i32 = 0x29c77f8;
