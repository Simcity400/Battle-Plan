//! CConvoys embedded in CCountry+0x1200. The original country-row loan also
//! supplies the selected transport cache/pool/reservation union in `transport`.
//! Old read_country_convoys narrowed or clamped some pool values; retain native rows before any display calculation.

/// CConvoys primary vtable; collector offsets.rs CONVOY_VTABLE_RVA and pinned executable RTTI agree.
pub const VTABLE_RVA: usize = 0x027d_0c60;
/// CConvoys owner country link u32; constructor14100B130 copies the CCountry tag into +8.
pub const COUNTRY: usize = 8;
/// CConvoys embedded pool;14100B130 constructs it and CEB0 loads token2F4E into +10.
pub const EQUIPMENT_POOL: usize = 0x10;
/// CConvoys reservation pointer vector;14100C970 walks +50 through signed count+5C.
pub const RESERVATIONS: usize = 0x50;
/// Reservation capacity; sorted insertion14100AEE0 grows +50 when signed count+5C equals +58.
pub const RESERVATIONS_CAPACITY: usize = 0x58;
/// Reservation count;14100B520 inserts a bucket; D540 swap-erases its final subscriber's bucket.
pub const RESERVATIONS_COUNT: usize = 0x5c;
/// Signed allocated count;14100C120, C5D0, D0E0, D330, D540 and B520 update +6C.
pub const ASSIGNED: usize = 0x6c;
/// Cache-valid byte;14100C970 tests +78 before FF8A00 recomputation; CEB0 invalidates after load.
pub const CACHE_VALID: usize = 0x78;
/// Signed64 quantity cache;14100C970 fills +80 from FF8A00 before signed /100000 and low32.
pub const CACHE_RAW: usize = 0x80;
pub const POOL_CLAMP_MAGNITUDE: i64 = 0x53e2_d620_fb40;

pub mod transport;
