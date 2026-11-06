//! Licensed-production status, embedded in production status.
//!
//! The container owns four arrays and an owner backlink. Serialization writes
//! only the first as available variant handles and the third as nested
//! owned licence records. Offsets use this container as the base.

/// Container available-variant pointer array.
pub const AVAILABLE: usize = 0x08;
/// Container owner pointer.
pub const OWNER: usize = 0x68;
/// Container pointer stride.
pub const POINTER_STRIDE: usize = 0x08;
/// Container allocation size.
pub const SIZE: usize = 0x70;
/// Licensed-production status constructor.
pub const CONSTRUCT_RVA: usize = 0x014d_32a0;
/// Licence registration appends to both owner and giver arrays.
pub const REGISTER_LICENSE_RVA: usize = 0x014d_7630;
