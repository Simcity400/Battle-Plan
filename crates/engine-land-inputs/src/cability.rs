
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a0_13e8);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x02a0_1438);
pub const SECONDARY_BASE: usize = 0x08;
pub const VALID: usize = 0x10;
pub const TOKEN: usize = 0x70;
pub const TOKEN_LENGTH: usize = 0x80;
pub const TOKEN_CAPACITY: usize = 0x88;
pub const TARGET_KIND: usize = 0x90;
pub const ALLOCATE_COST: usize = 0x94;
pub const DURATION_HOURS: usize = 0xa8;
#[cfg(test)]
pub const COOLDOWN_HOURS: usize = 0xac;
pub const COST_SCALE: i64 = 100_000;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0108_f390);
