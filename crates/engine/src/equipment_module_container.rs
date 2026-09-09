//! Candidate equipment-module container embedded at CEquipmentVariant+B0.
//! The old reader checked 306D170 as a vtable, but the executable's RTTI locator does not validate there.
//! Preserve these as a candidate layout; do not authenticate an object using this word alone.

/// Candidate module-container identity word; read_compact_air_variant_modules compares it, but RTTI validation fails.
pub const IDENTITY_WORD_RVA_CANDIDATE: usize = 0x0306_d170;
/// Candidate module-container data pointer; old air reader uses variant+B8, eight bytes after its assumed header.
pub const ENTRIES_CANDIDATE: usize = 0x08;
/// Candidate module-container u32 capacity; old air reader uses variant+C0, relative header+10.
pub const ENTRIES_CAPACITY_CANDIDATE: usize = 0x10;
/// Candidate module-container u32 count; old air reader uses variant+C4, relative header+14.
pub const ENTRIES_COUNT_CANDIDATE: usize = 0x14;
