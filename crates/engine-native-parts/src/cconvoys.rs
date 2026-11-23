
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0286_4500);
pub const COUNTRY: usize = 8;
pub const EQUIPMENT_POOL: usize = 0x10;
pub const RESERVATIONS: usize = 0x50;
#[cfg(test)]
pub const ASSIGNED: usize = 0x6c;
pub const CACHE_VALID: usize = 0x78;
pub const CACHE_RAW: usize = 0x80;
