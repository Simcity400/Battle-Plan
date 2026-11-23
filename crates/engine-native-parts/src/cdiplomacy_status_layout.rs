use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_e5e8);
pub const COUNTRY: usize = 0x2d0;
pub const FACTION: usize = 0x2d8;
pub const OVERLORD: usize = 0x188;
pub const CURRENT_AUTONOMY_STATUS: usize = 0x3b8;
pub const SUBJECTS: usize = 0x170;
pub const RELATIONS: usize = 0x08;
pub const OWNED_RELATIONS: usize = 0x20;
pub const OWNED_WARGOALS: usize = 0x68;
pub const ENEMY_COUNTRIES: usize = 0x98;
pub const WARTIME_ALLIES: usize = 0xb0;
#[cfg(any(test, feature = "test-support"))]
pub const RELATION_COUNT: usize = 0x14;
#[cfg(any(test, feature = "test-support"))]
pub const RELATION_CAPACITY: usize = 0x10;
#[cfg(any(test, feature = "test-support"))]
pub const OWNED_RELATION_COUNT: usize = 0x2c;
#[cfg(any(test, feature = "test-support"))]
pub const OWNED_RELATION_CAPACITY: usize = 0x28;
