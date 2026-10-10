//! Window singleton used by event dispatch. Its activity setter writes a
//! global byte without rebuilding graphics or finalizing settings.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02ae_6818;
/// Singleton pointer initialized on first use.
pub const SINGLETON_RVA: usize = 0x0343_9230;
/// Allocation size for an object containing the vtable.
pub const SIZE: usize = 8;
/// Singleton getter returning the initialized pointer.
pub const GETTER_RVA: usize = 0x0220_d6a0;
/// Activity-setter virtual-dispatch offset used by resize subtype 5.
pub const SET_ACTIVITY_VSLOT: usize = 0x18;
/// Activity setter writing its byte argument to the global activity byte.
pub const SET_ACTIVITY_RVA: usize = 0x0220_d770;
/// Global activity byte, separate from object fields.
pub const ACTIVITY_RVA: usize = 0x030a_5cd8;
