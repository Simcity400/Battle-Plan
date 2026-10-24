//! Country-embedded convoy state. The country row records the signed quantity
//! cache only when its validity byte is set.

/// Convoy-state primary vtable.
pub const VTABLE_RVA: usize = 0x027e_7130;
/// Owner country link, u32, copied during construction.
pub const COUNTRY: usize = 8;
/// Embedded convoy equipment pool.
pub const EQUIPMENT_POOL: usize = 0x10;
/// Reservation-pointer vector.
pub const RESERVATIONS: usize = 0x50;
/// Signed allocated count.
#[cfg(test)]
pub const ASSIGNED: usize = 0x6c;
/// Cache-valid byte; dirty state recomputes and loading invalidates it.
pub const CACHE_VALID: usize = 0x78;
/// Signed i64 quantity cache; getters divide by fixed5 scale before taking the low word.
pub const CACHE_RAW: usize = 0x80;
