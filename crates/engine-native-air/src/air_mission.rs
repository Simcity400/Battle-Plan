
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_9040);

pub const ENABLED_MASK: usize = 0x10;
pub const CURRENT_TYPE: usize = 0x14;
pub const EFFECTIVE_AIRCRAFTS: usize = 0xe0;
pub const ACTIVE: usize = 0x20;
pub const CONTEXT_MODE: usize = 0x18;
pub const CAPACITY_CLASS: usize = 0x1c;
pub const WING: usize = 0x28;
pub const REGION: usize = 0x30;
pub const REACHED_PROVINCES_DATA: usize = 0x38;
#[cfg(any(test, feature = "test-support"))]
pub const REACHED_PROVINCES_CAPACITY: usize = 0x40;
#[cfg(any(test, feature = "test-support"))]
pub const REACHED_PROVINCES_COUNT: usize = 0x44;
pub const LAST_TARGET_PROVINCE: usize = 0x70;
pub const TRAINING_MASK: u32 = 0x800;
pub const MISSION_BIT_INDEX_RVA: Rva = Rva(0x0109_9d90);
