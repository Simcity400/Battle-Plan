//! CCountryDecryptionState inline rows in CCryptology.

use engine_sites::Rva;
/// Decryption-state vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a3_7ea0);
/// Decryption-state array stride.
pub const STRIDE: usize = 0x38;
/// Target country reference, u32.
pub const TARGET_COUNTRY: usize = 0x08;
/// Activated-cipher bonus duration, i32.
#[cfg(test)]
pub const BONUS_DAYS: usize = 0x0c;
/// Activated-cipher bonus byte, set to one on activation.
pub const BONUS_ACTIVE: usize = 0x10;
/// Decrypting byte, interpreted as nonzero.
pub const DECRYPTING: usize = 0x12;
/// Amount, signed i64 fixed5, without narrowing.
pub const AMOUNT: usize = 0x18;
/// Element serializer.
#[cfg(test)]
pub const SAVE_RVA: usize = 0x0149_4d90;
