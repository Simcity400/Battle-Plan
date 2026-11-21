
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a4_c2e8);
pub const RECORDS: usize = 8;

pub const CAPACITY: usize = 0x20;
pub const CURRENT_SLOT: usize = 0x24;
pub const WRAPPED: usize = 0x28;
pub const RECORD_FIELDS: usize = 0x30;
pub const HISTORY: usize = 0x38;
