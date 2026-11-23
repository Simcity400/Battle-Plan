
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_b658);
pub const TASK_FORCE: usize = 0x08;
pub const KIND: usize = 0x14;
pub const IS_IN_REGIONS: usize = 0x30;
#[cfg(any(test, feature = "test-support"))]
pub const RUNNING_AWAY: usize = 0x31;
pub const ENGAGEMENT_RULE: usize = 0x34;
pub const BOMBARDMENT_REGION: usize = 0x60;
pub const ACCESSIBLE_REGIONS: usize = 0xc8;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0102_df90);
