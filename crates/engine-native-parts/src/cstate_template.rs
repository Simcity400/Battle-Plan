
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029c_0908);
pub const PROVINCES: usize = 0x08;
pub const PROVINCE_COUNT: usize = 0x14;
pub const DISPLAY_NAME: usize = 0x20;
pub const ID: usize = 0xa0;
pub const RESOURCES: usize = 0xf8;
#[cfg(any(test, feature = "test-support"))]
pub const RESOURCE_CAPACITY: usize = 0x100;
#[cfg(any(test, feature = "test-support"))]
pub const RESOURCE_COUNT: usize = 0x104;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00b0_df10);

pub const CONTINENT: usize = 0x140;
