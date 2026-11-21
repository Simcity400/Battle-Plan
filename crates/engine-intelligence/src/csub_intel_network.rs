
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x2a1_dd28);
pub const CONSTRUCTOR_RVA: Rva = Rva(0x125_cfd0);

pub const STRIDE: usize = 0xd8;
pub const GAIN_FACTOR: usize = 0x08;
pub const STRENGTH: usize = 0x20;
pub const STRENGTH_TARGET: usize = 0x40;
pub const TARGET_FROM_OPERATIVES: usize = 0x48;
pub const TARGET_FROM_COUNTERINTELLIGENCE: usize = 0x50;
pub const QUIET: usize = 0x58;
