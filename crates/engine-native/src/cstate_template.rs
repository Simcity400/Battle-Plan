//! State template selected by the simulation state.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029c_0908);
/// Province-ID array.
pub const PROVINCES: usize = 0x08;
/// Province count, signed i32.
pub const PROVINCE_COUNT: usize = 0x14;
/// Fallback display-name string used when both state names are empty.
pub const DISPLAY_NAME: usize = 0x20;
/// State ID, u32, copied into the simulation state.
pub const ID: usize = 0xa0;
/// Resource-entry pointer.
pub const RESOURCES: usize = 0xf8;
/// Resource-array signed capacity, grown before append.
#[cfg(any(test, feature = "test-support"))]
pub const RESOURCE_CAPACITY: usize = 0x100;
/// Resource-array signed count, incremented on append.
#[cfg(any(test, feature = "test-support"))]
pub const RESOURCE_COUNT: usize = 0x104;
/// Constructor initializing resource fields.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00b0_df10);

/// Retained continent-definition pointer initialized to the database null
/// continent. Province traversal does not clear it first.
pub const CONTINENT: usize = 0x140;
