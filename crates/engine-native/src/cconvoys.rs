//! Country-embedded convoy state. The country row records the signed quantity
//! cache only when its validity byte is set.

/// Convoy-state primary vtable.
pub const VTABLE_RVA: usize = 0x027d_0c60;
/// Owner country link, u32, copied during construction.
pub const COUNTRY: usize = 8;
/// Embedded convoy equipment pool.
pub const EQUIPMENT_POOL: usize = 0x10;
/// Reservation-pointer vector.
pub const RESERVATIONS: usize = 0x50;
/// Reservation capacity.
pub const RESERVATIONS_CAPACITY: usize = 0x58;
/// Signed reservation count; removing a final subscriber swap-erases its bucket.
pub const RESERVATIONS_COUNT: usize = 0x5c;
/// Signed allocated count.
pub const ASSIGNED: usize = 0x6c;
/// Cache-valid byte; dirty state recomputes and loading invalidates it.
pub const CACHE_VALID: usize = 0x78;
/// Signed i64 quantity cache; getters divide by fixed5 scale before taking the low word.
pub const CACHE_RAW: usize = 0x80;
/// Signed pool-sum clamp magnitude, applied after every addition.
pub const POOL_CLAMP_MAGNITUDE: i64 = 0x53e2_d620_fb40;
