//! CEquipmentModule definitions shared by air and other designs.

/// CEquipmentModule primary vtable.
pub const VTABLE_RVA: usize = 0x0292_1850;
/// Equipment-module active byte; zero omits the definition from serialization.
pub const ACTIVE: usize = 0x10;
/// CEquipmentModule std::string script token.
pub const NAME: usize = 0x18;
/// Candidate module byte at +2D0; oil_airbase_cache_value reads a definition through module-entry+8, concrete type not authenticated there.
pub const CARRIER_CACHE_FLAG_CANDIDATE: usize = 0x2d0;
