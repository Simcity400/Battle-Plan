//! Licensed-production status, embedded in production status.
//!
//! The container owns four arrays and an owner backlink. Serialization writes
//! only the first as available variant handles and the third as nested
//! owned licence records. Offsets use this container as the base.

/// Container available-variant pointer array.
pub const AVAILABLE: usize = 0x08;
/// Available-array capacity, i32; the CPdxArray header at `AVAILABLE` places it after the data pointer.
pub const AVAILABLE_CAPACITY: usize = 0x10;
/// Available-array count, i32.
pub const AVAILABLE_COUNT: usize = 0x14;
/// Second licence pointer array, not serialized or recorded.
pub const SECOND_ARRAY: usize = 0x20;
/// Second-array count, i32.
pub const SECOND_ARRAY_COUNT: usize = 0x2c;
/// Owned-licence object array.
pub const OWNED_LICENSES: usize = 0x38;
/// Owned-licence array capacity, i32.
pub const OWNED_LICENSES_CAPACITY: usize = 0x40;
/// Owned-licence array count, i32.
pub const OWNED_LICENSES_COUNT: usize = 0x44;
/// Granted-licence object array, a reverse index of owned licences.
/// The same object belongs to both owners; this index is not saved or recorded.
pub const GRANTED_LICENSES: usize = 0x50;
/// Granted-licence array count, i32.
pub const GRANTED_LICENSES_COUNT: usize = 0x5c;
/// Container owner pointer.
pub const OWNER: usize = 0x68;
/// Container pointer stride.
pub const POINTER_STRIDE: usize = 0x08;
/// Container allocation size.
pub const SIZE: usize = 0x70;
/// Licensed-production status constructor.
pub const CONSTRUCT_RVA: usize = 0x0142_4980;
/// Licensed-production serializer writes available handles and owned licences only.
pub const SERIALIZE_RVA: usize = 0x0142_9400;
/// Licensed-production deserializer.
pub const DESERIALIZE_RVA: usize = 0x0142_8b50;
/// Licence registration appends to both owner and giver arrays.
pub const REGISTER_LICENSE_RVA: usize = 0x0142_8d10;
