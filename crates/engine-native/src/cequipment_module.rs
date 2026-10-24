//! CEquipmentModule definitions shared by air and other designs.

/// CEquipmentModule primary vtable.
pub const VTABLE_RVA: usize = 0x0293_7e50;
/// Equipment-module active byte; zero omits the definition from serialization.
pub const ACTIVE: usize = 0x10;
/// Registered country modifier key for this module's design XP cost.
pub const DESIGN_COST_MODIFIER: usize = 0x2c4;
/// CEquipmentModule std::string script token.
pub const NAME: usize = 0x18;
/// Candidate module byte; oil_airbase_cache_value reads a definition through the module entry's definition pointer, concrete type not authenticated there.
pub const CARRIER_CACHE_FLAG_CANDIDATE: usize = 0x2d0;
