
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0285_bba0);
pub const DATA: usize = 0x08;
pub const CAPACITY: usize = 0x10;
pub const COUNT: usize = 0x14;
pub const MAXIMUM: usize = 0x38;
pub const ACTIVE: usize = 0x20;
pub const CATEGORY_FACTOR: usize = 0x30;
pub const CAMO_SHOW_AT_RVA: Rva = Rva(0x033e_1658);

pub use game_model::acclimatization::*;
