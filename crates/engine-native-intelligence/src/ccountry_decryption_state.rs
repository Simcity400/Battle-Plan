
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a3_7ea0);
pub const STRIDE: usize = 0x38;
pub const TARGET_COUNTRY: usize = 0x08;
#[cfg(test)]
pub const BONUS_DAYS: usize = 0x0c;
pub const BONUS_ACTIVE: usize = 0x10;
pub const DECRYPTING: usize = 0x12;
pub const AMOUNT: usize = 0x18;
#[cfg(test)]
pub const SAVE_RVA: usize = 0x0149_4d90;
