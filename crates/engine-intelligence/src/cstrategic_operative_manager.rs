
use engine_sites::Rva;
pub const SIZE: usize = 0x68;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0f2_68a0);
pub const OPERATIVES_DATA: usize = 0x08;
pub const OPERATIVE_STRIDE: usize = 8;
#[cfg(test)]
pub const SAVE_RVA: usize = 0x0f2_7240;
